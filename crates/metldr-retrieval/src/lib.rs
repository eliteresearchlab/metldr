//! Keyword retrieval for page chat and global search: a BM25 index over the stored chunks and
//! the rank fusion that merges it with the dense (embedding) neighbours.
//!
//! BM25 (Robertson and Sparck Jones) with k1 = 1.2 and b = 0.75; the IDF is the Lucene form
//! ln(1 + (N - df + 0.5) / (df + 0.5)), which stays positive for common terms. Fusion is
//! Reciprocal Rank Fusion (Cormack, Clarke and Buettcher, 2009) with k = 60: it reads only each
//! hit's position, because the dense leg returns no similarity and BM25 scores are not
//! comparable with cosine similarity.
//!
//! A query word the index has never seen is read as a typo: the closest indexed word within one
//! or two edits (Smith-Waterman with affine gaps, SIMD where the target has it) answers for it.

pub mod query;

use std::collections::HashMap as StdMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

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

/// One stored chunk as the extension keeps it.
pub struct Chunk<'a> {
    pub id: &'a str,
    /// "article", "page", "email" or "pdf".
    pub kind: &'a str,
    pub title: &'a str,
    pub content: &'a str,
    pub source_url: &'a str,
    pub source_id: &'a str,
    pub summary: bool,
}

struct Doc {
    id: Box<str>,
    kind: Box<str>,
    title: Box<str>,
    content: Box<str>,
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
    terms: HashMap<Rc<str>, u32>,
    /// The terms by their length in characters, the haystacks for typo matching: a typo search
    /// reads only the words whose length it can reach.
    lengths: Vec<Lengths>,
    postings: Vec<Vec<(u32, u32)>>,
    /// Scratch for `add`, flat by term: one more than the term's place in the chunk's counts,
    /// or 0. Counting a chunk's terms is then one array read per word, not a scan of the counts.
    seen: Vec<u32>,
    docs: Vec<Option<Doc>>,
    /// Token counts by slot, apart from `docs`: the BM25 loop then reads 4 bytes per posting,
    /// not a whole chunk record.
    lens: Vec<u32>,
    slots: HashMap<Box<str>, u32>,
    free: Vec<u32>,
    sources: HashMap<Box<str>, u32>,
    total_len: u64,
}

/// The indexed words of one length, each beside its term (shared with `Index::terms`).
#[derive(Default)]
struct Lengths {
    words: Vec<Rc<str>>,
    terms: Vec<u32>,
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

const SNIPPET: usize = 200;

/// Global search sources and the context the answer reads.
#[derive(serde::Serialize)]
pub struct Sources<'a> {
    pub context: String,
    pub sources: Vec<Source<'a>>,
}

#[derive(serde::Serialize)]
pub struct Source<'a> {
    pub index: usize,
    pub title: &'a str,
    pub url: &'a str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub score: u32,
    pub snippet: &'a str,
}

impl Doc {
    /// The chunk's title, else its page address, else its id.
    fn label(&self) -> &str {
        [&self.title, &self.source_url]
            .into_iter()
            .find(|s| !s.is_empty())
            .map_or(&self.id, |s| s)
    }
}

/// JavaScript's Math.round for the non-negative scores here.
fn whole(score: f64) -> u32 {
    (score + 0.5).floor() as u32
}

fn before_query(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

/// One page per source: the address without chunk suffix, query or trailing slash.
fn page_key(url: &str) -> &str {
    let url = url.split(":chunk:").next().unwrap_or(url);
    let url = before_query(url);
    url.strip_suffix('/').unwrap_or(url)
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

    pub fn add(&mut self, chunk: &Chunk) {
        let Chunk {
            id,
            source_url,
            source_id,
            ..
        } = *chunk;
        self.remove(id);
        let mut counts: Vec<(u32, u32)> = Vec::new();
        let mut len = 0u32;
        let (terms, lengths, postings, seen) = (
            &mut self.terms,
            &mut self.lengths,
            &mut self.postings,
            &mut self.seen,
        );
        tokenize(chunk.content, &mut |word| {
            len += 1;
            let term = match terms.get(word) {
                Some(&term) => term,
                None => {
                    let term = postings.len() as u32;
                    let chars = word.chars().count();
                    if lengths.len() <= chars {
                        lengths.resize_with(chars + 1, Lengths::default);
                    }
                    let word: Rc<str> = word.into();
                    postings.push(Vec::new());
                    seen.push(0);
                    lengths[chars].words.push(word.clone());
                    lengths[chars].terms.push(term);
                    terms.insert(word, term);
                    term
                }
            };
            let at = &mut seen[term as usize];
            if *at == 0 {
                counts.push((term, 1));
                *at = counts.len() as u32;
            } else {
                counts[*at as usize - 1].1 += 1;
            }
        });
        for &(term, _) in &counts {
            self.seen[term as usize] = 0;
        }
        let slot = self.free.pop().unwrap_or_else(|| {
            self.docs.push(None);
            self.lens.push(0);
            self.docs.len() as u32 - 1
        });
        self.lens[slot as usize] = len;
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
            kind: chunk.kind.into(),
            title: chunk.title.into(),
            content: chunk.content.into(),
            len,
            terms: counts.into_iter().map(|(term, _)| term).collect(),
            summary: chunk.summary,
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
            if let Some(term) = self.terms.get(word).copied().or_else(|| self.closest(word))
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
                let len = self.lens[slot as usize];
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

    /// The indexed word `word` most likely misspells: within one edit up to seven characters and
    /// two beyond, its length within that many of the word's, best alignment first, then the
    /// word in more chunks. Words under four characters are too short to tell a typo from another
    /// word.
    fn closest(&self, word: &str) -> Option<u32> {
        let chars = word.chars().count();
        if chars < 4 {
            return None;
        }
        let typos = if chars <= 7 { 1 } else { 2 };
        let config = neo_frizbee::Config {
            max_typos: Some(typos as u16),
            sort: neo_frizbee::SortStrategy::Unsorted,
            ..Default::default()
        };
        let mut matcher = neo_frizbee::Matcher::new(word, &config);
        let reach = chars - typos..=(chars + typos).min(self.lengths.len().saturating_sub(1));
        let mut best: Option<(u16, usize, u32)> = None;
        for same in self.lengths.get(reach).into_iter().flatten() {
            for found in matcher.match_list(&same.words) {
                let term = same.terms[found.index as usize];
                let common = self.postings[term as usize].len();
                if common > 0
                    && best.is_none_or(|(score, most, _)| (found.score, common) > (score, most))
                {
                    best = Some((found.score, common, term));
                }
            }
        }
        best.map(|(_, _, term)| term)
    }

    /// Fuses the dense neighbours (ids, nearest first) with the keyword hits for `query`, then
    /// lifts summaries and spreads results across sources. At most 50 hits, best first.
    pub fn rank<'a>(
        &'a self,
        query: &str,
        dense: &[impl AsRef<str>],
        keyword_limit: usize,
    ) -> Vec<Hit<'a>> {
        let ranked = self.ranked(query, dense, keyword_limit);
        let hits = ranked.into_iter().map(|h| Hit {
            id: &self.doc(h.slot).id,
            score: h.score,
            kind: h.kind,
        });
        hits.collect()
    }

    /// The id of the chunk at `slot`, as `keyword` returns it.
    pub fn id(&self, slot: u32) -> &str {
        &self.doc(slot).id
    }

    fn doc(&self, slot: u32) -> &Doc {
        self.docs[slot as usize]
            .as_ref()
            .expect("a ranked slot holds a chunk")
    }

    fn ranked(&self, query: &str, dense: &[impl AsRef<str>], keyword_limit: usize) -> Vec<Ranked> {
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
        let doc = |slot: u32| self.doc(slot);
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
        hits
    }

    /// The fused hits for a search, each with its score rounded to a whole number: `candidates`
    /// of them, from `5 × candidates` keyword hits and the dense neighbours.
    fn search(
        &self,
        focused: &str,
        dense: &[impl AsRef<str>],
        candidates: usize,
    ) -> Vec<(u32, u32)> {
        let mut hits = self.ranked(focused, dense, candidates * 5);
        hits.truncate(candidates);
        hits.into_iter().map(|h| (h.slot, whole(h.score))).collect()
    }

    /// Page chat context: the best `limit` chunks, only from `page` when one is given (same
    /// address up to its query, or one address a prefix of the other), each under a numbered
    /// source line. Empty when nothing matches.
    pub fn context(
        &self,
        focused: &str,
        dense: &[impl AsRef<str>],
        limit: usize,
        page: &str,
    ) -> String {
        let page_base = before_query(page);
        let from_page = |doc: &Doc| {
            let base = before_query(&doc.source_url);
            page.is_empty()
                || (!doc.source_url.is_empty()
                    && (&*doc.source_url == page
                        || base.starts_with(page_base)
                        || page_base.starts_with(base)))
        };
        let mut out = String::new();
        let hits = self
            .search(focused, dense, limit * 4)
            .into_iter()
            .map(|(slot, _)| self.doc(slot));
        for (i, doc) in hits.filter(|doc| from_page(doc)).take(limit).enumerate() {
            if i > 0 {
                out.push_str("\n\n---\n\n");
            }
            out.push_str(&format!(
                "[Source {}: {}]\n{}",
                i + 1,
                doc.label(),
                doc.content
            ));
        }
        out
    }

    /// Global search answer material: the best `limit` sources, one chunk per page, scored up
    /// by how many of the question's keywords they contain, and the numbered context built from
    /// them.
    pub fn sources(
        &self,
        question: &str,
        focused: &str,
        dense: &[impl AsRef<str>],
        limit: usize,
    ) -> Sources<'_> {
        let terms = crate::query::keywords(question);
        let mut hits: Vec<Ranked> = self
            .search(focused, dense, limit * 4)
            .into_iter()
            .enumerate()
            .map(|(at, (slot, score))| {
                let doc = self.doc(slot);
                // terms hold no spaces, so content and title are searched apart, never joined
                let (content, title) = (doc.content.to_lowercase(), doc.title.to_lowercase());
                let found = terms
                    .iter()
                    .filter(|term| content.contains(term.as_str()) || title.contains(term.as_str()))
                    .count();
                let share = if terms.is_empty() {
                    0.5
                } else {
                    found as f64 / terms.len() as f64
                };
                let score = f64::from(whole(f64::from(score) * (1.0 + share)).min(100));
                Ranked {
                    slot,
                    score,
                    kind: Match::Keyword,
                    at: at as u32,
                }
            })
            .collect();
        order(&mut hits);
        let mut pages: Vec<&str> = Vec::new();
        let mut out = Sources {
            context: String::new(),
            sources: Vec::new(),
        };
        for hit in hits {
            let doc = self.doc(hit.slot);
            let page = page_key(if doc.source_url.is_empty() {
                &doc.id
            } else {
                &doc.source_url
            });
            if pages.contains(&page) {
                continue;
            }
            pages.push(page);
            let index = out.sources.len() + 1;
            if index > 1 {
                out.context.push_str("\n\n");
            }
            out.context
                .push_str(&format!("[{index}] {}:\n{}", doc.label(), doc.content));
            out.sources.push(Source {
                index,
                title: doc.label(),
                url: &doc.source_url,
                kind: match &*doc.kind {
                    "email" => "email",
                    "pdf" => "pdf",
                    _ => "page",
                },
                score: hit.score as u32,
                snippet: doc
                    .content
                    .char_indices()
                    .nth(SNIPPET)
                    .map_or(&doc.content, |(end, _)| &doc.content[..end]),
            });
            if index == limit {
                break;
            }
        }
        out
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

    fn put(index: &mut Index, id: &str, content: &str, url: &str, source_id: &str, summary: bool) {
        let chunk = Chunk {
            id,
            kind: "article",
            title: "",
            content,
            source_url: url,
            source_id,
            summary,
        };
        index.add(&chunk);
    }

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
        put(
            &mut index,
            "a",
            "rust ownership rules",
            "https://a",
            "a",
            false,
        );
        put(
            &mut index,
            "b",
            "rust rust rust ownership borrowing lifetimes and many other words",
            "https://b",
            "b",
            false,
        );
        put(&mut index, "c", "gardening tips", "https://c", "c", false);
        let hits = index.rank("rust ownership", &[] as &[&str], 10);
        assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), ["a", "b"]);
        assert!(hits.iter().all(|h| h.kind == Match::Keyword));
        index.remove("a");
        assert!(!index.has_source("https://a"));
        assert_eq!(index.rank("ownership", &[] as &[&str], 10)[0].id, "b");
    }

    #[test]
    fn finds_the_word_a_typo_misspells() {
        let mut index = Index::default();
        put(
            &mut index,
            "a",
            "schema migrations run first",
            "",
            "",
            false,
        );
        put(&mut index, "b", "the scheme of things", "", "", false);
        put(&mut index, "c", "reading about receiving", "", "", false);
        assert_eq!(index.keyword("shcema", 5), [0]);
        assert_eq!(index.keyword("recieving", 5), [2]);
        assert!(index.keyword("xyz qqqq", 5).is_empty());
    }

    #[test]
    fn fusion_scores_a_double_first_place_100_and_penalises_repeated_sources() {
        let mut index = Index::default();
        put(
            &mut index,
            "p:1",
            "alpha beta",
            "https://x/page?a=1",
            "x",
            false,
        );
        put(
            &mut index,
            "p:2",
            "alpha gamma",
            "https://x/page#two",
            "x",
            false,
        );
        put(
            &mut index,
            "s",
            "summary of alpha",
            "https://y",
            "y:summary",
            true,
        );
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

    #[test]
    fn context_keeps_the_current_page_and_sources_keep_one_chunk_per_page() {
        let mut index = Index::default();
        put(
            &mut index,
            "a:0",
            "tides follow the moon",
            "https://sea.test/tides?x=1",
            "a",
            false,
        );
        put(
            &mut index,
            "a:1",
            "spring tides and neap tides",
            "https://sea.test/tides/",
            "a",
            false,
        );
        put(
            &mut index,
            "b:0",
            "moon landing history",
            "https://space.test/moon",
            "b",
            false,
        );
        let context = index.context("tides moon", &["a:0"], 3, "https://sea.test/tides");
        assert_eq!(context.matches("[Source ").count(), 2);
        assert!(!context.contains("landing"));
        assert!(
            context.starts_with("[Source 1: https://sea.test/tides?x=1]\ntides follow the moon")
        );
        assert_eq!(
            index.context("tides", &[] as &[&str], 3, "https://other.test"),
            ""
        );

        let found = index.sources("what about tides and the moon", "tides moon", &["a:0"], 5);
        let urls: Vec<_> = found.sources.iter().map(|s| s.url).collect();
        assert_eq!(
            urls,
            ["https://sea.test/tides?x=1", "https://space.test/moon"]
        );
        assert_eq!(
            (
                found.sources[0].index,
                found.sources[0].kind,
                found.sources[0].score
            ),
            (1, "page", 100)
        );
        assert!(
            found
                .context
                .starts_with("[1] https://sea.test/tides?x=1:\ntides follow the moon\n\n[2] ")
        );
    }
}
