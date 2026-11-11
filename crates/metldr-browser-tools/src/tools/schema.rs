use crate::{Result, valid_handle};
use serde_json::Value;

pub(super) fn handle() -> Value {
    serde_json::json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9_-]+$"})
}

pub(super) fn handle_argument(value: Option<&Value>, error: &'static str) -> Result<String> {
    Ok(value
        .and_then(Value::as_str)
        .filter(|value| valid_handle(value))
        .ok_or(error)?
        .to_owned())
}
