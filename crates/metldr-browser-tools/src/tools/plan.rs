use crate::{Call, Definition, Result};
use serde_json::{Map, Value, json};

pub const NAME: &str = "propose_task_plan";
pub const ORDER: u8 = 10;

pub fn definition() -> Definition {
    Definition {
        name: NAME,
        description: "Call before any effectful browser step and again when scope changes materially. Await user approval before proceeding.",
        input_schema: json!({"type":"object","properties":{"goal":{"type":"string","minLength":1,"maxLength":1024},"intended_effects":{"type":"array","maxItems":16,"items":{"type":"string","minLength":1,"maxLength":512}},"scope":{"type":"string","minLength":1,"maxLength":1024}},"required":["goal","intended_effects","scope"],"additionalProperties":false}),
    }
}

pub fn decode(object: &Map<String, Value>) -> Result<Call> {
    if object.len() != 3 {
        return Err("invalid task plan input".into());
    }
    let text = |key: &str, limit| -> Result<String> {
        Ok(object
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control))
            .ok_or("invalid task plan text")?
            .to_owned())
    };
    let effects = object
        .get("intended_effects")
        .and_then(Value::as_array)
        .filter(|v| v.len() <= 16)
        .ok_or("invalid task plan effects")?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
                .map(str::to_owned)
                .ok_or("invalid task plan effect")
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(Call::ProposeTaskPlan {
        goal: text("goal", 1024)?,
        intended_effects: effects,
        scope: text("scope", 1024)?,
    })
}
