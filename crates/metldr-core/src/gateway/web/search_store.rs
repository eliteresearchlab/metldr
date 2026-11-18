//! The search store: pages, emails and PDFs cut into chunks for search, kept in the cache under
//! the extension's schema (`rag:` entries, the saved vectors, one hash per source), and the
//! searches over them. Nothing here creates the cache.

use super::*;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};
use web_sys::{IdbKeyRange, IdbTransactionMode};

const PAGES: &str = "page_cache";
const HASHES: &str = "rag_metadata";
const VECTORS: &str = "system:vectors";
/// How long a chunk is meant to last, as the cache records it: 60 days.
const CHUNK_KEEP_MS: f64 = 60.0 * 24.0 * 3_600_000.0;
/// Chunks stored and indexed per cache write.
const BATCH: usize = 8;
const MODEL_RETRY_MS: f64 = 30_000.0;
const SAVE_DELAY_MS: i32 = 300;
/// Recent searches' slow half (expansion, dense neighbours): at most this many, this long.
const QUERIES: usize = 100;
const QUERY_KEEP_MS: f64 = 300_000.0;
const EXPAND: &str = "You are a search query optimizer. Output ONLY comma-separated search \
    terms, no explanation.\nRULES:\n1. If user asks about what they \"read\", \"saw\", \"forgot\" \
    - extract the TOPIC keywords only\n2. Ignore personal phrases like \"what was the pdf i read \
    about\" - focus on the subject\n3. Generate 8-12 diverse terms: synonyms, related concepts, \
    different phrasings\nExample: \"what was the pdf i read about islam and ummah\" → \"islam, \
    ummah, muslim, islamic, community, faith, prophetic, religion\"";

/// Search over the stored chunks: keywords (`metldr_retrieval`), the static embeddings
/// (`crate::dense`) and their fusion.
#[derive(Default)]
struct SearchIndex {
    keywords: metldr_retrieval::Index,
    dense: crate::dense::Dense,
}

impl SearchIndex {
    /// Takes the search model fetched by `search_model_fetch`; false when none is ready.
    pub fn use_search_model(&mut self) -> bool {
        model::take_search_model().is_some_and(|bytes| self.dense.load_model(bytes))
    }

    pub fn has_model(&self) -> bool {
        self.dense.has_model()
    }

    /// Embeds an already added chunk, for chunks added before the model was ready.
    pub fn embed(&mut self, id: &str, content: &str) {
        self.dense.add(id, content);
    }

    /// Keeps a saved vector blob for the chunks added next (see `Dense::restore`).
    pub fn restore_vectors(&mut self, blob: &[u8]) {
        self.dense.restore(blob);
    }

    /// Every chunk's vector, to save.
    pub fn vectors(&self) -> Vec<u8> {
        self.dense.blob()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        id: &str,
        kind: &str,
        title: &str,
        content: &str,
        source_url: &str,
        source_id: &str,
        summary: bool,
    ) {
        let chunk = metldr_retrieval::Chunk {
            id,
            kind,
            title,
            content,
            source_url,
            source_id,
            summary,
        };
        self.keywords.add(&chunk);
        self.dense.add(id, content);
    }

    pub fn remove(&mut self, id: &str) {
        self.keywords.remove(id);
        self.dense.remove(id);
    }

    pub fn has_source(&self, source: &str) -> bool {
        self.keywords.has_source(source)
    }

    /// The ids of the `limit` chunks nearest `query` by meaning, nearest first.
    pub fn dense(&mut self, query: &str, limit: u32) -> Vec<String> {
        self.dense.search(query, limit as usize)
    }

    /// Page chat context for `focused` (see `Index::context`); `page` may be empty.
    pub fn context(&self, focused: &str, dense: Vec<String>, limit: u32, page: &str) -> String {
        self.keywords.context(focused, &dense, limit as usize, page)
    }

    /// `{context, sources: [{index, title, url, type, score, snippet}]}` as JSON.
    pub fn sources(
        &self,
        question: &str,
        focused: &str,
        dense: Vec<String>,
        limit: u32,
    ) -> Option<String> {
        let sources = self
            .keywords
            .sources(question, focused, &dense, limit as usize);
        serde_json::to_string(&sources).ok()
    }
}

/// A source as the extension names it.
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    source_id: String,
    source_url: String,
    source_type: String,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Clone)]
struct Found {
    focused: String,
    dense: Vec<String>,
}

#[derive(Default)]
struct Store {
    index: RefCell<SearchIndex>,
    /// Each source's content fingerprint and how many chunks it was cut into.
    hashes: RefCell<HashMap<String, (String, usize)>>,
    indexing: RefCell<HashSet<String>>,
    queries: RefCell<VecDeque<(String, f64, Found)>>,
    model_tried: Cell<f64>,
    saving: Cell<bool>,
}

thread_local! {
    static STORE: RefCell<Option<Rc<Store>>> = const { RefCell::new(None) };
}

fn now() -> f64 {
    js_sys::Date::now()
}

async fn sleep(ms: i32) {
    let timer = js_sys::Promise::new(&mut |resolve, _| {
        if let Ok(set_timeout) = get(&js_sys::global(), "setTimeout").dyn_into::<js_sys::Function>()
        {
            let _ = set_timeout.call2(&JsValue::UNDEFINED, &resolve, &ms.into());
        }
    });
    let _ = JsFuture::from(timer).await;
}

/// The cache's content fingerprint as the extension wrote it: length, first and last 200
/// UTF-16 units.
fn fingerprint(text: &str) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    let head = String::from_utf16_lossy(&units[..units.len().min(200)]);
    let tail = String::from_utf16_lossy(&units[units.len().saturating_sub(200)..]);
    format!("{}:{head}:{tail}", units.len())
}

fn remember(index: &mut SearchIndex, entry: &Value) {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let meta = &entry["metadata"];
    index.add(
        &text(&entry["id"]),
        &text(&entry["type"]),
        &text(&meta["title"]),
        &text(&entry["content"]),
        &text(&meta["sourceUrl"]),
        &text(&meta["sourceId"]),
        meta["isSummary"] == true,
    );
}

/// Every stored chunk entry.
async fn stored() -> Vec<Value> {
    let Some(db) = page_summary::cache().await else {
        return Vec::new();
    };
    let read = async {
        let range = IdbKeyRange::bound_with_lower_open_and_upper_open(
            &"rag:".into(),
            &"rag;".into(),
            false,
            true,
        )
        .ok()?;
        let store = db
            .transaction_with_str(PAGES)
            .and_then(|tx| tx.object_store(PAGES))
            .ok()?;
        let all = keys::done(&store.get_all_with_key(&range).ok()?)
            .await
            .ok()?;
        from_js(&all).as_array().cloned()
    };
    let found = read.await.unwrap_or_default();
    db.close();
    found
}

async fn load() -> Rc<Store> {
    let store = Store::default();
    store.model_tried.set(now());
    if model::search_model_fetch().await {
        store.index.borrow_mut().use_search_model();
    }
    if let Some(db) = page_summary::cache().await {
        let read = async {
            let tx = db
                .transaction_with_str_sequence(&js_sys::Array::of2(&PAGES.into(), &HASHES.into()))
                .ok()?;
            let vectors = keys::done(&tx.object_store(PAGES).ok()?.get(&VECTORS.into()).ok()?)
                .await
                .ok()?;
            let hashes = keys::done(&tx.object_store(HASHES).ok()?.get_all().ok()?)
                .await
                .ok()?;
            Some((vectors, hashes))
        };
        if let Some((vectors, hashes)) = read.await {
            if let Ok(value) = js_sys::Reflect::get(&vectors, &"value".into())
                && let Ok(blob) = value.dyn_into::<js_sys::Uint8Array>()
            {
                store.index.borrow_mut().restore_vectors(&blob.to_vec());
            }
            for entry in from_js(&hashes).as_array().into_iter().flatten() {
                if let (Some(id), Some(hash)) =
                    (entry["sourceId"].as_str(), entry["contentHash"].as_str())
                {
                    let chunks = entry["chunkCount"].as_u64().unwrap_or(0) as usize;
                    store
                        .hashes
                        .borrow_mut()
                        .insert(id.to_owned(), (hash.to_owned(), chunks));
                }
            }
        }
        db.close();
    }
    for entry in stored().await {
        remember(&mut store.index.borrow_mut(), &entry);
    }
    Rc::new(store)
}

/// The store, loaded once; a search model that was missing is tried again now and then.
async fn store() -> Rc<Store> {
    let store = match STORE.with_borrow(Clone::clone) {
        Some(store) => store,
        None => {
            let loaded = load().await;
            // Two first calls may both load; the first to finish is kept.
            STORE.with_borrow_mut(|slot| Rc::clone(slot.get_or_insert(loaded)))
        }
    };
    if !store.index.borrow().has_model() && now() - store.model_tried.get() > MODEL_RETRY_MS {
        store.model_tried.set(now());
        let later = Rc::clone(&store);
        spawn_local(async move {
            if model::search_model_fetch().await && later.index.borrow_mut().use_search_model() {
                for entry in stored().await {
                    let text = |key: &str| entry[key].as_str().unwrap_or_default().to_owned();
                    later
                        .index
                        .borrow_mut()
                        .embed(&text("id"), &text("content"));
                }
                save_soon(&later);
            }
        });
    }
    store
}

async fn save(store: &Store) {
    let Some(db) = page_summary::cache().await else {
        return;
    };
    let blob = js_sys::Uint8Array::from(store.index.borrow().vectors().as_slice());
    let record = js_sys::Object::new();
    for (key, value) in [
        ("url", JsValue::from(VECTORS)),
        ("value", blob.into()),
        ("timestamp", now().into()),
    ] {
        let _ = js_sys::Reflect::set(&record, &key.into(), &value);
    }
    let written = async {
        let tx = db
            .transaction_with_str_and_mode(PAGES, IdbTransactionMode::Readwrite)
            .map_err(err)?;
        tx.object_store(PAGES)
            .and_then(|s| s.put(&record))
            .map_err(err)?;
        dictionary::finished(&tx).await
    };
    let _ = written.await;
    db.close();
}

/// Saves the vectors once after a burst of changes.
fn save_soon(store: &Rc<Store>) {
    if store.saving.replace(true) {
        return;
    }
    let store = Rc::clone(store);
    spawn_local(async move {
        sleep(SAVE_DELAY_MS).await;
        store.saving.set(false);
        save(&store).await;
    });
}

/// Writes `entries` and their source's hash in one cache transaction.
async fn keep(entries: &[Value], hash: Option<(&str, &str, usize)>) {
    let Some(db) = page_summary::cache().await else {
        return;
    };
    keep_in(&db, entries, hash, &[]).await;
    db.close();
}

/// [`keep`] on an open cache, so one indexing run opens it once.
/// Chunk ids in `removed` are deleted in the same transaction.
async fn keep_in(
    db: &web_sys::IdbDatabase,
    entries: &[Value],
    hash: Option<(&str, &str, usize)>,
    removed: &[String],
) {
    let written = async {
        let names = js_sys::Array::of2(&PAGES.into(), &HASHES.into());
        let tx = db
            .transaction_with_str_sequence_and_mode(&names, IdbTransactionMode::Readwrite)
            .map_err(err)?;
        let pages = tx.object_store(PAGES).map_err(err)?;
        for entry in entries {
            pages.put(&to_js(entry)).map_err(err)?;
        }
        for id in removed {
            pages.delete(&format!("rag:{id}").into()).map_err(err)?;
        }
        if let Some((source, hash, chunks)) = hash {
            let record = json!({"sourceId": source, "contentHash": hash, "chunkCount": chunks, "timestamp": now()});
            tx.object_store(HASHES)
                .and_then(|s| s.put(&to_js(&record)))
                .map_err(err)?;
        }
        dictionary::finished(&tx).await
    };
    let _ = written.await;
}

fn entry(id: String, source: &Source, content: &str, meta: Value) -> Value {
    json!({"url": format!("rag:{id}"), "id": id, "type": source.source_type, "content": content,
        "metadata": meta, "timestamp": now(), "ttl": CHUNK_KEEP_MS})
}

fn report(progress: &Option<js_sys::Function>, percent: u32) {
    if let Some(progress) = progress {
        let _ = progress.call1(&JsValue::UNDEFINED, &percent.into());
    }
}

/// Indexes `text` from `source` unless the same text is indexed already; `progress` hears 0 to
/// 100. A second call for a source being indexed waits for the first.
async fn index(
    store: &Rc<Store>,
    text: &str,
    source: &Source,
    progress: &Option<js_sys::Function>,
) {
    let id = &source.source_id;
    if store.indexing.borrow().contains(id) {
        while store.indexing.borrow().contains(id) {
            sleep(100).await;
        }
        return;
    }
    store.indexing.borrow_mut().insert(id.clone());
    report(progress, 5);
    let hash = fingerprint(text);
    let known = store.hashes.borrow().get(id).cloned();
    let before = known.as_ref().map_or(0, |(_, chunks)| *chunks);
    let present = store.index.borrow().has_source(id);
    let done = match known {
        Some((known, _)) if known == hash && present => true,
        None if present => {
            store.hashes.borrow_mut().insert(id.clone(), (hash.clone(), 0));
            keep(&[], Some((id, &hash, 0))).await;
            true
        }
        _ => false,
    };
    if !done {
        report(progress, 10);
        let kind = match source.source_type.as_str() {
            "email" => crate::ContentType::Email,
            "pdf" => crate::ContentType::Pdf,
            _ => crate::ContentType::Article,
        };
        let chunks = crate::chunk_for_embedding(text, kind);
        let total = chunks.len();
        report(progress, 20);
        let db = page_summary::cache().await;
        for (batch, part) in chunks.chunks(BATCH).enumerate() {
            let entries: Vec<Value> = part
                .iter()
                .map(|chunk| {
                    let meta = json!({"sourceId": id, "sourceUrl": source.source_url,
                        "title": source.title.as_deref().unwrap_or_default(),
                        "chunkIndex": chunk.index, "totalChunks": total});
                    entry(
                        format!("{}:{id}:chunk:{}", source.source_type, chunk.index),
                        source,
                        &chunk.text,
                        meta,
                    )
                })
                .collect();
            if let Some(db) = &db {
                keep_in(db, &entries, None, &[]).await;
            }
            for entry in &entries {
                remember(&mut store.index.borrow_mut(), entry);
            }
            let share = (batch + 1) * BATCH * 70 / total.max(1);
            report(progress, 20 + share.min(70) as u32);
        }
        if total > 0 {
            // A page that now cuts into fewer chunks drops the ones past its new end.
            let chunk_id = |n| format!("{}:{id}:chunk:{n}", source.source_type);
            let stale: Vec<String> = (total..before).map(chunk_id).collect();
            for gone in &stale {
                store.index.borrow_mut().remove(gone);
            }
            store.hashes.borrow_mut().insert(id.clone(), (hash.clone(), total));
            if let Some(db) = &db {
                keep_in(db, &[], Some((id, &hash, total)), &stale).await;
            }
            store.queries.borrow_mut().clear();
        }
        if let Some(db) = db {
            db.close();
        }
        if total > 0 {
            save(store).await;
        }
    }
    store.indexing.borrow_mut().remove(id);
    report(progress, 100);
}

fn source(value: JsValue) -> Result<Source, JsValue> {
    serde_json::from_value(from_js(&value)).map_err(|_| js_error("That source is not valid."))
}

/// Indexes `text` from `source` `{sourceId, sourceUrl, sourceType, title?}` for search.
#[wasm_bindgen]
pub async fn search_index(
    text: String,
    source_of: JsValue,
    progress: Option<js_sys::Function>,
) -> Result<(), JsValue> {
    let source = source(source_of)?;
    index(&store().await, &text, &source, &progress).await;
    Ok(())
}

/// Indexes a source's summary once, as its own searchable entry.
#[wasm_bindgen]
pub async fn search_index_summary(text: String, source_of: JsValue) -> Result<(), JsValue> {
    let mut source = source(source_of)?;
    if text.trim().is_empty() {
        return Ok(());
    }
    let store = store().await;
    let id = format!("{}:summary", source.source_id);
    if store.hashes.borrow().contains_key(&id) {
        return Ok(());
    }
    let title = format!(
        "Summary: {}",
        source
            .title
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(&source.source_url)
    );
    let meta = json!({"sourceId": id, "sourceUrl": source.source_url, "sourceType": source.source_type,
        "title": title, "isSummary": true});
    source.source_id = id.clone();
    let summary = entry(format!("{}:{id}", source.source_type), &source, &text, meta);
    let hash = fingerprint(&text);
    store.hashes.borrow_mut().insert(id.clone(), (hash.clone(), 1));
    keep(std::slice::from_ref(&summary), Some((&id, &hash, 1))).await;
    remember(&mut store.index.borrow_mut(), &summary);
    save_soon(&store);
    Ok(())
}

/// Whether anything from `source` is indexed.
#[wasm_bindgen]
pub async fn search_has(source: String) -> bool {
    store().await.index.borrow().has_source(&source)
}

/// Whether `source` is being indexed now.
#[wasm_bindgen]
pub fn search_is_indexing(source: &str) -> bool {
    STORE.with_borrow(|store| {
        store
            .as_ref()
            .is_some_and(|store| store.indexing.borrow().contains(source))
    })
}

/// Indexes `text` unless `source` is indexed: `already-indexed`, `was-indexing` or `indexed-now`.
#[wasm_bindgen]
pub async fn search_ensure(
    text: String,
    source_of: JsValue,
    progress: Option<js_sys::Function>,
) -> Result<String, JsValue> {
    let source = source(source_of)?;
    let store = store().await;
    if store.index.borrow().has_source(&source.source_id) {
        return Ok("already-indexed".into());
    }
    let waited = store.indexing.borrow().contains(&source.source_id);
    index(&store, &text, &source, &progress).await;
    Ok(if waited {
        "was-indexing"
    } else {
        "indexed-now"
    }
    .into())
}

/// The slow half of a search for `query`: the model's expansion, then the dense neighbours of
/// `5 × candidates` hits; kept a while for repeats.
async fn found(store: &Store, query: &str, candidates: u32) -> Found {
    let key = format!("{query}:{candidates}");
    let fresh = |at: f64| now() - at < QUERY_KEEP_MS;
    if let Some((_, _, found)) = store
        .queries
        .borrow()
        .iter()
        .find(|(k, at, _)| *k == key && fresh(*at))
    {
        return found.clone();
    }
    let expanded = match target(None, None).await {
        Ok(target) => ask(target, EXPAND, format!("Query: \"{query}\""))
            .await
            .ok(),
        Err(_) => None,
    };
    let terms = expanded.map(|text| metldr_retrieval::query::suggested_terms(&text).join(" "));
    let expanded = match terms.filter(|terms| !terms.is_empty()) {
        Some(terms) => format!("{query} {terms}"),
        None => metldr_retrieval::query::expand_locally(query),
    };
    let focused = metldr_retrieval::query::focus(&expanded);
    let dense = store.index.borrow_mut().dense(&focused, candidates * 5);
    let found = Found { focused, dense };
    let mut queries = store.queries.borrow_mut();
    queries.retain(|(k, at, _)| *k != key && fresh(*at));
    if queries.len() == QUERIES {
        queries.pop_front();
    }
    queries.push_back((key, now(), found.clone()));
    found
}

/// Page chat context: the best chunks for `query`, only from `page` when it is not empty.
#[wasm_bindgen]
pub async fn search_context(query: String, limit: u32, page: String) -> String {
    let store = store().await;
    let Found { focused, dense } = found(&store, &query, limit * 4).await;
    store.index.borrow().context(&focused, dense, limit, &page)
}

/// Global search: `{context, sources}`, one chunk per page, scored up by the question's words.
#[wasm_bindgen]
pub async fn search_sources(query: String, limit: u32) -> JsValue {
    to_js(&sources(&query, limit).await)
}

/// `{context, sources}` for `query`, as [`search_sources`] gives it.
pub(super) async fn sources(query: &str, limit: u32) -> Value {
    let store = store().await;
    let Found { focused, dense } = found(&store, query, limit * 4).await;
    let sources = store.index.borrow().sources(query, &focused, dense, limit);
    let sources = sources.and_then(|json| serde_json::from_str(&json).ok());
    sources.unwrap_or_else(|| json!({"context": "", "sources": []}))
}
