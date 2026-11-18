//! Downloadable dictionaries for the word popup: the languages offered, where each letter's
//! file lives, and the records a letter file becomes, one per word with all its senses.

use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap};

const BASE: &str =
    "https://media.githubusercontent.com/media/WhiteHades/wikitionary-dictionary-json/master/dist";

/// Each language as its code, its name and its folder in the dictionary files.
pub const LANGUAGES: [(&str, &str, &str); 22] = [
    ("en", "English", "English"),
    ("es", "Spanish", "Spanish"),
    ("fr", "French", "French"),
    ("de", "German", "German"),
    ("it", "Italian", "Italian"),
    ("pt", "Portuguese", "Portuguese"),
    ("nl", "Dutch", "Dutch"),
    ("sv", "Swedish", "Swedish"),
    ("pl", "Polish", "Polish"),
    ("ro", "Romanian", "Romanian"),
    ("cs", "Czech", "Czech"),
    ("fi", "Finnish", "Finnish"),
    ("da", "Danish", "Danish"),
    ("no", "Norwegian", "Norwegian"),
    ("nb", "Norwegian (Bokmål)", "Norwegian%20(Bokm%C3%A5l)"),
    ("nn", "Norwegian (Nynorsk)", "Norwegian%20(Nynorsk)"),
    ("id", "Indonesian", "Indonesian"),
    ("ms", "Malay", "Malay"),
    ("tr", "Turkish", "Turkish"),
    ("vi", "Vietnamese", "Vietnamese"),
    ("lt", "Lithuanian", "Lithuanian"),
    ("sk", "Slovak", "Slovak"),
];

/// A language's files come one per letter, in this order.
pub const LETTERS: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";

/// The offered language with this code, as stored: its code is `'static`.
pub fn language(code: &str) -> Option<&'static str> {
    LANGUAGES
        .iter()
        .find(|(c, ..)| *c == code)
        .map(|(c, ..)| *c)
}

/// The file of `code`'s words that start with letter number `index`.
pub fn letter_url(code: &str, index: usize) -> Option<String> {
    let (.., folder) = LANGUAGES.iter().find(|(c, ..)| *c == code)?;
    let letter = char::from(*LETTERS.get(index)?);
    Some(format!("{BASE}/{folder}/{letter}.json"))
}

#[derive(Deserialize)]
struct Entry<'a> {
    #[serde(borrow)]
    word: Cow<'a, str>,
    #[serde(borrow, default)]
    pos: Cow<'a, str>,
    #[serde(borrow, default)]
    definition: Cow<'a, str>,
}

/// One word's record: its first sense, and the senses after it as `[pos, definition]`.
#[derive(Serialize)]
struct Record<'a> {
    word: Cow<'a, str>,
    pos: &'a str,
    definition: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    more: Vec<(&'a str, &'a str)>,
}

/// A letter file, `[{word, pos, definition}]` with one entry per sense, as a JSON array of
/// records keyed by the lowercase word, each holding every sense in file order, and how many
/// records it holds. None when the file is not that shape.
pub fn records(body: &[u8]) -> Option<(Vec<u8>, usize)> {
    let entries: Vec<Entry> = serde_json::from_slice(body).ok()?;
    let mut index = HashMap::with_capacity(entries.len());
    let mut records: Vec<Record> = Vec::with_capacity(entries.len());
    for entry in &entries {
        if entry.definition.is_empty() {
            continue;
        }
        let word = lower(&entry.word);
        match index.get(word.as_ref()) {
            Some(&at) => {
                let record: &mut Record = &mut records[at];
                record.more.push((&entry.pos, &entry.definition));
            }
            None => {
                index.insert(word.clone(), records.len());
                records.push(Record {
                    word,
                    pos: &entry.pos,
                    definition: &entry.definition,
                    more: Vec::new(),
                });
            }
        }
    }
    let mut out = Vec::with_capacity(body.len() + records.len() * 4);
    serde_json::to_writer(&mut out, &records).ok()?;
    Some((out, records.len()))
}

/// `word` as lookups key it, borrowed when it already is: most dictionary words are.
fn lower(word: &str) -> Cow<'_, str> {
    let word = word.trim();
    if word
        .bytes()
        .any(|b| b.is_ascii_uppercase() || !b.is_ascii())
    {
        Cow::Owned(word.to_lowercase())
    } else {
        Cow::Borrowed(word)
    }
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn every_sense_of_a_word_reaches_one_record() {
        let file = br#"[{"word":"a","pos":"letter","definition":"1. First."},
            {"word":"A","pos":"symbol","definition":"1. Grade."},
            {"word":"ab","pos":"noun","definition":""},
            {"word":"ab","pos":"noun","definition":"1. Muscle."}]"#;
        let (json, count) = records(file).unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            String::from_utf8(json).unwrap(),
            r#"[{"word":"a","pos":"letter","definition":"1. First.","more":[["symbol","1. Grade."]]},{"word":"ab","pos":"noun","definition":"1. Muscle."}]"#
        );
        assert_eq!(
            letter_url("nb", 25).unwrap(),
            format!("{BASE}/Norwegian%20(Bokm%C3%A5l)/z.json")
        );
        assert!(letter_url("xx", 0).is_none() && letter_url("en", 26).is_none());
    }
}
