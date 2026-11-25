//! A first summary made of the page's own sentences, for a reader with no model set up: the
//! sentences most central to the page (LexRank: TF-IDF cosine similarity between sentences,
//! ranked with PageRank), each weighed down gently by how late it comes, shown in reading order.
//! Chosen over lead sentences, TextRank with BM25 and centroid scoring on news (CNN/DailyMail)
//! and paper (arXiv) summaries.

use std::collections::HashMap;

/// Sentences shorter than this read as captions or fragments.
const MIN_WORDS: usize = 5;
/// Sentences longer than this are run-on lists or lost punctuation.
const MAX_WORDS: usize = 80;
/// Only the first sentences are ranked: the work grows with their square.
const MAX_SENTENCES: usize = 400;
const DAMPING: f32 = 0.85;
const ROUNDS: usize = 30;
/// How strongly a later sentence is weighed down: its score over (position + 1) to this power.
const LATENESS: f32 = 0.25;

/// The sentences of `markdown` that best summarize it, in reading order, until they hold
/// `budget` words; empty when the page has fewer than three sentences to choose from.
pub fn first_summary(markdown: &str, budget: usize) -> Vec<&str> {
    let all = sentences(markdown);
    let candidates: Vec<(usize, &str)> = all
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, s)| (MIN_WORDS..=MAX_WORDS).contains(&s.split_whitespace().count()) && ends_a_sentence(s))
        .take(MAX_SENTENCES)
        .collect();
    if candidates.len() < 3 {
        return Vec::new();
    }
    let vectors = tfidf(candidates.iter().map(|(_, s)| *s));
    let ranks = pagerank(&vectors);
    let top = ranks.iter().copied().fold(0.0, f32::max).max(f32::MIN_POSITIVE);
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    let score = |i: usize| ranks[i] / top / (candidates[i].0 as f32 + 1.0).powf(LATENESS);
    order.sort_by(|&a, &b| score(b).total_cmp(&score(a)));
    let mut chosen = Vec::new();
    let mut words = 0;
    for i in order {
        if words >= budget {
            break;
        }
        words += candidates[i].1.split_whitespace().count();
        chosen.push(i);
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|i| candidates[i].1).collect()
}

/// Whether `sentence` closes with a stop, so taglines, captions and menu lines are left out.
fn ends_a_sentence(sentence: &str) -> bool {
    sentence.trim_end_matches(['"', '\'', ')', ']', '”', '’']).ends_with(['.', '!', '?'])
}

/// The prose sentences of `markdown`: headings, tables and code are left out, list markers and
/// emphasis stay as written. A stop (. ! ?) ends a sentence when a space and then a capital,
/// digit or quote follow, unless it closes an abbreviation or an initial; a line break between
/// blocks always ends one.
fn sentences(markdown: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('|') {
            continue;
        }
        let body = trimmed.trim_start_matches(['-', '*', '+', '>', ' ']);
        let body = body.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches(['.', ')', ' ']);
        split(if body.is_empty() { trimmed } else { body }, &mut out);
    }
    out
}

fn split<'a>(text: &'a str, out: &mut Vec<&'a str>) {
    const ABBREVIATIONS: [&str; 22] = [
        "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e", "u.s", "u.k", "no",
        "fig", "eq", "al", "inc", "ltd", "co", "approx",
    ];
    let bytes = text.as_bytes();
    let mut start = 0;
    for (at, &byte) in bytes.iter().enumerate() {
        if !matches!(byte, b'.' | b'!' | b'?') {
            continue;
        }
        let mut end = at + 1;
        while end < bytes.len() && matches!(bytes[end], b'"' | b'\'' | b')' | b']') {
            end += 1;
        }
        if end >= bytes.len() || !bytes[end].is_ascii_whitespace() {
            continue;
        }
        let next = text[end..].trim_start().chars().next();
        let opens = next.is_some_and(|c| c.is_uppercase() || c.is_ascii_digit() || "\"'([“‘".contains(c));
        let word_start = text[..at].rfind(char::is_whitespace).map_or(0, |space| space + 1);
        let word = text[word_start..at].trim_start_matches(['"', '(', '\'']).to_ascii_lowercase();
        let initial = word.chars().count() == 1 && word.chars().all(char::is_alphabetic);
        if !opens || byte == b'.' && (initial || ABBREVIATIONS.contains(&word.as_str())) {
            continue;
        }
        let sentence = text[start..end].trim();
        if !sentence.is_empty() {
            out.push(sentence);
        }
        start = end;
    }
    let rest = text[start..].trim();
    if !rest.is_empty() {
        out.push(rest);
    }
}

/// Each sentence's TF-IDF vector, unit length, as (term, weight) sorted by term.
fn tfidf<'a>(sentences: impl Iterator<Item = &'a str>) -> Vec<Vec<(u32, f32)>> {
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut counts: Vec<Vec<(u32, f32)>> = Vec::new();
    for sentence in sentences {
        let mut terms: Vec<u32> = Vec::new();
        metldr_retrieval::tokenize(sentence, &mut |word| {
            let next = ids.len() as u32;
            terms.push(*ids.entry(word.to_owned()).or_insert(next));
        });
        terms.sort_unstable();
        let mut tf: Vec<(u32, f32)> = Vec::with_capacity(terms.len());
        for term in terms {
            match tf.last_mut() {
                Some((last, n)) if *last == term => *n += 1.0,
                _ => tf.push((term, 1.0)),
            }
        }
        counts.push(tf);
    }
    let mut df = vec![0.0f32; ids.len()];
    for tf in &counts {
        for &(term, _) in tf {
            df[term as usize] += 1.0;
        }
    }
    let n = counts.len() as f32;
    for tf in &mut counts {
        for (term, weight) in tf.iter_mut() {
            *weight *= (n / df[*term as usize]).ln() + 1.0;
        }
        let norm = tf.iter().map(|(_, w)| w * w).sum::<f32>().sqrt();
        if norm > 0.0 {
            tf.iter_mut().for_each(|(_, w)| *w /= norm);
        }
    }
    counts
}

fn dot(a: &[(u32, f32)], b: &[(u32, f32)]) -> f32 {
    let (mut i, mut j, mut sum) = (0, 0, 0.0);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                sum += a[i].1 * b[j].1;
                i += 1;
                j += 1;
            }
        }
    }
    sum
}

/// PageRank over the sentences' cosine similarities (each row normalized to sum to one).
fn pagerank(vectors: &[Vec<(u32, f32)>]) -> Vec<f32> {
    let n = vectors.len();
    let mut weights = vec![0.0f32; n * n];
    for i in 0..n {
        for j in i + 1..n {
            let similarity = dot(&vectors[i], &vectors[j]);
            weights[i * n + j] = similarity;
            weights[j * n + i] = similarity;
        }
    }
    for row in weights.chunks_exact_mut(n) {
        let sum: f32 = row.iter().sum();
        if sum > 0.0 {
            row.iter_mut().for_each(|w| *w /= sum);
        }
    }
    let mut rank = vec![1.0 / n as f32; n];
    let mut next = vec![0.0f32; n];
    for _ in 0..ROUNDS {
        next.fill((1.0 - DAMPING) / n as f32);
        for (i, row) in weights.chunks_exact(n).enumerate() {
            let share = DAMPING * rank[i];
            for (to, w) in next.iter_mut().zip(row) {
                *to += share * w;
            }
        }
        std::mem::swap(&mut rank, &mut next);
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_summarized_by_its_central_sentences_in_reading_order() {
        let page = "# Rust\n\nFrom Wikipedia, the free encyclopedia\n\nRust is a programming language that is fast and memory safe. \
            Mr. Hoare started Rust as a personal project in 2006. The weather was nice that day. \
            Rust's ownership model makes the language memory safe without a garbage collector. \
            Many companies now write fast and memory safe services in the Rust language.\n\n\
            ```\nfn main() {}\n```\n\n| a | b |\n\n- Cargo builds Rust programs and fetches the crates they use.";
        let summary = first_summary(page, 25);
        assert_eq!(summary[0], "Rust is a programming language that is fast and memory safe.");
        assert!(summary.iter().all(|s| !s.contains("fn main") && !s.contains("encyclopedia") && !s.starts_with('|') && !s.starts_with('#')));
        assert!(sentences(page).contains(&"Mr. Hoare started Rust as a personal project in 2006."));
        assert!(first_summary("Too short. Really.", 50).is_empty());
    }
}
