//! The page summary settings in `chrome.storage.local.summaryPrefs`, and the decision made from
//! them when a page is extracted.

use super::*;
use crate::decide::summary::{self as summary_gate, Form, Prefs};

const KEY: &str = "summaryPrefs";

async fn stored() -> Prefs {
    let stored = match storage_get(&to_js(&json!([KEY]))) {
        Ok(read) => from_js(&JsFuture::from(read).await.unwrap_or(JsValue::NULL)),
        Err(_) => Value::Null,
    };
    Prefs::from_stored(&stored[KEY])
}

/// The settings with their defaults filled in.
#[wasm_bindgen]
pub async fn summary_prefs_load() -> JsValue {
    to_js(&json!(stored().await))
}

/// Saves the settings form `{mode, allowlist, denylist, minAutoWords}`, where the lists are
/// typed one per line or comma separated, and answers the settings as stored.
#[wasm_bindgen]
pub async fn summary_prefs_save(form: JsValue) -> Result<JsValue, JsValue> {
    let form: Form = serde_json::from_value(from_js(&form))
        .map_err(|_| js_error("Those summary settings are not valid."))?;
    let prefs = json!(form.into_prefs());
    let write = storage_set(&to_js(&json!({KEY: prefs}))).map_err(|e| js_error(err(e)))?;
    JsFuture::from(write).await.map_err(|e| js_error(err(e)))?;
    Ok(to_js(&prefs))
}

/// `{action, reason}` for the extracted page: summarize now, ask, wait or skip. `manual` is a
/// summary the reader asked for.
#[wasm_bindgen]
pub async fn summary_gate(page: JsValue, manual: bool) -> JsValue {
    let decision = summary_gate::classify(&from_js(&page), &stored().await, manual);
    to_js(&json!(decision))
}
