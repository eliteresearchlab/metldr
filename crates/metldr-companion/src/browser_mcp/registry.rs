//! The MCP `tools/list` payload.
//!
//! The tool set lives in `metldr_browser_tools::tools`. This module only turns
//! each registered tool into its wire form, so a new tool is advertised by
//! registering it there and nothing here changes.

use metldr_browser_tools::tools::{Annotations, TOOLS, Tool};
use serde_json::{Value, json};

/// Every registered tool in advertised order.
pub(super) fn list() -> Value {
    json!({"tools": TOOLS.iter().map(entry).collect::<Vec<_>>()})
}

fn entry(tool: &Tool) -> Value {
    let definition = (tool.definition)();
    let Annotations {
        read_only,
        destructive,
        idempotent,
        open_world,
    } = tool.annotations;
    json!({
        "name": definition.name,
        "description": definition.description,
        "inputSchema": definition.input_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "idempotentHint": idempotent,
            "openWorldHint": open_world,
        }
    })
}
