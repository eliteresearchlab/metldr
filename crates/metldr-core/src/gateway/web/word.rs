//! The word popup's lookup in the service worker: the free dictionary API, then the model the
//! reader chose for page tools, then the dictionaries they downloaded.

use super::*;
use crate::word::{self, Context, Lookup};
use web_sys::{IdbDatabase, IdbOpenDbRequest};

const API_MS: u32 = 8_000;
/// The database the side panel downloads dictionaries into, one store per language.
const DICTIONARIES: &str = "metldr-dictionary";

/// `word` and its page context to `{definitions, synonyms, language, source}`.
#[wasm_bindgen]
pub async fn gateway_word_lookup(word: String, context: JsValue) -> Result<JsValue, JsValue> {
    let word = word::normalize(&word);
    if word.is_empty() || word.len() > 100 {
        return Err(js_error("That word cannot be looked up."));
    }
    let context: Context = serde_json::from_value(from_js(&context)).unwrap_or_default();
    let stored = from_js(
        &JsFuture::from(storage_get(&to_js(&json!(["selectedLanguages"]))).map_err(js_error_from)?)
            .await
            .unwrap_or(JsValue::NULL),
    );
    let detected = if context.page_lang.is_empty() {
        gateway_detect_language(word::detection_text(&word, &context))
            .await
            .unwrap_or_else(|_| "en".into())
    } else {
        context.page_lang.clone()
    };
    let languages = word::priority(&detected, &word::chosen(&stored["selectedLanguages"]));
    let found = match api(&word, &languages).await {
        Some(found) => Some(found),
        None => match model(&word, &languages[0], &context).await {
            Some(found) => Some(found),
            None => local(&word, &languages).await,
        },
    };
    found
        .map(|found| to_js(&json!(found)))
        .ok_or_else(|| js_error("That word was not found."))
}

fn js_error_from(error: JsValue) -> JsValue {
    js_error(err(error))
}

async fn api(word: &str, languages: &[String]) -> Option<Lookup> {
    let encoded = String::from(js_sys::encode_uri_component(word));
    for language in languages {
        let url = format!("https://api.dictionaryapi.dev/api/v2/entries/{language}/{encoded}");
        if let Ok(body) = get_json(&url, &[], API_MS).await
            && let Some(found) = word::from_api(&body, language)
        {
            return Some(found);
        }
    }
    None
}

async fn model(word: &str, language: &str, context: &Context) -> Option<Lookup> {
    let target = target(None, None).await.ok()?;
    let route = target.route.id;
    let messages = [Message {
        role: Role::User,
        content: word::prompt(word, language, context),
    }];
    let stop = AtomicBool::new(false);
    let (text, _) = reply(
        target,
        word::SYSTEM,
        &messages,
        Some(&word::schema()),
        &stop,
        &mut |_| {},
    )
    .await
    .ok()?;
    Some(word::from_model(&text, language, route))
}

/// Reads the downloaded dictionaries in place. It never creates the database: when none was
/// downloaded the open is aborted before the database exists.
async fn local(word: &str, languages: &[String]) -> Option<Lookup> {
    let factory: web_sys::IdbFactory = get(&js_sys::global(), "indexedDB").dyn_into().ok()?;
    let open: IdbOpenDbRequest = factory.open(DICTIONARIES).ok()?;
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
    let names = db.object_store_names();
    for language in languages.iter().filter(|code| names.contains(code)) {
        let store = db
            .transaction_with_str(language)
            .and_then(|tx| tx.object_store(language))
            .ok()?;
        let record = keys::done(&store.get(&word.into()).ok()?).await.ok()?;
        if let Some(found) = word::from_local(&from_js(&record), language) {
            db.close();
            return Some(found);
        }
    }
    db.close();
    None
}
