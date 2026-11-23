//! Auto summary: a page summarizes on its own only when the reader's settings and the
//! page's shape both say it is an article.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const ALLOWLIST: [&str; 34] = [
    "nytimes.com",
    "theguardian.com",
    "bbc.com",
    "cnn.com",
    "reuters.com",
    "apnews.com",
    "npr.org",
    "wsj.com",
    "arstechnica.com",
    "techcrunch.com",
    "theverge.com",
    "wired.com",
    "engadget.com",
    "thenextweb.com",
    "medium.com",
    "substack.com",
    "dev.to",
    "hashnode.com",
    "blog.giganticlist.com",
    "docs.",
    "documentation.",
    "developer.",
    "api.",
    "guide.",
    "docs.github.com",
    "developer.mozilla.org",
    "stackoverflow.com",
    ".edu",
    "arxiv.org",
    "scholar.google.com",
    "wikipedia.org",
    "wikimedia.org",
    "news.ycombinator.com",
    "reddit.com",
];

const DENYLIST: [&str; 30] = [
    "mail.google.com",
    "gmail.com",
    "calendar.google.com",
    "drive.google.com",
    "app.slack.com",
    "web.whatsapp.com",
    "trello.com",
    "asana.com",
    "notion.so",
    "amazon.com",
    "ebay.com",
    "etsy.com",
    "shopify.",
    "cart",
    "checkout",
    "twitter.com",
    "x.com",
    "facebook.com",
    "instagram.com",
    "tiktok.com",
    "linkedin.com",
    "youtube.com",
    "search?",
    "/search",
    "results?",
    "dashboard",
    "admin",
    "console",
    "panel",
    "localhost",
];

const MIN_AUTO_WORDS: f64 = 500.0;
const MIN_PROMPT_WORDS: f64 = 250.0;
/// A page asked for by hand still needs this many words to be worth a summary.
const MIN_MANUAL_WORDS: f64 = 80.0;

/// The settings as `chrome.storage.local.summaryPrefs` keeps them.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    /// manual, auto or smart.
    pub mode: String,
    pub allowlist: Vec<String>,
    pub denylist: Vec<String>,
    pub min_auto_words: f64,
    pub min_prompt_words: f64,
}

fn owned(list: &[&str]) -> Vec<String> {
    list.iter().map(|item| (*item).to_owned()).collect()
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: "manual".into(),
            allowlist: owned(&ALLOWLIST),
            denylist: owned(&DENYLIST),
            min_auto_words: MIN_AUTO_WORDS,
            min_prompt_words: MIN_PROMPT_WORDS,
        }
    }
}

fn list(value: &Value, fallback: Vec<String>) -> Vec<String> {
    let Some(items) = value.as_array() else {
        return fallback;
    };
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => text.trim().to_owned(),
            Value::Null | Value::Bool(false) => String::new(),
            other => other.to_string().trim().to_owned(),
        })
        .filter(|item| !item.is_empty())
        .collect()
}

fn number(value: &Value, fallback: f64) -> f64 {
    value.as_f64().filter(|n| n.is_finite()).unwrap_or(fallback)
}

impl Prefs {
    /// The stored settings over the defaults; a field of the wrong type keeps its default.
    pub fn from_stored(stored: &Value) -> Self {
        let base = Self::default();
        Self {
            mode: stored["mode"].as_str().map_or(base.mode, str::to_owned),
            allowlist: list(&stored["allowlist"], base.allowlist),
            denylist: list(&stored["denylist"], base.denylist),
            min_auto_words: number(&stored["minAutoWords"], base.min_auto_words),
            min_prompt_words: number(&stored["minPromptWords"], base.min_prompt_words),
        }
    }
}

/// A list typed one entry per line or comma separated; the fallback when it holds none.
pub fn parse_list(text: &str, fallback: &[String]) -> Vec<String> {
    let parts: Vec<String> = text
        .split(['\n', ','])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect();
    if parts.is_empty() {
        fallback.to_vec()
    } else {
        parts
    }
}

/// The settings form's fields to the stored settings.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Form {
    pub mode: String,
    pub allowlist: String,
    pub denylist: String,
    pub min_auto_words: Value,
}

impl Form {
    pub fn into_prefs(self) -> Prefs {
        let base = Prefs::default();
        let min_auto_words = number(&self.min_auto_words, base.min_auto_words);
        Prefs {
            allowlist: parse_list(&self.allowlist, &base.allowlist),
            denylist: parse_list(&self.denylist, &base.denylist),
            mode: self.mode,
            min_auto_words,
            min_prompt_words: base.min_prompt_words,
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Decision {
    /// skip, auto, prompt or wait.
    pub action: &'static str,
    pub reason: &'static str,
}

const fn decide(action: &'static str, reason: &'static str) -> Decision {
    Decision { action, reason }
}

/// Whether the extracted page summarizes now (`auto`), asks first (`prompt`), waits for the
/// reader (`wait`) or is not a page to summarize (`skip`).
pub fn classify(page: &Value, prefs: &Prefs, manual: bool) -> Decision {
    let url = page["url"].as_str().unwrap_or("");
    let content_type = page["contentType"].as_str().unwrap_or("text/html");
    let signals = &page["pageSignals"];
    let flag = |name: &str| signals[name].as_bool().unwrap_or(false);
    let measure = |name: &str| signals[name].as_f64().unwrap_or(0.0);
    let words = page["wordCount"].as_f64().unwrap_or(0.0);
    let listed = |list: &[String]| list.iter().any(|entry| url.contains(entry.as_str()));

    if listed(&prefs.denylist) {
        return decide("skip", "denylist");
    }
    // A PDF arrives as the text an extension page read from it.
    if !content_type.starts_with("text/") && content_type != "application/pdf" {
        return decide("skip", "non_text");
    }
    if manual {
        return if words < MIN_MANUAL_WORDS {
            decide("skip", "too_short")
        } else {
            decide("auto", "manual_trigger")
        };
    }
    let not_for_reading = [
        "isSPA",
        "isDashboard",
        "isSearch",
        "isCart",
        "isFeed",
        "isSocialFeed",
        "canvasHeavy",
    ]
    .iter()
    .any(|name| flag(name))
        || measure("buttonToParagraphRatio") > 2.0
        || measure("linkDensity") > 0.55;
    if not_for_reading {
        return decide("skip", "non_reader");
    }
    if prefs.mode == "manual" {
        return decide("wait", "manual_mode");
    }
    let headed = flag("hasArticleTag") || measure("h1Count") > 0.0;
    let density = measure("textDensity");
    let allowed = listed(&prefs.allowlist);
    let confident = if allowed {
        words >= prefs.min_prompt_words
    } else {
        headed && density > 0.4 && words >= prefs.min_auto_words
    };
    if confident {
        return decide(
            "auto",
            if allowed {
                "allowlist"
            } else {
                "article_confident"
            },
        );
    }
    let likely = (flag("hasMain") || headed) && words >= prefs.min_prompt_words && density > 0.25;
    if prefs.mode == "smart" && likely {
        return decide("prompt", "medium_confidence");
    }
    decide("wait", "low_confidence")
}

#[cfg(test)]
mod checks {
    use super::*;
    use serde_json::json;

    fn article(url: &str, words: u32) -> Value {
        json!({"url": url, "wordCount": words,
               "pageSignals": {"hasArticleTag": true, "textDensity": 0.6, "hasMain": true}})
    }

    #[test]
    fn bad_fields_keep_defaults() {
        let prefs = Prefs::from_stored(
            &json!({"mode": "auto", "allowlist": [" a.com ", "", null], "minAutoWords": "x"}),
        );
        assert_eq!(prefs.mode, "auto");
        assert_eq!(prefs.allowlist, ["a.com"]);
        assert_eq!(prefs.denylist, Prefs::default().denylist);
        assert_eq!(prefs.min_auto_words, 500.0);
        assert_eq!(parse_list(" a.com,\nb.org ,", &[]), ["a.com", "b.org"]);
        assert_eq!(parse_list("  ", &["z".into()]), ["z"]);
    }

    #[test]
    fn decisions() {
        let auto = Prefs {
            mode: "auto".into(),
            ..Prefs::default()
        };
        let smart = Prefs {
            mode: "smart".into(),
            ..Prefs::default()
        };
        let manual = Prefs::default();
        assert_eq!(
            classify(&article("https://mail.google.com/x", 900), &auto, true),
            decide("skip", "denylist")
        );
        assert_eq!(
            classify(&json!({"contentType": "image/png"}), &auto, false),
            decide("skip", "non_text")
        );
        let pdf = json!({"contentType": "application/pdf", "wordCount": 900});
        assert_eq!(classify(&pdf, &manual, true), decide("auto", "manual_trigger"));
        assert_eq!(
            classify(&article("https://a.io", 50), &manual, true),
            decide("skip", "too_short")
        );
        assert_eq!(
            classify(&article("https://a.io", 100), &manual, true),
            decide("auto", "manual_trigger")
        );
        assert_eq!(
            classify(&article("https://a.io", 900), &manual, false),
            decide("wait", "manual_mode")
        );
        assert_eq!(
            classify(&article("https://arxiv.org/abs/1", 300), &auto, false),
            decide("auto", "allowlist")
        );
        assert_eq!(
            classify(&article("https://a.io", 900), &auto, false),
            decide("auto", "article_confident")
        );
        assert_eq!(
            classify(&article("https://a.io", 300), &smart, false),
            decide("prompt", "medium_confidence")
        );
        assert_eq!(
            classify(&article("https://a.io", 300), &auto, false),
            decide("wait", "low_confidence")
        );
        let feed =
            json!({"url": "https://a.io", "wordCount": 900, "pageSignals": {"linkDensity": 0.7}});
        assert_eq!(classify(&feed, &auto, false), decide("skip", "non_reader"));
    }
}
