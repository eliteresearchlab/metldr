//! The downloaded dictionaries in the service worker: one IndexedDB store per language, filled
//! one letter file at a time so a download resumes where it stopped, and what the settings show.

use super::*;
use crate::dictionary::{self, LANGUAGES, LETTERS};
use std::collections::BTreeMap;
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbTransaction, IdbTransactionMode};

/// The database the word lookup reads, one store per language and `meta`.
pub(super) const DATABASE: &str = "metldr-dictionary";
const VERSION: u32 = 3;
const META: &str = "meta";
const SELECTED: &str = "selectedLanguages";
/// Each unfinished download as `{code: {nextIndex, entriesProcessed}}`.
const PROGRESS: &str = "dictDownloadProgress";
const DOWNLOADING: &str = "downloadingLanguages";
const LETTER_MS: u32 = 120_000;
/// The largest letter file read, in bytes; English `s` is the largest at about 14 MB.
const LETTER_BOUND: usize = 48 << 20;

#[derive(Default)]
struct Downloads {
    /// The unfinished downloads: the next letter and the records stored so far.
    progress: BTreeMap<&'static str, (usize, u32)>,
    /// The ones running in this worker.
    running: Vec<&'static str>,
    /// The ones that stored nothing, say offline; only the reader choosing one again retries.
    failed: Vec<&'static str>,
    loaded: bool,
}

thread_local! {
    static DOWNLOADS: RefCell<Downloads> = RefCell::default();
}

/// `{action: "state" | "select" | "delete", code, on}` to the dictionaries as settings show
/// them: `{languages: [{code, name}], selected, downloaded, progress: {code: percent}}`.
/// Reading the state also resumes unfinished downloads and starts the chosen languages.
#[wasm_bindgen]
pub async fn dictionary_request(request: JsValue) -> Result<JsValue, JsValue> {
    let request = from_js(&request);
    let code = request["code"].as_str().and_then(dictionary::language);
    load().await;
    let mut selected = chosen().await;
    match (request["action"].as_str(), code) {
        (Some("state"), _) => {}
        (Some("select"), Some(code)) => {
            selected.retain(|c| c != code);
            if request["on"].as_bool() == Some(true) {
                selected.push(code.into());
                DOWNLOADS.with_borrow_mut(|d| d.failed.retain(|c| *c != code));
            }
            write(SELECTED, json!(selected)).await.map_err(js_error)?;
        }
        (Some("delete"), Some(code)) => {
            remove(code).await.map_err(js_error)?;
            selected.retain(|c| c != code);
            write(SELECTED, json!(selected)).await.map_err(js_error)?;
        }
        _ => return Err(js_error("That dictionary request is not valid.")),
    }
    let downloaded = downloaded().await;
    let wanted = DOWNLOADS.with_borrow(|d| d.progress.keys().copied().collect::<Vec<_>>());
    let chosen = selected.iter().filter_map(|c| dictionary::language(c));
    for code in wanted.into_iter().chain(chosen) {
        if !downloaded.iter().any(|d| d == code) {
            start(code);
        }
    }
    let progress: serde_json::Map<String, Value> = DOWNLOADS.with_borrow(|d| {
        d.progress
            .iter()
            .map(|(code, (next, _))| ((*code).into(), json!(next * 100 / LETTERS.len())))
            .collect()
    });
    let languages: Vec<Value> = LANGUAGES
        .iter()
        .map(|(code, name, _)| json!({"code": code, "name": name}))
        .collect();
    Ok(to_js(&json!({
        "languages": languages,
        "selected": selected,
        "downloaded": downloaded,
        "progress": progress,
    })))
}

/// The languages the reader chose; English until they choose.
async fn chosen() -> Vec<String> {
    let stored = read(SELECTED).await.unwrap_or_default();
    if stored.is_null() {
        return vec!["en".into()];
    }
    stored
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// The unfinished downloads as last saved, read once per worker.
async fn load() {
    if DOWNLOADS.with_borrow(|d| d.loaded) {
        return;
    }
    let stored = read(PROGRESS).await.unwrap_or_default();
    let started = read(DOWNLOADING).await.unwrap_or_default();
    DOWNLOADS.with_borrow_mut(|d| {
        d.loaded = true;
        let started = started
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str);
        for code in started.filter_map(dictionary::language) {
            let saved = &stored[code];
            let next = saved["nextIndex"].as_u64().unwrap_or(0) as usize;
            let records = saved["entriesProcessed"].as_u64().unwrap_or(0) as u32;
            d.progress.insert(code, (next.min(LETTERS.len()), records));
        }
    });
}

/// Saves the unfinished downloads in one write, so a closed worker resumes them.
async fn save() -> Result<(), Error> {
    let (progress, downloading) = DOWNLOADS.with_borrow(|d| {
        let map: serde_json::Map<String, Value> = d
            .progress
            .iter()
            .map(|(code, (next, records))| {
                (
                    (*code).into(),
                    json!({"nextIndex": next, "entriesProcessed": records}),
                )
            })
            .collect();
        (map, d.progress.keys().copied().collect::<Vec<_>>())
    });
    let items = to_js(&json!({PROGRESS: progress, DOWNLOADING: downloading}));
    JsFuture::from(storage_set(&items).map_err(err)?)
        .await
        .map_err(err)?;
    Ok(())
}

fn start(code: &'static str) {
    let fresh = DOWNLOADS.with_borrow_mut(|d| {
        if d.running.contains(&code) || d.failed.contains(&code) {
            return false;
        }
        d.running.push(code);
        d.progress.entry(code).or_insert((0, 0));
        true
    });
    if fresh {
        spawn_local(async move {
            let _ = download(code).await;
            DOWNLOADS.with_borrow_mut(|d| d.running.retain(|c| *c != code));
        });
    }
}

/// Fetches and stores each remaining letter of `code`, saving progress after each one, and
/// marks the language downloaded when any letter stored words. Deleting the language stops it.
async fn download(code: &'static str) -> Result<(), Error> {
    let db = database().await?;
    let filled = fill(&db, code).await;
    db.close();
    filled
}

async fn fill(db: &IdbDatabase, code: &'static str) -> Result<(), Error> {
    let progress = || DOWNLOADS.with_borrow(|d| d.progress.get(code).copied());
    while let Some((next, records)) = progress().filter(|(next, _)| *next < LETTERS.len()) {
        let url = dictionary::letter_url(code, next).ok_or("No such letter.")?;
        let stored = match get_json(&url, &[], LETTER_MS, LETTER_BOUND).await {
            Ok(body) => store(db, code, &body).await.unwrap_or(0),
            Err(_) => 0,
        };
        if progress().is_none() {
            // Deleted while this letter was stored: its words go too.
            return clear(db, code).await;
        }
        DOWNLOADS.with_borrow_mut(|d| d.progress.insert(code, (next + 1, records + stored)));
        save().await?;
    }
    let Some((_, records)) = DOWNLOADS.with_borrow_mut(|d| d.progress.remove(code)) else {
        return Ok(());
    };
    if records == 0 {
        DOWNLOADS.with_borrow_mut(|d| d.failed.push(code));
    } else {
        let tx = db
            .transaction_with_str_and_mode(META, IdbTransactionMode::Readwrite)
            .map_err(err)?;
        let meta = to_js(
            &json!({"key": format!("lang-{code}"), "downloaded": true, "timestamp": js_sys::Date::now()}),
        );
        tx.object_store(META)
            .and_then(|s| s.put(&meta))
            .map_err(err)?;
        finished(&tx).await?;
    }
    save().await
}

/// Puts one letter file's records in one transaction; answers how many it stored.
async fn store(db: &IdbDatabase, code: &str, body: &[u8]) -> Result<u32, Error> {
    let (json, count) = dictionary::records(body).ok_or("Not a dictionary file.")?;
    // One parse of the whole letter, not one object built per record across the boundary.
    let text = std::str::from_utf8(&json).map_err(|_| "Not text.")?;
    let records: js_sys::Array = js_sys::JSON::parse(text).map_err(err)?.unchecked_into();
    let tx = db
        .transaction_with_str_and_mode(code, IdbTransactionMode::Readwrite)
        .map_err(err)?;
    let store = tx.object_store(code).map_err(err)?;
    for record in records.iter() {
        store.put(&record).map_err(err)?;
    }
    finished(&tx).await?;
    Ok(count as u32)
}

/// Stops `code`'s download and deletes its words and its downloaded mark.
async fn remove(code: &'static str) -> Result<(), Error> {
    DOWNLOADS.with_borrow_mut(|d| d.progress.remove(code));
    save().await?;
    let db = database().await?;
    let cleared = clear(&db, code).await;
    db.close();
    cleared
}

/// Deletes `code`'s words and its downloaded mark.
async fn clear(db: &IdbDatabase, code: &str) -> Result<(), Error> {
    let tx = db
        .transaction_with_str_sequence_and_mode(
            &js_sys::Array::of2(&code.into(), &META.into()),
            IdbTransactionMode::Readwrite,
        )
        .map_err(err)?;
    tx.object_store(code).and_then(|s| s.clear()).map_err(err)?;
    tx.object_store(META)
        .and_then(|s| s.delete(&format!("lang-{code}").into()))
        .map_err(err)?;
    finished(&tx).await
}

/// The languages marked downloaded, in offered order.
async fn downloaded() -> Vec<String> {
    let Ok(db) = database().await else {
        return Vec::new();
    };
    let all = db
        .transaction_with_str(META)
        .and_then(|tx| tx.object_store(META))
        .and_then(|store| store.get_all());
    let marks = match all {
        Ok(request) => from_js(&keys::done(&request).await.unwrap_or(JsValue::NULL)),
        Err(_) => Value::Null,
    };
    db.close();
    let marked = |code: &str| {
        let key = format!("lang-{code}");
        marks
            .as_array()
            .into_iter()
            .flatten()
            .any(|m| m["key"] == key && m["downloaded"] == true)
    };
    LANGUAGES
        .iter()
        .map(|(code, ..)| *code)
        .filter(|c| marked(c))
        .map(String::from)
        .collect()
}

/// The dictionary database, with a store for every offered language and `meta`.
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
            let params = web_sys::IdbObjectStoreParameters::new();
            params.set_key_path(&"word".into());
            for (code, ..) in LANGUAGES.iter().filter(|(code, ..)| !names.contains(code)) {
                let _ = db.create_object_store_with_optional_parameters(code, &params);
            }
            if !names.contains(META) {
                let meta = web_sys::IdbObjectStoreParameters::new();
                meta.set_key_path(&"key".into());
                let _ = db.create_object_store_with_optional_parameters(META, &meta);
            }
        }
    });
    open.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    keys::done(&open)
        .await?
        .dyn_into()
        .map_err(|_| "The dictionaries could not be opened.".into())
}

/// Waits for `tx` to commit.
async fn finished(tx: &IdbTransaction) -> Result<(), Error> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let complete = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let failed = Closure::once_into_js(move || {
            let _ = reject.call1(&JsValue::NULL, &"The dictionary could not be saved.".into());
        });
        tx.set_oncomplete(Some(complete.unchecked_ref()));
        tx.set_onerror(Some(failed.unchecked_ref()));
        tx.set_onabort(Some(failed.unchecked_ref()));
    });
    JsFuture::from(promise).await.map(|_| ()).map_err(err)
}

/// Clears the cached page summaries. It never creates the cache: when there is none the open
/// is aborted before the database exists.
#[wasm_bindgen]
pub async fn summary_cache_clear() -> Result<(), JsValue> {
    const CACHE: &str = "metldr_cache";
    const PAGES: &str = "page_cache";
    let factory: web_sys::IdbFactory = get(&js_sys::global(), "indexedDB")
        .dyn_into()
        .map_err(|_| js_error("No IndexedDB here."))?;
    let open: IdbOpenDbRequest = factory.open(CACHE).map_err(|e| js_error(err(e)))?;
    let refuse = Closure::once_into_js({
        let open = open.clone();
        move || {
            if let Some(tx) = open.transaction() {
                let _ = tx.abort();
            }
        }
    });
    open.set_onupgradeneeded(Some(refuse.unchecked_ref()));
    let Ok(db) = keys::done(&open)
        .await
        .and_then(|db| db.dyn_into::<IdbDatabase>().map_err(|_| Error::new()))
    else {
        return Ok(());
    };
    let cleared = async {
        if !db.object_store_names().contains(PAGES) {
            return Ok(());
        }
        let tx = db
            .transaction_with_str_and_mode(PAGES, IdbTransactionMode::Readwrite)
            .map_err(err)?;
        tx.object_store(PAGES)
            .and_then(|s| s.clear())
            .map_err(err)?;
        finished(&tx).await
    }
    .await;
    db.close();
    cleared.map_err(|_| js_error("The cached summaries could not be cleared."))
}
