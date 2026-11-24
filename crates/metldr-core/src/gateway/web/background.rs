//! The service worker's messages from the extension's pages, its in-page frames and its content
//! scripts, answered here in one place. Opening the side panel stays in the worker's script: it
//! must run while the click that asked for it still counts as the reader's gesture.

use super::*;
use std::cell::Cell;
use web_sys::{IdbDatabase, IdbObjectStoreParameters, IdbOpenDbRequest};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_namespace = ["chrome", "tabs"], js_name = sendMessage)]
    fn tabs_send(tab: i32, message: &JsValue, options: &JsValue) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, js_namespace = ["chrome", "tabs"], js_name = get)]
    fn tabs_get(tab: i32) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, js_namespace = ["chrome", "tabs"], js_name = query)]
    fn tabs_query(query: &JsValue) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, js_namespace = ["chrome", "permissions"], js_name = contains)]
    fn permissions_contains(permissions: &JsValue) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn console_warn(message: &str, error: &JsValue);
}

/// The cache's version and stores; its schema is exactly these (`cache_ready`).
const CACHE_VERSION: u32 = 9;
const CACHE_STORES: [(&str, &str); 5] = [
    ("email_sessions", "emailId"),
    ("email_summaries", "emailId"),
    ("email_replies", "emailId"),
    ("page_cache", "url"),
    ("rag_metadata", "sourceId"),
];

thread_local! {
    static CACHE_OPEN: Cell<bool> = const { Cell::new(false) };
    /// The last summary in line: summaries run one at a time, in the order asked.
    static SUMMARY_TAIL: RefCell<Option<js_sys::Promise>> = const { RefCell::new(None) };
}

/// A rejection's readable reason.
fn reason(error: &JsValue) -> String {
    error
        .as_string()
        .or_else(|| get(error, "message").as_string())
        .unwrap_or_else(|| "The browser refused the request.".into())
}

async fn promise(made: Result<js_sys::Promise, JsValue>) -> Result<JsValue, String> {
    JsFuture::from(made.map_err(|e| reason(&e))?).await.map_err(|e| reason(&e))
}

/// Opens the cache with its schema once per worker: stores it no longer names are deleted, no
/// store keeps an index, and missing stores are made. The core's readers never create it.
async fn cache_ready() -> Result<(), String> {
    if CACHE_OPEN.get() {
        return Ok(());
    }
    let factory: web_sys::IdbFactory = get(&js_sys::global(), "indexedDB").dyn_into().map_err(|_| "No IndexedDB here.")?;
    let open: IdbOpenDbRequest = factory.open_with_u32(page_summary::CACHE, CACHE_VERSION).map_err(|e| reason(&e))?;
    let upgrade = Closure::once_into_js({
        let open = open.clone();
        move || {
            let (Ok(result), Some(tx)) = (open.result(), open.transaction()) else { return };
            let Ok(db) = result.dyn_into::<IdbDatabase>() else { return };
            let names = db.object_store_names();
            for i in 0..names.length() {
                let Some(name) = names.get(i) else { continue };
                match CACHE_STORES.iter().any(|(wanted, _)| *wanted == name) {
                    false => {
                        let _ = db.delete_object_store(&name);
                    }
                    true => {
                        let Ok(store) = tx.object_store(&name) else { continue };
                        let indexes = store.index_names();
                        let indexes: Vec<String> = (0..indexes.length()).filter_map(|i| indexes.get(i)).collect();
                        for index in indexes {
                            let _ = store.delete_index(&index);
                        }
                    }
                }
            }
            for (name, key) in CACHE_STORES {
                if !db.object_store_names().contains(name) {
                    let parameters = IdbObjectStoreParameters::new();
                    parameters.set_key_path(&key.into());
                    let _ = db.create_object_store_with_optional_parameters(name, &parameters);
                }
            }
        }
    });
    open.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    let db: IdbDatabase = keys::done(&open).await?.dyn_into().map_err(|_| "The cache could not be opened.")?;
    db.close();
    CACHE_OPEN.set(true);
    email::prune().await;
    Ok(())
}

/// Runs `task` after every summary asked before it.
async fn in_line(task: impl std::future::Future<Output = Value>) -> Value {
    let done: Rc<RefCell<Option<js_sys::Function>>> = Rc::default();
    let ours = js_sys::Promise::new(&mut |resolve, _| *done.borrow_mut() = Some(resolve));
    let before = SUMMARY_TAIL.with_borrow_mut(|tail| tail.replace(ours));
    if let Some(before) = before {
        let _ = JsFuture::from(before).await;
    }
    let result = task.await;
    if let Some(resolve) = done.borrow_mut().take() {
        let _ = resolve.call0(&JsValue::NULL);
    }
    result
}

/// The answer to one message, or `None` when the worker's script handles that type.
#[wasm_bindgen]
pub async fn background_message(message: JsValue, sender: JsValue) -> JsValue {
    let msg = from_js(&message);
    let sender = from_js(&sender);
    let tab = sender["tab"]["id"].as_i64().map(|id| id as i32);
    let text = |key: &str| msg[key].as_str().unwrap_or_default().to_owned();
    let answer = match msg["type"].as_str().unwrap_or_default() {
        "SUMMARIZE_EMAIL" => email_summary_answer(&msg).await,
        "GET_REPLY_SUGGESTIONS" => kept_replies_answer(&text("emailId")).await,
        "GENERATE_REPLY_SUGGESTIONS" => replies_answer(&msg, tab).await,
        "EXTRACT_AND_SUMMARIZE" => match msg["tabId"].as_i64().map(|id| id as i32).or(tab) {
            Some(tab) => in_line(summarize(tab, &msg)).await,
            None => json!({"success": false, "error": "no tab"}),
        },
        "INPAGE_PULL" => return inpage_pull(&sender, tab).await,
        "WORD_LOOKUP" => word_answer(&msg).await,
        "DICTIONARY" => {
            return dictionary::dictionary_request(message).await.unwrap_or_else(|e| to_js(&json!({"error": reason(&e)})));
        }
        "STATS_RECORD" => {
            stats::stats_record(to_js(&msg["event"])).await;
            json!(true)
        }
        "STATS_SUMMARY" => return stats::stats_summary(msg["days"].as_u64().unwrap_or(7) as u32).await,
        "CLEAR_SUMMARY_CACHE" => match page_summary::summary_cache_clear().await {
            Ok(()) => json!({"ok": true}),
            Err(e) => json!({"ok": false, "error": reason(&e)}),
        },
        "GLOBAL_CHAT" => global_chat_answer(&msg).await,
        "GET_EMAIL_CACHE" => match cache_ready().await {
            Ok(()) => return email::email_cache(text("emailId")).await,
            Err(_) => json!({"cached": null, "emailCount": 0}),
        },
        _ => return JsValue::UNDEFINED,
    };
    to_js(&answer)
}

async fn email_summary_answer(msg: &Value) -> Value {
    let id = msg["emailId"].as_str().map(str::to_owned);
    let content = msg["emailContent"].as_str().unwrap_or_default().to_owned();
    let force = msg["forceRegenerate"].as_bool().unwrap_or(false);
    let summary = match cache_ready().await {
        Ok(()) => email::email_summary(content, id, to_js(&msg["metadata"]), force).await.map_err(|e| reason(&e)),
        Err(error) => Err(error),
    };
    match summary {
        Ok(summary) => json!({"summary": from_js(&summary)}),
        Err(error) => json!({
            "error": error,
            "summary": {"summary": format!("error: {error}"), "action_items": [], "dates": [], "confidence": "low"},
        }),
    }
}

async fn kept_replies_answer(id: &str) -> Value {
    if id.is_empty() {
        return json!({"success": false, "error": "no emailId"});
    }
    if let Err(error) = cache_ready().await {
        return json!({"success": false, "error": error});
    }
    match from_js(&email::email_kept_replies(id.to_owned()).await) {
        Value::Array(suggestions) if !suggestions.is_empty() => json!({"success": true, "suggestions": suggestions}),
        _ => json!({"success": false, "error": "no suggestions available"}),
    }
}

/// Reply suggestions for the email open in Gmail: kept ones unless `forceRegenerate`, else written
/// from the thread the Gmail tab reads out (or from the kept summary when it cannot).
async fn replies_answer(msg: &Value, sender_tab: Option<i32>) -> Value {
    let Some(id) = msg["emailId"].as_str().filter(|id| !id.is_empty()) else {
        return json!({"success": false, "error": "no emailId"});
    };
    if let Err(error) = cache_ready().await {
        return json!({"success": false, "error": error});
    }
    let force = msg["forceRegenerate"].as_bool().unwrap_or(false);
    if !force && let Value::Array(kept) = from_js(&email::email_kept_replies(id.to_owned()).await) {
        return json!({"success": true, "suggestions": kept});
    }
    let tab = match sender_tab {
        Some(tab) => Some(tab),
        None => promise(tabs_query(&to_js(&json!({"active": true, "lastFocusedWindow": true}))))
            .await
            .ok()
            .map(|tabs| from_js(&tabs))
            .and_then(|tabs| tabs[0]["id"].as_i64())
            .map(|id| id as i32),
    };
    let Some(tab) = tab else {
        return json!({"success": false, "error": "no active tab"});
    };
    let asked = to_js(&json!({"type": "GET_EMAIL_CONTENT", "emailId": id}));
    let read = promise(tabs_send(tab, &asked, &JsValue::UNDEFINED)).await.map(|r| from_js(&r)).unwrap_or_default();
    let (content, metadata) = match read["success"].as_bool() {
        Some(true) => (read["content"].as_str().unwrap_or_default().to_owned(), read["metadata"].clone()),
        _ => (String::new(), Value::Null),
    };
    match from_js(&email::email_replies(id.to_owned(), content, to_js(&metadata), force).await) {
        Value::Null => json!({"success": false, "error": "no valid suggestions generated"}),
        suggestions => json!({"success": true, "suggestions": suggestions}),
    }
}

/// An in-page frame's job, from the tab it sits in; the page never takes part, so it cannot
/// choose what the frame shows. The PDF tools read the tab's own address; the word popup's and
/// the Gmail reply card's jobs come from the tab's content script.
async fn inpage_pull(sender: &Value, tab: Option<i32>) -> JsValue {
    let url = sender["url"].as_str().unwrap_or_default();
    let frame = url.rfind("/inpage/").map_or("", |at| &url[at..]);
    match (tab, frame) {
        (Some(_), "/inpage/pdf.html") => to_js(&json!({
            "url": sender["tab"]["url"].as_str().unwrap_or_default(),
            "title": sender["tab"]["title"].as_str().unwrap_or_default(),
        })),
        (Some(tab), "/inpage/word.html" | "/inpage/replies.html" | "/inpage/summary.html") => {
            // Each frame's script answers only for its own frame.
            let kind = frame.trim_start_matches("/inpage/").trim_end_matches(".html");
            let ask = to_js(&json!({"type": "INPAGE_PULL", "frame": kind}));
            promise(tabs_send(tab, &ask, &to_js(&json!({"frameId": 0})))).await.unwrap_or(JsValue::NULL)
        }
        _ => JsValue::NULL,
    }
}

async fn word_answer(msg: &Value) -> Value {
    let Some(word) = msg["word"].as_str() else {
        return json!({"success": false, "error": "invalid word"});
    };
    let context = match &msg["context"] {
        Value::Null => json!({}),
        context => context.clone(),
    };
    match word::gateway_word_lookup(word.to_owned(), to_js(&context)).await {
        Ok(result) => {
            spawn_local(stats::stats_record(to_js(&json!({"kind": "lookup"}))));
            json!({"success": true, "result": from_js(&result)})
        }
        Err(error) => json!({"success": false, "error": reason(&error)}),
    }
}

async fn global_chat_answer(msg: &Value) -> Value {
    if msg["messages"].as_array().is_none_or(Vec::is_empty) {
        return json!({"ok": false, "error": "no messages provided"});
    }
    if let Err(error) = cache_ready().await {
        return json!({"ok": false, "error": error});
    }
    let mut result = from_js(&global_chat::gateway_global_chat(to_js(&msg["messages"])).await);
    if let Some(object) = result.as_object_mut() {
        object.entry("timing").or_insert(Value::Null);
    }
    result
}

/// The page tools' summary of tab `tab`: a page its content script reads out, or a PDF whose text
/// an extension page read and sent. Gated by the reader's settings, cached by address, indexed
/// for search.
async fn summarize(tab: i32, msg: &Value) -> Value {
    let force = msg["force"].as_bool().unwrap_or(false);
    let manual = force || msg["trigger"].as_str() == Some("manual");
    let page = match msg["text"].as_str() {
        Some(text) => pdf_page(tab, text).await,
        None => {
            let ask = to_js(&json!({"type": "EXTRACT_ARTICLE"}));
            match promise(tabs_send(tab, &ask, &JsValue::UNDEFINED)).await.map(|r| from_js(&r)) {
                Err(_) => return json!({"success": false, "error": unreachable(tab).await}),
                Ok(reply) if reply["success"] != true => {
                    let error = reply["error"].as_str().unwrap_or("extraction failed");
                    return json!({"success": false, "error": error});
                }
                Ok(mut reply) => reply["data"].take(),
            }
        }
    };
    if !page.is_object() {
        return json!({"success": false, "error": "extraction failed"});
    }
    if page["skip"] == true {
        // The asking extension page reads a PDF tab's file and sends its text.
        if page["reason"] == "pdf" {
            return json!({"success": false, "pdf": true});
        }
        return json!({"success": false, "skip": true, "reason": page["reason"]});
    }
    let gate = from_js(&summary_gate::summary_gate(to_js(&page), manual).await);
    match gate["action"].as_str() {
        Some("skip") => return json!({"success": false, "skip": true, "reason": gate["reason"]}),
        Some("prompt") => return json!({"success": false, "prompt": true, "reason": gate["reason"]}),
        _ => {}
    }
    if let Err(error) = cache_ready().await {
        return json!({"success": false, "error": error});
    }
    let url = page["url"].as_str().unwrap_or_default().to_owned();
    let title = page["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&url).to_owned();
    let kind = if page["contentType"] == "application/pdf" { "pdf" } else { "article" };
    let source = json!({"sourceId": url, "sourceUrl": url, "sourceType": kind, "title": title});
    if let Some(content) = page["content"].as_str().filter(|c| !url.is_empty() && !c.is_empty()) {
        spawn_local(index(search_store::search_index(content.to_owned(), to_js(&source))));
    }
    let summary = match page_summary::gateway_page_summary(to_js(&page), force).await {
        Ok(summary) => from_js(&summary),
        Err(error) => return json!({"success": false, "error": reason(&error)}),
    };
    if !url.is_empty() && summary["timing"]["cached"] != true {
        let bullets: Vec<&str> = summary["bullets"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        spawn_local(index(search_store::search_index_summary(bullets.join("\n"), to_js(&source))));
    }
    json!({"success": true, "summary": summary})
}

/// Why tab `tab`'s page did not answer: the reader withheld metldr's access to its site (the
/// panel's capture says the same), or its content script is not there yet.
async fn unreachable(tab: i32) -> &'static str {
    let url = promise(tabs_get(tab)).await.map(|t| from_js(&t)["url"].as_str().unwrap_or_default().to_owned()).unwrap_or_default();
    let site = url.split_once("://").map(|(scheme, rest)| {
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
        let host = if host.starts_with('[') { host.split_inclusive(']').next().unwrap_or(host) } else { host.split(':').next().unwrap_or(host) };
        format!("{scheme}://{host}/*")
    });
    let allowed = match site {
        Some(site) => promise(permissions_contains(&to_js(&json!({"origins": [site]})))).await.ok().and_then(|v| v.as_bool()),
        None => None,
    };
    if allowed == Some(false) {
        "Allow metldr on this site in the extension's site access first."
    } else {
        "Refresh the page, then try again."
    }
}

/// Indexing for search runs beside the answer; a failure costs only the search entry.
async fn index(task: impl std::future::Future<Output = Result<(), JsValue>>) {
    if let Err(error) = task.await {
        console_warn("metldr: search indexing failed", &error);
    }
}

/// The PDF tab `tab` shows, as a page of `text` an extension page read from its file.
async fn pdf_page(tab: i32, text: &str) -> Value {
    let tab = promise(tabs_get(tab)).await.map(|t| from_js(&t)).unwrap_or_default();
    let url = tab["url"].as_str().unwrap_or_default();
    let words = text.split_whitespace().count();
    let file = url.split(['?', '#']).next().unwrap_or_default().rsplit('/').next().unwrap_or_default();
    let named = js_sys::decode_uri_component(file).ok().and_then(|f| f.as_string()).unwrap_or_else(|| file.to_owned());
    let title = tab["title"].as_str().filter(|t| !t.is_empty()).map(str::to_owned).unwrap_or(if named.is_empty() { "PDF".into() } else { named });
    json!({
        "title": title,
        "url": url,
        "content": text,
        "wordCount": words,
        "readTime": format!("{} min", words.div_ceil(200).max(1)),
        "contentType": "application/pdf",
    })
}
