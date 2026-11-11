use crate::{Call, Definition, Result, SETTINGS_TOOL};
use serde_json::{Map, Value, json};

pub const NAME: &str = SETTINGS_TOOL;
pub const ORDER: u8 = 9;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Read the saved provider and agent settings for this run: each stored value, its effective value and the source that supplies it. Read-only. Credentials, endpoints and file paths are never returned. An agent cannot change any setting, including security switches such as offline mode; changes belong to the user's settings view.",
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    // Any argument is a request to set a value, which agents never do.
    if object.is_empty() {
        Ok(Call::Settings)
    } else {
        Err("settings are read-only for agents".into())
    }
}
