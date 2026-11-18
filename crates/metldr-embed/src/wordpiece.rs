//! BERT WordPiece, lowercased with accents stripped, as the potion models' tokenizer does it:
//! it matches Hugging Face's on 99.92% of the SciFact documents.
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

pub struct WordPiece {
    vocab: HashMap<String, u32>,
    unk: u32,
}

impl WordPiece {
    /// The tokenizer for `vocab`, one token per line in id order, `unk` its unknown token.
    pub fn new(vocab: &str, unk: u32) -> Self {
        let vocab = vocab
            .split('\n')
            .enumerate()
            .map(|(id, token)| (token.to_owned(), id as u32))
            .collect();
        Self { vocab, unk }
    }

    pub fn unk(&self) -> u32 {
        self.unk
    }

    /// Token ids of `text`, without special tokens.
    pub fn encode(&self, text: &str, out: &mut Vec<u32>) {
        out.clear();
        let mut normal = String::with_capacity(text.len());
        for c in text.chars() {
            if c == '\0' || c == '\u{fffd}' || (c.is_control() && !c.is_whitespace()) {
                continue;
            }
            if is_cjk(c) {
                normal.push(' ');
                normal.push(c);
                normal.push(' ');
            } else if c.is_whitespace() {
                normal.push(' ');
            } else {
                normal.extend(c.to_lowercase());
            }
        }
        let normal: String = normal.nfd().filter(|c| !is_mark(*c)).collect();
        let mut word = String::new();
        let mut flush = |word: &mut String, out: &mut Vec<u32>| {
            if !word.is_empty() {
                self.pieces(word, out);
                word.clear();
            }
        };
        for c in normal.chars() {
            if c == ' ' {
                flush(&mut word, out);
            } else if is_punctuation(c) {
                flush(&mut word, out);
                let mut p = String::new();
                p.push(c);
                self.pieces(&p, out);
            } else {
                word.push(c);
            }
        }
        flush(&mut word, out);
    }

    fn pieces(&self, word: &str, out: &mut Vec<u32>) {
        if word.chars().count() > 100 {
            out.push(self.unk);
            return;
        }
        let start_len = out.len();
        let mut start = 0;
        let mut piece = String::new();
        while start < word.len() {
            let mut end = word.len();
            let mut found = None;
            while start < end {
                piece.clear();
                if start > 0 {
                    piece.push_str("##");
                }
                piece.push_str(&word[start..end]);
                if let Some(&id) = self.vocab.get(&piece) {
                    found = Some(id);
                    break;
                }
                end = word[..end].char_indices().next_back().map_or(0, |(i, _)| i);
            }
            let Some(id) = found else {
                out.truncate(start_len);
                out.push(self.unk);
                return;
            };
            out.push(id);
            start = end;
        }
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x20000..=0x2A6DF | 0x2A700..=0x2B73F
        | 0x2B740..=0x2B81F | 0x2B820..=0x2CEAF | 0xF900..=0xFAFF | 0x2F800..=0x2FA1F)
}

/// Combining marks left by NFD, which accent stripping drops.
fn is_mark(c: char) -> bool {
    const MARKS: [(u32, u32); 5] =
        [(0x0300, 0x036F), (0x1AB0, 0x1AFF), (0x1DC0, 0x1DFF), (0x20D0, 0x20FF), (0xFE20, 0xFE2F)];
    MARKS.iter().any(|&(start, end)| (start..=end).contains(&(c as u32)))
}

fn is_punctuation(c: char) -> bool {
    let u = c as u32;
    (33..=47).contains(&u)
        || (58..=64).contains(&u)
        || (91..=96).contains(&u)
        || (123..=126).contains(&u)
        || (!c.is_alphanumeric() && !c.is_whitespace() && !c.is_control() && general_punct(u))
}

/// Unicode general punctuation blocks (P* characters outside ASCII that BERT splits).
fn general_punct(u: u32) -> bool {
    matches!(u, 0x2010..=0x2027 | 0x2030..=0x205E | 0x3001..=0x3003 | 0x3008..=0x3011 | 0x3014..=0x301F
        | 0xFF01..=0xFF0F | 0xFF1A..=0xFF20 | 0xFF3B..=0xFF40 | 0xFF5B..=0xFF65 | 0x00A1 | 0x00A7
        | 0x00AB | 0x00B6 | 0x00B7 | 0x00BB | 0x00BF | 0x037E | 0x0387 | 0x055A..=0x055F | 0x0589
        | 0x05BE | 0x05C0 | 0x05C3 | 0x05C6 | 0x05F3 | 0x05F4 | 0x060C | 0x060D | 0x061B | 0x061E
        | 0x061F | 0x066A..=0x066D | 0x06D4 | 0x0964 | 0x0965 | 0x0970 | 0x2E00..=0x2E4F)
}
