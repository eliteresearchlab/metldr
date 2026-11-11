use super::schema;
use crate::{Call, Definition, Result, SELECT_TOOL};
use serde_json::{Map, Value, json};

pub const NAME: &str = SELECT_TOOL;
pub const ORDER: u8 = 4;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Propose selecting an observed option. The user must approve before dispatch.",
        input_schema: json!({
            "type":"object",
            "properties":{
                "handle":schema::handle(),
                "option":schema::handle()
            },
            "required":["handle","option"],
            "additionalProperties":false
        }),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 2 || !object.contains_key("handle") || !object.contains_key("option") {
        return Err("invalid select_option input".into());
    }
    let handle = schema::handle_argument(object.get("handle"), "invalid browser select handle")?;
    let option = schema::handle_argument(object.get("option"), "invalid browser option handle")?;
    Ok(Call::SelectOption { handle, option })
}
