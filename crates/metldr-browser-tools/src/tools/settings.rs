use crate::{Call, Definition, Result, SETTINGS_TOOL};
use serde_json::{Map, Value, json};

pub const NAME: &str = SETTINGS_TOOL;
pub const ORDER: u8 = 9;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Read provider and agent setting states, revisions, sources, runtime availability and Pi offline mode for this run. Read-only. Free-form values, credentials, endpoints and file paths are never returned. An agent cannot change any setting; changes belong to the user's settings view.",
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
