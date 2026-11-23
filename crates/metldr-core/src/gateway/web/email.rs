//! Email summaries and reply suggestions for the Gmail tools: kept in the cache by email id,
//! written by the preferred route, indexed for search and counted in stats (see `crate::email`).

use super::*;
use crate::{email, stats as counts};
use web_sys::IdbTransactionMode;

const SUMMARIES: &str = "email_summaries";
const REPLIES: &str = "email_replies";
const SESSIONS: &str = "email_sessions";

/// The value kept for `id` in `store`.
async fn read(store: &str, id: &str) -> Option<Value> {
    let db = page_summary::cache().await?;
    let found = async {
        let store = db.transaction_with_str(store).and_then(|tx| tx.object_store(store)).ok()?;
        let record = keys::done(&store.get(&id.into()).ok()?).await.ok()?;
        Some(from_js(&record))
    };
    let record = found.await;
    db.close();
    record
}

/// Keeps `value` for `id` in `store` as `{emailId, value}`; `None` deletes it.
async fn write(store: &str, id: &str, value: Option<&Value>) {
    let Some(db) = page_summary::cache().await else { return };
    let written = async {
        let tx = db.transaction_with_str_and_mode(store, IdbTransactionMode::Readwrite).map_err(err)?;
        let files = tx.object_store(store).map_err(err)?;
        match value {
            Some(value) => files.put(&to_js(&json!({"emailId": id, "value": value}))),
            None => files.delete(&id.into()),
        }
        .map_err(err)?;
        dictionary::finished(&tx).await
    };
    let _ = written.await;
    db.close();
}

/// The model's JSON answer to `prompt` on the preferred route, and the model.
async fn answer(system: &str, prompt: String, schema: &Value) -> Result<(Value, String), Error> {
    let target = target(None, None).await?;
    let model = target.model.clone();
    let messages = [Message { role: Role::User, content: prompt }];
    crate::gateway::check_conversation(system, &messages).map_err(str::to_owned)?;
    let stop = AtomicBool::new(false);
    let (text, _) = reply(target, system, &messages, Some(schema), &stop, &mut |_| {}).await?;
    let parsed = email::parse_json(&text).ok_or("The model did not answer with JSON.")?;
    Ok((parsed, model))
}

fn source(id: &str, title: &str) -> JsValue {
    to_js(&json!({"sourceId": id, "sourceUrl": format!("email://{id}"), "sourceType": "email", "title": title}))
}

/// Replies kept for `id` that are worth sending, if any.
async fn kept_replies(id: &str) -> Option<Value> {
    let kept = read(REPLIES, id).await?;
    let sendable = |r: &&Value| r["body"].as_str().is_some_and(|b| b.trim().len() > 10);
    let replies: Vec<Value> = kept["value"].as_array()?.iter().filter(sendable).cloned().collect();
    (!replies.is_empty()).then(|| Value::Array(replies))
}

/// Writes, keeps and counts reply suggestions for `id`; an empty list when the model gave none.
async fn written_replies(id: &str, content: &str, summary: &Value, metadata: &Value) -> Vec<Value> {
    let prompt = email::reply_prompt(metadata, summary, content);
    let Ok((parsed, _)) = answer(email::REPLY_SYSTEM, prompt, &email::reply_schema()).await else {
        return Vec::new();
    };
    let replies = email::replies_from(&parsed);
    if !replies.is_empty() {
        write(REPLIES, id, Some(&json!(replies))).await;
        stats::record(&counts::Event::Replies(replies.len() as f64)).await;
    }
    replies
}

/// The summary of email `content`: `{summary, action_items, dates, key_facts, tags, domain,
/// intent, reasoning, urgency, model, time_ms, cached}`, the kept one unless `force`. With an
/// `id` the email and its summary are indexed for search, and replies are written in the
/// background when none are kept.
#[wasm_bindgen]
pub async fn email_summary(
    content: String,
    id: Option<String>,
    metadata: JsValue,
    force: bool,
) -> Result<JsValue, JsValue> {
    let started = js_sys::Date::now();
    let metadata = from_js(&metadata);
    let id = id.filter(|id| !id.is_empty());
    let subject = metadata["subject"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    if let Some(id) = id.as_deref().filter(|_| !content.is_empty()) {
        let (text, source) = (content.clone(), source(id, subject.as_deref().unwrap_or("Email Thread")));
        spawn_local(async move {
            let _ = search_store::search_index(text, source).await;
        });
    }
    if let Some(id) = id.as_deref() {
        if force {
            write(REPLIES, id, None).await;
        } else if let Some(mut kept) = read(SUMMARIES, id).await.map(|mut record| record["value"].take()).filter(Value::is_object) {
            if kept_replies(id).await.is_none() {
                let (id, content, summary, metadata) = (id.to_owned(), content.clone(), kept.clone(), metadata.clone());
                spawn_local(async move {
                    written_replies(&id, &content, &summary, &metadata).await;
                });
            }
            kept["time_ms"] = json!((js_sys::Date::now() - started).round());
            kept["cached"] = json!(true);
            return Ok(to_js(&kept));
        }
    }
    let facts = email::facts(&content);
    let prompt = email::summary_prompt(&metadata, &email::snippet(&content), &facts);
    let (parsed, model) = answer(email::SUMMARY_SYSTEM, prompt, &email::summary_schema())
        .await
        .map_err(js_error)?;
    let mut summary = email::summary_from(&parsed, &facts, &model);
    let ms = (js_sys::Date::now() - started).round();
    summary["time_ms"] = json!(ms);
    summary["cached"] = json!(false);
    let text = summary["summary"].as_str().unwrap_or_default().to_owned();
    if let Some(id) = id {
        write(SUMMARIES, &id, Some(&summary)).await;
        let actions = summary["action_items"].as_array().into_iter().flatten().filter_map(Value::as_str);
        let indexed: Vec<&str> = std::iter::once(text.as_str()).filter(|t| !t.is_empty()).chain(actions).collect();
        let indexed = indexed.join("\n");
        let source = source(&id, subject.as_deref().unwrap_or("Email"));
        let (content, kept, metadata) = (content.clone(), summary.clone(), metadata.clone());
        spawn_local(async move {
            let _ = search_store::search_index_summary(indexed, source).await;
            if kept_replies(&id).await.is_none() {
                written_replies(&id, &content, &kept, &metadata).await;
            }
        });
    }
    let event = counts::Event::Summary {
        of: counts::Read::Email,
        words_in: content.split_whitespace().count() as f64,
        words_out: text.split_whitespace().count() as f64,
        tokens_out: (text.len() as f64 / 4.0).round(),
        ms,
        model: &model,
        site: "",
    };
    stats::record(&event).await;
    Ok(to_js(&summary))
}

/// Reply suggestions for email `id`: the kept ones unless `force`, else written from `content`
/// (or the kept summary when the email could not be read). `null` when there are none.
#[wasm_bindgen]
pub async fn email_replies(id: String, content: String, metadata: JsValue, force: bool) -> JsValue {
    if force {
        write(REPLIES, &id, None).await;
    } else if let Some(kept) = kept_replies(&id).await {
        return to_js(&kept);
    }
    let summary = read(SUMMARIES, &id).await.map(|mut record| record["value"].take()).unwrap_or(Value::Null);
    let content = match content.trim() {
        "" => summary["summary"].as_str().unwrap_or_default().to_owned(),
        _ => content,
    };
    let replies = written_replies(&id, &content, &summary, &from_js(&metadata)).await;
    if replies.is_empty() { JsValue::NULL } else { to_js(&json!(replies)) }
}

/// The reply suggestions kept for email `id`, or `null`.
#[wasm_bindgen]
pub async fn email_kept_replies(id: String) -> JsValue {
    kept_replies(&id).await.map_or(JsValue::NULL, |replies| to_js(&replies))
}

/// The kept summary of email `id` and how many messages its thread had: `{cached, emailCount}`.
#[wasm_bindgen]
pub async fn email_cache(id: String) -> JsValue {
    let summary = read(SUMMARIES, &id).await.map_or(Value::Null, |mut record| record["value"].take());
    let count = read(SESSIONS, &id).await.and_then(|record| record["emailCount"].as_u64()).unwrap_or(0);
    to_js(&json!({"cached": summary, "emailCount": count}))
}

/// Keeps `summary` for email `id`, and its thread's message count when given.
#[wasm_bindgen]
pub async fn email_cache_set(id: String, summary: JsValue, email_count: Option<u32>) {
    write(SUMMARIES, &id, Some(&from_js(&summary))).await;
    if let Some(count) = email_count {
        let Some(db) = page_summary::cache().await else { return };
        let written = async {
            let tx = db.transaction_with_str_and_mode(SESSIONS, IdbTransactionMode::Readwrite).map_err(err)?;
            let record = to_js(&json!({"emailId": id, "emailCount": count}));
            tx.object_store(SESSIONS).and_then(|s| s.put(&record)).map_err(err)?;
            dictionary::finished(&tx).await
        };
        let _ = written.await;
        db.close();
    }
}
