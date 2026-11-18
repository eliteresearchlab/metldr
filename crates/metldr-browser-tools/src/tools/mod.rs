//! One module per browser tool.
//!
//! Each module owns its name, its position in the advertised order, its JSON
//! schema, its safety hints and its argument decoding. `TOOLS` is the only
//! list: adding a tool means adding a module and one entry here, and every
//! caller (provider request builders, the MCP server, the installed agents)
//! picks it up without editing their own dispatch.

pub mod act;
pub mod logs;
pub mod navigate;
pub mod network;
pub mod plan;
pub mod secret;
pub mod settings;
pub mod snapshot;
pub mod status;

use crate::{Call, Definition, Result};
use serde_json::{Map, Value};

/// Advisory safety hints advertised with a tool.
///
/// A hint never grants anything. The companion's grant check stays the only
/// authority over what an approved call may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Annotations {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

/// One registered tool.
pub struct Tool {
    pub name: &'static str,
    /// Position in the advertised order. Lower comes first.
    pub order: u8,
    pub annotations: Annotations,
    pub definition: fn() -> Definition,
    pub decode: fn(&Map<String, Value>) -> Result<Call>,
}

const WRITE: Annotations = Annotations {
    read_only: false,
    destructive: true,
    idempotent: false,
    open_world: true,
};

const LOCAL_WRITE: Annotations = Annotations {
    read_only: false,
    destructive: false,
    idempotent: false,
    open_world: true,
};

const READ: Annotations = Annotations {
    read_only: true,
    destructive: false,
    idempotent: true,
    open_world: true,
};

/// The one tool list: each row is a tool's position, name, hints, definition and decoder. It
/// makes `TOOLS` and a decoder lookup of its own, so a caller that only decodes tool calls
/// (an answer that must not call tools) links no tool's definition.
macro_rules! tools {
    ($($order:literal $name:expr, $hints:ident, $definition:path, $decode:path;)*) => {
        /// Every browser tool, in advertised order.
        pub static TOOLS: [Tool; 16] = [$(Tool {
            name: $name,
            order: $order,
            annotations: $hints,
            definition: $definition,
            decode: $decode,
        },)*];

        /// The decoder of the tool named `name`.
        pub(crate) fn decoder(name: &str) -> Option<fn(&Map<String, Value>) -> Result<Call>> {
            $(if name == $name {
                return Some($decode);
            })*
            None
        }
    };
}

tools! {
    1 snapshot::NAME, READ, snapshot::definition, snapshot::decode;
    2 navigate::NAME, WRITE, navigate::definition, navigate::decode;
    3 "click", WRITE, act::click_definition, act::click_decode;
    4 "fill", WRITE, act::fill_definition, act::fill_decode;
    5 "type", WRITE, act::type_definition, act::type_decode;
    6 "press", WRITE, act::press_definition, act::press_decode;
    7 "select", WRITE, act::select_definition, act::select_decode;
    8 "hover", LOCAL_WRITE, act::hover_definition, act::hover_decode;
    9 "drag", WRITE, act::drag_definition, act::drag_decode;
    10 "scroll", LOCAL_WRITE, act::scroll_definition, act::scroll_decode;
    11 logs::NAME, READ, logs::definition, logs::decode;
    12 status::NAME, READ, status::definition, status::decode;
    13 settings::NAME, READ, settings::definition, settings::decode;
    14 plan::NAME, LOCAL_WRITE, plan::definition, plan::decode;
    15 network::NAME, READ, network::definition, network::decode;
    16 secret::NAME, WRITE, secret::definition, secret::decode;
}

/// Every definition in advertised order.
pub fn definitions() -> [Definition; 16] {
    TOOLS.each_ref().map(|tool| (tool.definition)())
}
