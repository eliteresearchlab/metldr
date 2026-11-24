//! A page's summary for the page tools: read from the cache by address, else written by the
//! preferred route, cached and counted in stats. The cache's schema is the extension's; this
//! never creates it.

use super::*;
use crate::{page, stats as counts};
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbTransactionMode};

pub(super) const CACHE: &str = "metldr_cache";
const PAGES: &str = "page_cache";
/// How long a cached summary is meant to last, in milliseconds, as the cache records it.
const KEEP_MS: f64 = 3_600_000.0;

/// The page as the extension's extractor read it.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    title: String,
    #[serde(default)]
    url: Option<String>,
    content: String,
    author: Option<String>,
    publish_date: Option<String>,
    publication: Option<String>,
    word_count: Option<f64>,
    read_time: Option<String>,
    extraction_time: Option<f64>,
    /// The page's type: `application/pdf` for the text an extension page read from a PDF.
    #[serde(default)]
    content_type: Option<String>,
}

/// The cache, when the extension has made it; opening never creates it.
pub(super) async fn cache() -> Option<IdbDatabase> {
    let factory: web_sys::IdbFactory = get(&js_sys::global(), "indexedDB").dyn_into().ok()?;
    let open: IdbOpenDbRequest = factory.open(CACHE).ok()?;
    let refuse = Closure::once_into_js({
        let open = open.clone();
        move || {
            if let Some(tx) = open.transaction() {
                let _ = tx.abort();
            }
        }
    });
    open.set_onupgradeneeded(Some(refuse.unchecked_ref()));
    let db: IdbDatabase = keys::done(&open).await.ok()?.dyn_into().ok()?;
    if db.object_store_names().contains(PAGES) {
        Some(db)
    } else {
        db.close();
        None
    }
}

async fn cached(url: &str) -> Option<Value> {
    let db = cache().await?;
    let read = async {
        let store = db
            .transaction_with_str(PAGES)
            .and_then(|tx| tx.object_store(PAGES))
            .ok()?;
        let entry = keys::done(&store.get(&url.into()).ok()?).await.ok()?;
        Some(from_js(&entry)["summary"].take()).filter(Value::is_object)
    };
    let found = read.await;
    db.close();
    found
}

async fn keep(url: &str, summary: &Value) {
    let Some(db) = cache().await else { return };
    let entry =
        json!({"url": url, "summary": summary, "timestamp": js_sys::Date::now(), "ttl": KEEP_MS});
    let written = async {
        let tx = db
            .transaction_with_str_and_mode(PAGES, IdbTransactionMode::Readwrite)
            .map_err(err)?;
        tx.object_store(PAGES)
            .and_then(|store| store.put(&to_js(&entry)))
            .map_err(err)?;
        dictionary::finished(&tx).await
    };
    let _ = written.await;
    db.close();
}

/// The summary of `page`: `{title, bullets, timing, ...}`, the cached one unless `force`.
pub(super) async fn gateway_page_summary(page: JsValue, force: bool) -> Result<JsValue, JsValue> {
    let started = js_sys::Date::now();
    let page: Page =
        serde_json::from_value(from_js(&page)).map_err(|_| js_error("That page is not valid."))?;
    let url = page.url.as_deref().filter(|url| !url.is_empty());
    if !force
        && let Some(mut summary) = match url {
            Some(url) => cached(url).await,
            None => None,
        }
    {
        summary["timing"]["cached"] = json!(true);
        summary["timing"]["total"] = json!((js_sys::Date::now() - started).round());
        return Ok(to_js(&summary));
    }
    let mut content = format!("ARTICLE METADATA:\n- Title: \"{}\"\n", page.title);
    let about = [
        ("Author", &page.author),
        ("Publication", &page.publication),
        ("Published", &page.publish_date),
    ];
    for (label, value) in about {
        if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
            content.push_str(&format!("- {label}: {value}\n"));
        }
    }
    content.push_str("\n---\nARTICLE CONTENT:\n\n");
    content.push_str(&page.content);
    let request = Summarize {
        content,
        kind: Some("key-points".into()),
        length: Some("medium".into()),
        context: Some(format!("This article is titled \"{}\"", page.title)),
    };
    let asked = js_sys::Date::now();
    let (text, _, model) = summarized(&request).await.map_err(js_error)?;
    let llm = (js_sys::Date::now() - asked).round();
    let mut bullets = page::bullets(&text);
    if bullets.is_empty() {
        bullets.push("could not generate summary".into());
    }
    let words_out = bullets
        .iter()
        .map(|b| b.split_whitespace().count())
        .sum::<usize>() as f64;
    let timing = json!({"extraction": page.extraction_time.unwrap_or(0.0), "llm": llm,
        "total": (js_sys::Date::now() - started).round(), "cached": false, "model": model});
    let summary = json!({"title": page.title, "author": page.author, "publishDate": page.publish_date,
        "publication": page.publication, "bullets": bullets, "readTime": page.read_time,
        "wordCount": page.word_count, "timestamp": js_sys::Date::now(),
        "timing": timing});
    if let Some(url) = url {
        keep(url, &summary).await;
    }
    let event = counts::Event::Summary {
        of: if page.content_type.as_deref() == Some("application/pdf") { counts::Read::Pdf } else { counts::Read::Page },
        words_in: page.word_count.unwrap_or(0.0),
        words_out,
        tokens_out: (text.len() as f64 / 4.0).round(),
        ms: llm,
        model: &model,
        site: counts::site(url.unwrap_or_default()),
    };
    stats::record(&event).await;
    Ok(to_js(&summary))
}

/// Clears the cached page summaries. It never creates the cache.
pub(super) async fn summary_cache_clear() -> Result<(), JsValue> {
    let Some(db) = cache().await else {
        return Ok(());
    };
    let cleared = async {
        let tx = db
            .transaction_with_str_and_mode(PAGES, IdbTransactionMode::Readwrite)
            .map_err(err)?;
        tx.object_store(PAGES)
            .and_then(|s| s.clear())
            .map_err(err)?;
        dictionary::finished(&tx).await
    };
    let cleared = cleared.await;
    db.close();
    cleared.map_err(|_| js_error("The cached summaries could not be cleared."))
}
