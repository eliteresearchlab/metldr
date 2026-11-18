//! Search query text: the topic words of a remembering question, the local expansion when no
//! model is available, the terms a model suggests, and the keywords that confirm a hit.

/// A phrase to drop: fixed words, then words that may follow it (one from each list, in order).
struct Phrase {
    words: &'static [&'static [&'static str]],
    optional: &'static [&'static [&'static str]],
}

/// Ways of asking about something read earlier; none of them names the topic.
const MEMORY_PHRASES: &[Phrase] = &[
    Phrase {
        words: &[&["what"], &["was"]],
        optional: &[&["the", "that", "a"]],
    },
    Phrase {
        words: &[&["where"], &["did"], &["i"], &["read", "see", "find"]],
        optional: &[&["about"]],
    },
    Phrase {
        words: &[
            &["i"],
            &["read", "saw", "found", "remember", "recall", "forgot"],
        ],
        optional: &[&["about", "that"]],
    },
    Phrase {
        words: &[&["can"], &["you"], &["find", "remind"]],
        optional: &[&["about"]],
    },
    Phrase {
        words: &[&["can"], &["you"], &["tell"], &["me"]],
        optional: &[&["about"]],
    },
    Phrase {
        words: &[&["do"], &["you"], &["remember", "know"]],
        optional: &[&["about"]],
    },
    Phrase {
        words: &[&["that"], &["i"], &["read", "saw", "found"]],
        optional: &[&["about"]],
    },
    Phrase {
        words: &[&["i"], &["forgot"]],
        optional: &[&["about", "what"]],
    },
    Phrase {
        words: &[&["remind"], &["me"]],
        optional: &[&["about", "of", "what"]],
    },
    Phrase {
        words: &[&["what"], &["did"], &["i"]],
        optional: &[],
    },
    Phrase {
        words: &[&["show"], &["me"]],
        optional: &[&["the", "that", "a"]],
    },
    Phrase {
        words: &[&["find"]],
        optional: &[&["the", "that", "a", "my"]],
    },
];

const STOPWORDS: &[&str] = &[
    "the", "and", "but", "for", "with", "from", "was", "are", "were", "been", "have", "has", "had",
    "does", "did", "will", "would", "could", "should", "may", "might", "must", "can", "this",
    "that", "these", "those", "you", "your", "our", "they", "their", "its", "what", "which", "who",
    "whom", "where", "when", "why", "how", "all", "each", "every", "both", "few", "more", "most",
    "other", "some", "any", "not", "only", "own", "same", "than", "too", "very", "just", "also",
    "now", "here", "there", "then", "once", "again", "read", "saw", "found", "remember", "recall",
    "forgot", "remind", "show", "find", "tell", "know", "about", "thing",
];

const SYNONYMS: &[(&str, &[&str])] = &[
    ("order", &["purchase", "bought", "confirmation", "shipping"]),
    (
        "orders",
        &["purchases", "bought", "confirmations", "shipping"],
    ),
    ("invoice", &["bill", "receipt", "payment", "statement"]),
    ("invoices", &["bills", "receipts", "payments", "statements"]),
    ("email", &["mail", "message", "notification"]),
    ("emails", &["mails", "messages", "notifications"]),
    ("article", &["post", "blog", "news", "story"]),
    ("articles", &["posts", "blogs", "news", "stories"]),
];

const MAX_SUGGESTED: usize = 12;

/// The topic words of `query`, lowercase and single spaced: "what was the pdf I read about
/// tides" becomes "pdf tides".
pub fn focus(query: &str) -> String {
    let lower = query.to_lowercase();
    let mut words: Vec<&str> = lower.split_whitespace().collect();
    for phrase in MEMORY_PHRASES {
        let mut at = 0;
        while at < words.len() {
            match phrase.length_at(&words[at..]) {
                Some(length) => drop(words.drain(at..at + length)),
                None => at += 1,
            }
        }
    }
    words.join(" ")
}

impl Phrase {
    fn length_at(&self, words: &[&str]) -> Option<usize> {
        if words.len() < self.words.len()
            || !self
                .words
                .iter()
                .zip(words)
                .all(|(choices, word)| choices.contains(word))
        {
            return None;
        }
        let mut length = self.words.len();
        for choices in self.optional {
            if words.get(length).is_some_and(|word| choices.contains(word)) {
                length += 1;
            }
        }
        Some(length)
    }
}

/// `query` with related words for the common nouns it names, used when no model can expand it.
pub fn expand_locally(query: &str) -> String {
    let lower = query.to_lowercase();
    let mut out = query.to_owned();
    for word in lower.split_whitespace() {
        if let Some((_, extra)) = SYNONYMS.iter().find(|(noun, _)| *noun == word) {
            for synonym in *extra {
                out.push(' ');
                out.push_str(synonym);
            }
        }
    }
    out
}

/// Up to twelve search terms from a model's comma or semicolon separated answer, lowercase,
/// skipping fragments too short, too long or labelled ("terms: ...").
pub fn suggested_terms(answer: &str) -> Vec<String> {
    answer
        .split([',', ';'])
        .map(|part| {
            part.chars()
                .filter(|c| !matches!(c, '"' | '\'' | '\n'))
                .collect::<String>()
        })
        .map(|part| part.trim().to_lowercase())
        .filter(|term| (3..40).contains(&term.chars().count()) && !term.contains(':'))
        .take(MAX_SUGGESTED)
        .collect()
}

/// The words of `query` that must appear in a relevant chunk: three or more characters and not
/// a stopword.
pub fn keywords(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .filter(|word| word.chars().count() > 2 && !STOPWORDS.contains(word))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembering_phrases_leave_the_topic() {
        assert_eq!(
            focus("What was the PDF I read about   islam and ummah"),
            "pdf islam and ummah"
        );
        assert_eq!(focus("can you tell me about the theory"), "the theory");
        assert_eq!(focus("find theory of everything"), "theory of everything");
        assert_eq!(focus("remind me of my invoices"), "my invoices");
    }

    #[test]
    fn expansion_and_keywords_follow_the_query_words() {
        assert_eq!(
            expand_locally("Orders from May"),
            "Orders from May purchases bought confirmations shipping"
        );
        assert_eq!(
            suggested_terms("\"Islam\", ummah; ok, terms: x, faith\n"),
            ["islam", "ummah", "faith"]
        );
        assert_eq!(
            keywords("Where did I read about Rust ownership"),
            ["rust", "ownership"]
        );
    }
}
