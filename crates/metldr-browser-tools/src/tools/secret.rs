use super::schema;
use crate::{Call, Definition, Result, SECRET_TOOL, SecretKind};
use serde_json::{Map, Value, json};

pub const NAME: &str = SECRET_TOOL;
pub const ORDER: u8 = 12;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Ask the user to type a password or one-time code into an observed secret field. You never see or send the secret. The user consents to each entry and types it in MetLDR. The result is an opaque reference.",
        input_schema: json!({
            "type":"object",
            "properties":{
                "handle":schema::handle(),
                "kind":{"type":"string","enum":["password","one_time_code"]}
            },
            "required":["handle","kind"],
            "additionalProperties":false
        }),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 2 || !object.contains_key("handle") || !object.contains_key("kind") {
        return Err("invalid enter_secret input".into());
    }
    let handle = schema::handle_argument(object.get("handle"), "invalid secret field handle")?;
    let kind = match object["kind"].as_str() {
        Some("password") => SecretKind::Password,
        Some("one_time_code") => SecretKind::OneTimeCode,
        _ => return Err("secret kind must be password or one_time_code".into()),
    };
    Ok(Call::EnterSecret { handle, kind })
}
