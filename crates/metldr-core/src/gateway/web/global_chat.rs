//! Global search chat: a question answered from everything the reader has read and saved,
//! with the numbered sources it cites.

use super::*;
use web_sys::IdbTransactionMode;

const PAGES: &str = "page_cache";
const EMAILS: &str = "email_summaries";
/// Sources a question is answered from.
const SOURCES: u32 = 8;
/// Earlier messages the model sees beside the question.
const HISTORY: usize = 6;
const RULES: &str = "RULES:\n\
- Extract and state the key facts from each source (dates, amounts, names, order IDs, etc.)\n\
- For each relevant source, write 1-2 sentences with the specific details found\n\
- Cite once at end of each item: [1], [2], etc.\n\
- Don't say \"details not available\" - if info exists in the source, extract it\n\
- Be direct and factual, no fluff";
const NOTHING: &str = "I don't have any indexed content to search. Try summarizing some emails, \
    articles, or PDFs first.";

/// The saved summary of each source, `[n] SUMMARY: ...`, read in one cache transaction.
async fn summaries(sources: &[Value]) -> Vec<String> {
    let Some(db) = page_summary::cache().await else {
        return Vec::new();
    };
    let read = async {
        let names = js_sys::Array::of2(&PAGES.into(), &EMAILS.into());
        let tx = db
            .transaction_with_str_sequence_and_mode(&names, IdbTransactionMode::Readonly)
            .ok()?;
        let (pages, emails) = (tx.object_store(PAGES).ok()?, tx.object_store(EMAILS).ok()?);
        let mut found = Vec::new();
        for source in sources {
            let url = source["url"].as_str().unwrap_or_default();
            let (store, key) = match url.strip_prefix("email://") {
                Some(email) => (&emails, email.rsplit('/').next().unwrap_or_default()),
                None => (&pages, url),
            };
            if key.is_empty() {
                continue;
            }
            let Ok(request) = store.get(&key.into()) else {
                continue;
            };
            let record = keys::done(&request)
                .await
                .map(|record| from_js(&record))
                .unwrap_or_default();
            let summary = if url.starts_with("email://") {
                &record["value"]
            } else {
                &record["summary"]
            };
            let points = ["bullets", "keyPoints"]
                .iter()
                .find_map(|key| summary[*key].as_array().filter(|p| !p.is_empty()));
            if let Some(points) = points {
                let joined: Vec<&str> = points.iter().filter_map(Value::as_str).collect();
                found.push(format!(
                    "[{}] SUMMARY: {}",
                    source["index"],
                    joined.join(". ")
                ));
            }
        }
        Some(found)
    };
    let found = read.await.unwrap_or_default();
    db.close();
    found
}

/// Answers the last user message of `messages` `[{role, content}]` from the search store:
/// `{ok, content, sources, timing: {total, model}}`, or `{ok: false, error}`.
#[wasm_bindgen]
pub async fn gateway_global_chat(messages: JsValue) -> JsValue {
    let started = js_sys::Date::now();
    let elapsed = || (js_sys::Date::now() - started).round();
    let messages = from_js(&messages);
    let messages = messages.as_array().map(Vec::as_slice).unwrap_or_default();
    let is_user = |m: &&Value| m["role"] == "user";
    let Some(question) = messages
        .iter()
        .rev()
        .find(is_user)
        .and_then(|m| m["content"].as_str())
    else {
        return to_js(&json!({"ok": false, "error": "no user message"}));
    };
    let found = search_store::sources(question, SOURCES).await;
    let context = found["context"].as_str().unwrap_or_default();
    let sources = found["sources"].as_array().cloned().unwrap_or_default();
    if context.is_empty() {
        return to_js(&json!({"ok": true, "content": NOTHING, "sources": [],
            "timing": {"total": elapsed(), "model": "none"}}));
    }
    let summaries = summaries(&sources).await;
    let context = match summaries.is_empty() {
        true => context.to_owned(),
        false => format!(
            "SUMMARIES:\n{}\n\nDETAILED SOURCES:\n{context}",
            summaries.join("\n")
        ),
    };
    let system = format!(
        "You are metldr, answering questions from the user's saved emails, \
        articles, and documents.\n\nSOURCES:\n{context}\n\n{RULES}"
    );
    let earlier = messages
        .iter()
        .filter(|m| m["content"].as_str() != Some(question));
    let earlier: Vec<&Value> = earlier.collect();
    let mut turns: Vec<Message> = earlier[earlier.len().saturating_sub(HISTORY)..]
        .iter()
        .filter_map(|m| {
            serde_json::from_value(json!({"role": m["role"], "content": m["content"]})).ok()
        })
        .collect();
    turns.push(Message {
        role: Role::User,
        content: question.to_owned(),
    });
    let answered = async {
        crate::gateway::check_conversation(&system, &turns).map_err(str::to_owned)?;
        let target = target(None, None).await?;
        let model = target.model.clone();
        let stop = AtomicBool::new(false);
        let (text, _) = reply(target, &system, &turns, None, &stop, &mut |_| {}).await?;
        Ok::<_, Error>((text, model))
    };
    match answered.await {
        Ok((text, model)) => to_js(&json!({"ok": true, "content": text, "sources": sources,
            "timing": {"total": elapsed(), "model": model}})),
        Err(error) => to_js(&json!({"ok": false, "error": error})),
    }
}
