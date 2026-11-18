//! Word lookup for the page's word popup: the order languages are tried in, the shape of a
//! definition from the free dictionary API, from a model, or from a downloaded dictionary.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Where the word sits on the page, as the content script sends it.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    #[serde(default)]
    pub full_sentence: String,
    #[serde(default)]
    pub context_before: String,
    #[serde(default)]
    pub context_after: String,
    /// The page's declared language, which saves a detection.
    #[serde(default)]
    pub page_lang: String,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition {
    pub definition: String,
    pub part_of_speech: String,
    pub example: Option<String>,
    pub synonyms: Vec<String>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Lookup {
    pub definitions: Vec<Definition>,
    pub synonyms: Vec<String>,
    pub language: String,
    pub source: String,
}

/// The word as dictionaries key it.
pub fn normalize(word: &str) -> String {
    word.trim().to_lowercase()
}

/// The languages the reader chose, from any shape the settings have held: a list, an object of
/// codes, or one code. English when none.
pub fn chosen(stored: &Value) -> Vec<String> {
    let languages: Vec<String> = match stored {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Value::Object(items) => items
            .values()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Value::String(code) => vec![code.clone()],
        Value::Null => Vec::new(),
        other => vec![other.to_string()],
    };
    if languages.is_empty() {
        vec!["en".into()]
    } else {
        languages
    }
}

/// The detected language first, then the chosen ones, then English as the last resort.
pub fn priority(detected: &str, chosen: &[String]) -> Vec<String> {
    let mut order = vec![detected.to_owned()];
    order.extend(chosen.iter().filter(|code| *code != detected).cloned());
    if detected != "en" && !chosen.iter().any(|code| code == "en") {
        order.push("en".into());
    }
    order
}

/// The text a language is detected from: the sentence, or the word with its neighbours.
pub fn detection_text(word: &str, context: &Context) -> String {
    let sentence = if context.full_sentence.is_empty() {
        format!(
            "{} {word} {}",
            context.context_before, context.context_after
        )
        .trim()
        .to_owned()
    } else {
        context.full_sentence.clone()
    };
    if sentence.len() > word.len() && sentence.len() >= 20 {
        sentence
    } else {
        format!("{sentence} {word}").trim().to_owned()
    }
}

pub const SYSTEM: &str = "You are a dictionary. Define words concisely.";

pub fn prompt(word: &str, language: &str, context: &Context) -> String {
    let context = if !context.full_sentence.is_empty() {
        format!("Context: \"{}\"", context.full_sentence)
    } else if !context.context_before.is_empty() || !context.context_after.is_empty() {
        format!(
            "Context: \"{} [{word}] {}\"",
            context.context_before, context.context_after
        )
    } else {
        String::new()
    };
    format!(
        "{context}\n\nDefine \"{word}\" ({language}). Return: part of speech, \
         definition (15-20 words in English), example sentence, 2 synonyms."
    )
}

const PARTS_OF_SPEECH: [&str; 10] = [
    "noun",
    "verb",
    "adjective",
    "adverb",
    "preposition",
    "conjunction",
    "pronoun",
    "interjection",
    "article",
    "other",
];

/// The structured output a model answers in.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pos": {"type": "string", "enum": PARTS_OF_SPEECH},
            "def": {"type": "string"},
            "example": {"type": "string"},
            "synonyms": {"type": "array", "items": {"type": "string"}, "maxItems": 3}
        },
        "required": ["pos", "def"]
    })
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().map_or_else(Vec::new, |items| {
        items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    })
}

/// A model's answer: its JSON when it kept to the schema, else its text as the definition.
pub fn from_model(text: &str, language: &str, source: &str) -> Lookup {
    let text = text.trim();
    let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let field = |name: &str| {
        parsed[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let synonyms = strings(&parsed["synonyms"]);
    Lookup {
        definitions: vec![Definition {
            definition: field("def")
                .or_else(|| field("definition"))
                .unwrap_or_else(|| text.to_owned()),
            part_of_speech: field("pos").unwrap_or_else(|| "unknown".into()),
            example: field("example"),
            synonyms: synonyms.clone(),
        }],
        synonyms,
        language: language.into(),
        source: source.into(),
    }
}

/// The first entry of a dictionaryapi.dev answer: common parts of speech first, archaic,
/// obsolete, vulgar and stub definitions dropped unless nothing else is left.
pub fn from_api(body: &[u8], language: &str) -> Option<Lookup> {
    let entries: Value = serde_json::from_slice(body).ok()?;
    let entry = &entries[0];
    let meanings = entry["meanings"].as_array().filter(|m| !m.is_empty())?;
    let rank = |pos: &str| {
        ["noun", "verb", "adjective", "adverb"]
            .iter()
            .position(|p| *p == pos)
            .unwrap_or(usize::MAX)
    };
    let mut ordered: Vec<&Value> = meanings.iter().collect();
    ordered.sort_by_key(|m| rank(m["partOfSpeech"].as_str().unwrap_or("")));
    let definition = |meaning: &Value, def: &Value| Definition {
        definition: def["definition"].as_str().unwrap_or("").to_owned(),
        part_of_speech: meaning["partOfSpeech"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_owned(),
        example: def["example"].as_str().map(str::to_owned),
        synonyms: strings(&def["synonyms"]),
    };
    let mut definitions = Vec::new();
    for meaning in &ordered {
        for def in meaning["definitions"].as_array().into_iter().flatten() {
            let text = def["definition"].as_str().unwrap_or("").to_lowercase();
            if text.len() < 10
                || ["vulgar", "archaic", "obsolete"]
                    .iter()
                    .any(|w| text.contains(w))
            {
                continue;
            }
            definitions.push(definition(meaning, def));
        }
    }
    if definitions.is_empty() {
        definitions.push(definition(&meanings[0], &meanings[0]["definitions"][0]));
    }
    let word = entry["word"].as_str().unwrap_or("").to_lowercase();
    let mut synonyms: Vec<String> = Vec::new();
    for meaning in meanings {
        for def in meaning["definitions"].as_array().into_iter().flatten() {
            for synonym in strings(&def["synonyms"]) {
                if !synonym.is_empty()
                    && synonym.to_lowercase() != word
                    && !synonyms.contains(&synonym)
                {
                    synonyms.push(synonym);
                }
            }
        }
    }
    Some(Lookup {
        definitions,
        synonyms,
        language: language.into(),
        source: "api".into(),
    })
}

/// A downloaded dictionary's record: `{word, definition, pos?}`.
pub fn from_local(record: &Value, language: &str) -> Option<Lookup> {
    let sense = |pos: &Value, definition: &Value| {
        Some(Definition {
            definition: definition.as_str()?.to_owned(),
            part_of_speech: pos.as_str().filter(|s| !s.is_empty()).unwrap_or("unknown").to_owned(),
            example: None,
            synonyms: Vec::new(),
        })
    };
    // The first sense, then the word's later senses as `[pos, definition]` pairs.
    let mut definitions = vec![sense(&record["pos"], &record["definition"])?];
    let more = record["more"].as_array().map_or(&[][..], Vec::as_slice);
    definitions.extend(more.iter().filter_map(|pair| sense(&pair[0], &pair[1])));
    Some(Lookup {
        definitions,
        synonyms: Vec::new(),
        language: language.into(),
        source: "local".into(),
    })
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn language_order() {
        assert_eq!(chosen(&json!(null)), ["en"]);
        assert_eq!(chosen(&json!({"a": "de", "b": "fr"})), ["de", "fr"]);
        assert_eq!(chosen(&json!("es")), ["es"]);
        assert_eq!(
            priority("de", &["fr".into(), "de".into()]),
            ["de", "fr", "en"]
        );
        assert_eq!(priority("en", &["fr".into()]), ["en", "fr"]);
    }

    #[test]
    fn api_ranking() {
        let body = br#"[{"word":"Run","meanings":[
            {"partOfSpeech":"interjection","definitions":[{"definition":"Said to make someone flee now."}]},
            {"partOfSpeech":"verb","definitions":[
                {"definition":"To move swiftly on foot.","example":"I run daily.",
                 "synonyms":["sprint","run"]},
                {"definition":"An archaic sense of running."}]}]}]"#;
        let found = from_api(body, "en").unwrap();
        assert_eq!(found.definitions.len(), 2);
        assert_eq!(found.definitions[0].part_of_speech, "verb");
        assert_eq!(
            found.definitions[0].example.as_deref(),
            Some("I run daily.")
        );
        assert_eq!(found.synonyms, ["sprint"]);
        assert!(from_api(b"{\"title\":\"No Definitions Found\"}", "en").is_none());
    }

    #[test]
    fn model_fallback() {
        let found = from_model(
            r#"{"pos":"noun","def":"A small dog.","synonyms":["pup"]}"#,
            "en",
            "ollama",
        );
        assert_eq!(found.definitions[0].definition, "A small dog.");
        assert_eq!(found.synonyms, ["pup"]);
        let plain = from_model("  a small dog  ", "en", "chrome-ai");
        assert_eq!(
            (
                plain.definitions[0].definition.as_str(),
                plain.definitions[0].part_of_speech.as_str()
            ),
            ("a small dog", "unknown")
        );
    }
}
