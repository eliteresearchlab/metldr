//! Keyword retrieval for page chat and global search: a BM25 index over the stored chunks and
//! the rank fusion that merges it with the dense (embedding) neighbours.
//!
//! BM25 (Robertson and Sparck Jones) with k1 = 1.2 and b = 0.75; the IDF is the Lucene form
//! ln(1 + (N - df + 0.5) / (df + 0.5)), which stays positive for common terms. Fusion is
//! Reciprocal Rank Fusion (Cormack, Clarke and Buettcher, 2009) with k = 60: it reads only each
//! hit's position, because the dense leg returns no similarity and BM25 scores are not
//! comparable with cosine similarity.

use std::collections::HashMap as StdMap;
use std::hash::{BuildHasherDefault, Hasher};

/// FNV-1a: the keys are short words and ids from our own index, so a keyed hash buys nothing
/// and SipHash costs several times the cycles per lookup and more wasm.
#[derive(Default)]
struct Fnv(u64);

impl Hasher for Fnv {
    fn write(&mut self, bytes: &[u8]) {
        let mut hash = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for &byte in bytes {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        self.0 = hash;
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

type HashMap<K, V> = StdMap<K, V, BuildHasherDefault<Fnv>>;

const K1: f64 = 1.2;
const B: f64 = 0.75;
const RRF_K: f64 = 60.0;
/// Fused scores use a 0 to 100 scale: a chunk ranked first by both lists scores 100.
const TOP_SCORE: f64 = 100.0;
const SUMMARY_BOOST: f64 = 8.0;
const REPEAT_PENALTY: f64 = 15.0;
const CANDIDATE_CAP: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    Semantic,
    Keyword,
    Hybrid,
}

impl Match {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Keyword => "keyword",
            Self::Hybrid => "hybrid",
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Hit<'a> {
    pub id: &'a str,
    pub score: f64,
    pub kind: Match,
}

struct Doc {
    id: Box<str>,
    len: u32,
    terms: Box<[u32]>,
    summary: bool,
    /// The source address without its query or fragment, for the repeat penalty.
    source: Box<str>,
    source_url: Box<str>,
    source_id: Box<str>,
}

/// The chunks in a dense arena; postings and scores index it by slot, so a search accumulates
/// into one flat array instead of a hash map per query.
#[derive(Default)]
pub struct Index {
    terms: HashMap<Box<str>, u32>,
    postings: Vec<Vec<(u32, u32)>>,
    docs: Vec<Option<Doc>>,
    slots: HashMap<Box<str>, u32>,
    free: Vec<u32>,
    sources: HashMap<Box<str>, u32>,
    total_len: u64,
}

/// Lowercase words of three or more characters; anything that is not a letter, digit or
/// underscore separates words.
pub fn tokenize(text: &str, each: &mut dyn FnMut(&str)) {
    let mut word = String::new();
    let mut chars = 0;
    let mut flush = |word: &mut String, chars: &mut usize| {
        if *chars > 2 {
            each(word);
        }
        word.clear();
        *chars = 0;
    };
    for c in text.chars() {
        if c.is_ascii() {
            if c.is_ascii_alphanumeric() || c == '_' {
                word.push(c.to_ascii_lowercase());
                chars += 1;
            } else {
                flush(&mut word, &mut chars);
            }
        } else if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
            chars += 1;
        } else {
            flush(&mut word, &mut chars);
        }
    }
    flush(&mut word, &mut chars);
}

fn source_base(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..end]
}

impl Index {
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn add(&mut self, id: &str, text: &str, source_url: &str, source_id: &str, summary: bool) {
        self.remove(id);
        let mut counts: Vec<(u32, u32)> = Vec::new();
        let mut len = 0u32;
        let (terms, postings) = (&mut self.terms, &mut self.postings);
        tokenize(text, &mut |word| {
            len += 1;
            let term = match terms.get(word) {
                Some(&term) => term,
                None => {
                    let term = postings.len() as u32;
                    postings.push(Vec::new());
                    terms.insert(word.into(), term);
                    term
                }
            };
            match counts.iter_mut().find(|(t, _)| *t == term) {
                Some((_, count)) => *count += 1,
                None => counts.push((term, 1)),
            }
        });
        let slot = self.free.pop().unwrap_or_else(|| {
            self.docs.push(None);
            self.docs.len() as u32 - 1
        });
        for &(term, count) in &counts {
            self.postings[term as usize].push((slot, count));
        }
        for key in [source_url, source_id] {
            if !key.is_empty() {
                *self.sources.entry(key.into()).or_default() += 1;
            }
        }
        self.total_len += u64::from(len);
        self.slots.insert(id.into(), slot);
        self.docs[slot as usize] = Some(Doc {
            id: id.into(),
            len,
            terms: counts.into_iter().map(|(term, _)| term).collect(),
            summary,
            source: source_base(source_url).into(),
            source_url: source_url.into(),
            source_id: source_id.into(),
        });
    }

    pub fn remove(&mut self, id: &str) {
        let Some(slot) = self.slots.remove(id) else {
            return;
        };
        let Some(doc) = self.docs[slot as usize].take() else {
            return;
        };
        for &term in &doc.terms {
            self.postings[term as usize].retain(|&(s, _)| s != slot);
        }
        for key in [&doc.source_url, &doc.source_id] {
            if let Some(count) = self.sources.get_mut(key) {
                *count -= 1;
                if *count == 0 {
                    self.sources.remove(key);
                }
            }
        }
        self.total_len -= u64::from(doc.len);
        self.free.push(slot);
    }

    /// Whether any chunk came from this page address or source id (never true for "").
    pub fn has_source(&self, source: &str) -> bool {
        self.sources.contains_key(source)
    }

    /// BM25 hits, best first, as slots; equal scores keep the earlier slot first.
    pub fn keyword(&self, query: &str, limit: usize) -> Vec<u32> {
        let n = self.slots.len();
        if n == 0 || limit == 0 {
            return Vec::new();
        }
        let mut terms: Vec<u32> = Vec::new();
        tokenize(query, &mut |word| {
            if let Some(&term) = self.terms.get(word)
                && !terms.contains(&term)
            {
                terms.push(term);
            }
        });
        let average = self.total_len as f64 / n as f64;
        let mut scores = vec![0.0f64; self.docs.len()];
        let mut touched: Vec<u32> = Vec::new();
        for term in terms {
            let posting = &self.postings[term as usize];
            let df = posting.len() as f64;
            let idf = (1.0 + (n as f64 - df + 0.5) / (df + 0.5)).ln();
            for &(slot, tf) in posting {
                let len = self.docs[slot as usize].as_ref().map_or(0, |doc| doc.len);
                let norm = K1 * (1.0 - B + B * (f64::from(len) / average));
                let tf = f64::from(tf);
                let score = &mut scores[slot as usize];
                if *score == 0.0 {
                    touched.push(slot);
                }
                *score += idf * (tf * (K1 + 1.0)) / (tf + norm);
            }
        }
        let mut hits: Vec<Ranked> = touched
            .into_iter()
            .map(|slot| Ranked {
                slot,
                score: scores[slot as usize],
                kind: Match::Keyword,
                at: slot,
            })
            .collect();
        // a common word touches most chunks: keep the best `limit` in linear time, sort only those
        if hits.len() > limit {
            hits.select_nth_unstable_by(limit - 1, best_first);
            hits.truncate(limit);
        }
        order(&mut hits);
        hits.into_iter().map(|hit| hit.slot).collect()
    }

    /// Fuses the dense neighbours (ids, nearest first) with the keyword hits for `query`, then
    /// lifts summaries and spreads results across sources. At most 50 hits, best first.
    pub fn rank<'a>(
        &'a self,
        query: &str,
        dense: &[impl AsRef<str>],
        keyword_limit: usize,
    ) -> Vec<Hit<'a>> {
        let mut fused = Fusion {
            hits: Vec::new(),
            at: vec![NONE; self.docs.len()],
        };
        fused.add(
            dense
                .iter()
                .filter_map(|id| self.slots.get(id.as_ref()).copied()),
            true,
        );
        fused.add(self.keyword(query, keyword_limit), false);

        let scale = TOP_SCORE * (RRF_K + 1.0) / 2.0;
        let doc = |slot: u32| {
            self.docs[slot as usize]
                .as_ref()
                .expect("a ranked slot holds a chunk")
        };
        let mut hits: Vec<Ranked> = fused
            .hits
            .into_iter()
            .enumerate()
            .map(|(at, f)| {
                let kind = match (f.dense, f.keyword) {
                    (true, true) => Match::Hybrid,
                    (true, false) => Match::Semantic,
                    _ => Match::Keyword,
                };
                Ranked {
                    slot: f.slot,
                    score: f.raw * scale,
                    kind,
                    at: at as u32,
                }
            })
            .collect();
        order(&mut hits);
        for hit in &mut hits {
            if doc(hit.slot).summary {
                hit.score = (hit.score + SUMMARY_BOOST).min(TOP_SCORE);
            }
        }
        order(&mut hits);
        // at most a hundred hits over a handful of sources: a short list beats a hash map
        let mut seen: Vec<(&str, u32)> = Vec::new();
        for hit in &mut hits {
            let source = &*doc(hit.slot).source;
            let repeats = match seen.iter_mut().find(|(s, _)| *s == source) {
                Some((_, count)) => {
                    *count += 1;
                    *count - 1
                }
                None => {
                    seen.push((source, 1));
                    0
                }
            };
            if repeats > 0 {
                hit.score = (hit.score - f64::from(repeats) * REPEAT_PENALTY).max(0.0);
            }
        }
        order(&mut hits);
        hits.truncate(CANDIDATE_CAP);
        hits.into_iter()
            .map(|h| Hit {
                id: &doc(h.slot).id,
                score: h.score,
                kind: h.kind,
            })
            .collect()
    }
}

const NONE: u32 = u32::MAX;

struct Fused {
    slot: u32,
    raw: f64,
    dense: bool,
    keyword: bool,
}

/// Reciprocal rank fusion state: the fused hits and, flat by slot, each slot's place in them.
struct Fusion {
    hits: Vec<Fused>,
    at: Vec<u32>,
}

impl Fusion {
    /// Adds one ranked list; a repeat within the same list counts once.
    fn add(&mut self, slots: impl IntoIterator<Item = u32>, from_dense: bool) {
        for (rank, slot) in slots.into_iter().enumerate() {
            let contribution = 1.0 / (RRF_K + rank as f64 + 1.0);
            let at = self.at[slot as usize];
            if at == NONE {
                self.at[slot as usize] = self.hits.len() as u32;
                let hit = Fused {
                    slot,
                    raw: contribution,
                    dense: from_dense,
                    keyword: !from_dense,
                };
                self.hits.push(hit);
                continue;
            }
            let hit = &mut self.hits[at as usize];
            if from_dense || hit.keyword {
                continue;
            }
            hit.raw += contribution;
            hit.keyword = true;
        }
    }
}

struct Ranked {
    slot: u32,
    score: f64,
    kind: Match,
    /// Position before this sort: equal scores keep their order, as a stable sort would.
    at: u32,
}

fn best_first(a: &Ranked, b: &Ranked) -> std::cmp::Ordering {
    b.score.total_cmp(&a.score).then(a.at.cmp(&b.at))
}

/// Best first. One unstable sort with the position as tiebreak orders exactly as a stable sort
/// and keeps a single sort routine in the wasm.
fn order(hits: &mut [Ranked]) {
    hits.sort_unstable_by(best_first);
    for (at, hit) in hits.iter_mut().enumerate() {
        hit.at = at as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        tokenize(text, &mut |w| out.push(w.to_owned()));
        out
    }

    #[test]
    fn tokens_are_lowercase_words_of_three_or_more_characters() {
        assert_eq!(
            words("The Rust-lang book, 2nd ed: ownership!"),
            ["the", "rust", "lang", "book", "2nd", "ownership"]
        );
        assert_eq!(
            words("Café Müller über naïve 東京大学"),
            ["café", "müller", "über", "naïve", "東京大学"]
        );
    }

    #[test]
    fn keyword_ranking_prefers_term_frequency_and_short_chunks() {
        let mut index = Index::default();
        index.add("a", "rust ownership rules", "https://a", "a", false);
        index.add(
            "b",
            "rust rust rust ownership borrowing lifetimes and many other words",
            "https://b",
            "b",
            false,
        );
        index.add("c", "gardening tips", "https://c", "c", false);
        let hits = index.rank("rust ownership", &[] as &[&str], 10);
        assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), ["a", "b"]);
        assert!(hits.iter().all(|h| h.kind == Match::Keyword));
        index.remove("a");
        assert!(!index.has_source("https://a"));
        assert_eq!(index.rank("ownership", &[] as &[&str], 10)[0].id, "b");
    }

    #[test]
    fn fusion_scores_a_double_first_place_100_and_penalises_repeated_sources() {
        let mut index = Index::default();
        index.add("p:1", "alpha beta", "https://x/page?a=1", "x", false);
        index.add("p:2", "alpha gamma", "https://x/page#two", "x", false);
        index.add("s", "summary of alpha", "https://y", "y:summary", true);
        let hits = index.rank("alpha beta", &["p:1", "s", "missing"], 10);
        assert_eq!(hits[0].id, "p:1");
        assert_eq!(hits[0].kind, Match::Hybrid);
        assert!((hits[0].score - 100.0).abs() < 1e-9);
        let second = hits.iter().find(|h| h.id == "p:2").unwrap();
        assert_eq!(second.kind, Match::Keyword);
        let raw: f64 = (1.0 / 62.0) * 100.0 * 61.0 / 2.0;
        assert!((second.score - (raw - 15.0).max(0.0)).abs() < 1e-9);
        assert!(index.has_source("y:summary") && index.has_source("https://y"));
    }
}
