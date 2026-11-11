use super::schema;
use crate::{CONTROL_TOOL, Call, Definition, Result};
use serde_json::{Map, Value, json};

pub const NAME: &str = CONTROL_TOOL;
pub const ORDER: u8 = 6;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Propose a checked state for an observed native checkbox or radio. The user must approve before dispatch. Re-observe before proposing and use only handles from the latest observation. Already matching state is a no-op. Effects are local to the control; radio controls cannot be unchecked directly.",
        input_schema: json!({
            "type":"object",
            "properties":{
                "handle":schema::handle(),
                "checked":{"type":"boolean"}
            },
            "required":["handle","checked"],
            "additionalProperties":false
        }),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 2 || !object.contains_key("handle") || !object.contains_key("checked") {
        return Err("invalid set_native_control input".into());
    }
    let handle = schema::handle_argument(object.get("handle"), "invalid native control handle")?;
    let checked = object["checked"]
        .as_bool()
        .ok_or("invalid native control checked state")?;
    Ok(Call::SetNativeControl { handle, checked })
}
