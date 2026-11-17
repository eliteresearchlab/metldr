use crate::{Call, Definition, Result, SECRET_TOOL, SecretKind, valid_ref};
use serde_json::{Map, Value, json};

pub const NAME: &str = SECRET_TOOL;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Ask the user to type a password or one-time code into a secret field from the latest snapshot. You never see or send the secret. The user consents to each entry and types it in MetLDR. The result is an opaque reference.",
        input_schema: json!({
            "type":"object",
            "properties":{
                "ref":{"type":"string","minLength":1,"maxLength":16,"pattern":"^[a-z0-9]+$"},
                "kind":{"type":"string","enum":["password","one_time_code"]}
            },
            "required":["ref","kind"],
            "additionalProperties":false
        }),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 2 || !object.contains_key("kind") {
        return Err("invalid enter_secret input".into());
    }
    let reference = object
        .get("ref")
        .and_then(Value::as_str)
        .filter(|value| valid_ref(value))
        .ok_or("invalid secret field ref")?
        .to_owned();
    let kind = match object["kind"].as_str() {
        Some("password") => SecretKind::Password,
        Some("one_time_code") => SecretKind::OneTimeCode,
        _ => return Err("secret kind must be password or one_time_code".into()),
    };
    Ok(Call::EnterSecret { reference, kind })
}
