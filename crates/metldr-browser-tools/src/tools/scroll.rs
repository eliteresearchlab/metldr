use super::schema;
use crate::{Call, Definition, Result, SCROLL_TOOL, ScrollDirection};
use serde_json::{Map, Value, json};

pub const NAME: &str = SCROLL_TOOL;
pub const ORDER: u8 = 5;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Propose scrolling an observed page or region. The user must approve. Observe again to read the result or use fresh handles. Positions report local observations only.",
        input_schema: json!({
            "type":"object",
            "properties":{
                "handle":schema::handle(),
                "direction":{"type":"string","enum":["up","down","left","right"]},
                "pixels":{"type":"integer","minimum":1,"maximum":2000}
            },
            "required":["handle","direction","pixels"],
            "additionalProperties":false
        }),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 3 {
        return Err("invalid scroll_page input".into());
    }
    let handle = schema::handle_argument(object.get("handle"), "invalid browser scroll handle")?;
    let direction = match object.get("direction").and_then(Value::as_str) {
        Some("up") => ScrollDirection::Up,
        Some("down") => ScrollDirection::Down,
        Some("left") => ScrollDirection::Left,
        Some("right") => ScrollDirection::Right,
        _ => return Err("invalid browser scroll direction".into()),
    };
    let pixels = object
        .get("pixels")
        .and_then(Value::as_u64)
        .filter(|pixels| (1..=2000).contains(pixels))
        .ok_or("invalid browser scroll distance")?;
    Ok(Call::Scroll {
        handle,
        direction,
        pixels: u16::try_from(pixels)?,
    })
}
