//! The Stats page's numbers in the service worker: the per-day records in `metldr-analytics`,
//! summarized for a period, and each answered gateway chat added to today's record.

use super::*;
use crate::stats::{self, Day};
use web_sys::{IdbDatabase, IdbObjectStoreParameters, IdbOpenDbRequest, IdbTransactionMode};

/// The analytics database the extension's tracker opens at this version, with these stores.
const DATABASE: &str = "metldr-analytics";
const VERSION: u32 = 2;
const DAYS: &str = "daily_stats";
const STORES: [(&str, &str, &str); 3] = [
    (DAYS, "date", "timeInPanel"),
    ("sessions", "id", "startTime"),
    ("events", "id", "timestamp"),
];

/// `{days, summaries, chatMessages, ...}` for the `days` up to today; zeros before any use.
#[wasm_bindgen]
pub async fn stats_summary(days: u32) -> JsValue {
    let all: Vec<Day> = match database().await {
        Ok(db) => {
            let read = async {
                let store = db
                    .transaction_with_str(DAYS)
                    .and_then(|tx| tx.object_store(DAYS))
                    .map_err(err)?;
                keys::done(&store.get_all().map_err(err)?).await
            };
            let found = read
                .await
                .map(|records| from_js(&records))
                .unwrap_or(Value::Null);
            db.close();
            serde_json::from_value(found).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };
    to_js(&json!(stats::summarize(&all, &today(), days)))
}

/// Adds one answered chat to today's record.
pub(super) async fn record_chat(usage: &Value, ms: f64, model: &str) {
    let tokens = usage["input"].as_u64().zip(usage["output"].as_u64());
    record(&stats::Event::Chat { tokens, ms, model }).await;
}

/// `event` added to today's record in one read and one write; a failure only loses the count.
pub(super) async fn record(event: &stats::Event<'_>) {
    let Ok(db) = database().await else { return };
    let recorded = async {
        let tx = db
            .transaction_with_str_and_mode(DAYS, IdbTransactionMode::Readwrite)
            .map_err(err)?;
        let store = tx.object_store(DAYS).map_err(err)?;
        let date = today();
        let day = from_js(&keys::done(&store.get(&date.as_str().into()).map_err(err)?).await?);
        store
            .put(&to_js(&stats::record(day, &date, event)))
            .map_err(err)?;
        dictionary::finished(&tx).await
    };
    let _ = recorded.await;
    db.close();
}

/// Counts what the extension's page tools did: `{kind: "summary", email, wordsIn, wordsOut,
/// tokensOut, ms, model, url}`, `{kind: "lookup"}` or `{kind: "replies", count}`; and a finished
/// companion run, `{kind: "run", input, output, ms, model}`, as one chat.
#[wasm_bindgen]
pub async fn stats_record(event: JsValue) {
    let event = from_js(&event);
    let number = |key: &str| event[key].as_f64().unwrap_or(0.0);
    let text = |key: &str| event[key].as_str().unwrap_or_default();
    match text("kind") {
        "summary" => {
            let summary = stats::Event::Summary {
                email: event["email"] == true,
                words_in: number("wordsIn"),
                words_out: number("wordsOut"),
                tokens_out: number("tokensOut"),
                ms: number("ms"),
                model: text("model"),
                site: stats::site(text("url")),
            };
            record(&summary).await;
        }
        "run" => {
            let tokens = event["input"].as_u64().zip(event["output"].as_u64());
            record(&stats::Event::Chat { tokens, ms: number("ms"), model: text("model") }).await;
        }
        "lookup" => record(&stats::Event::Lookup).await,
        "replies" => record(&stats::Event::Replies(number("count"))).await,
        _ => {}
    }
}

/// Today as the tracker keys it: the UTC date, YYYY-MM-DD.
fn today() -> String {
    String::from(js_sys::Date::new_0().to_iso_string())
        .chars()
        .take(10)
        .collect()
}

/// The analytics database, created with the tracker's stores and indexes when it is missing.
async fn database() -> Result<IdbDatabase, Error> {
    let factory: web_sys::IdbFactory = get(&js_sys::global(), "indexedDB")
        .dyn_into()
        .map_err(|_| "No IndexedDB here.")?;
    let open: IdbOpenDbRequest = factory.open_with_u32(DATABASE, VERSION).map_err(err)?;
    let upgrade = Closure::once_into_js({
        let open = open.clone();
        move || {
            let Some(db) = open
                .result()
                .ok()
                .and_then(|r| r.dyn_into::<IdbDatabase>().ok())
            else {
                return;
            };
            let names = db.object_store_names();
            for (name, key, index) in STORES
                .into_iter()
                .filter(|(name, ..)| !names.contains(name))
            {
                let params = IdbObjectStoreParameters::new();
                params.set_key_path(&key.into());
                if let Ok(store) = db.create_object_store_with_optional_parameters(name, &params) {
                    let _ = store.create_index_with_str(index, index);
                }
            }
        }
    });
    open.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    keys::done(&open)
        .await?
        .dyn_into()
        .map_err(|_| "The stats could not be opened.".into())
}
