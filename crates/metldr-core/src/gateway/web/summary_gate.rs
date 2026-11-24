//! The page summary settings in `chrome.storage.local.summaryPrefs`, and the decision made from
//! them when a page is extracted.

use super::*;
use crate::decide::summary::{self as summary_gate, Prefs};

const KEY: &str = "summaryPrefs";

async fn stored() -> Prefs {
    let stored = match storage_get(&to_js(&json!([KEY]))) {
        Ok(read) => from_js(&JsFuture::from(read).await.unwrap_or(JsValue::NULL)),
        Err(_) => Value::Null,
    };
    Prefs::from_stored(&stored[KEY])
}

/// `{action, reason}` for the extracted page: summarize now, ask, wait or skip. `manual` is a
/// summary the reader asked for.
pub(super) async fn summary_gate(page: JsValue, manual: bool) -> JsValue {
    let decision = summary_gate::classify(&from_js(&page), &stored().await, manual);
    to_js(&json!(decision))
}
