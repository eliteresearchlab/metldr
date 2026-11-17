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

const LOCAL_READ: Annotations = Annotations {
    read_only: true,
    destructive: false,
    idempotent: true,
    open_world: true,
};

/// Every browser tool, in advertised order.
pub static TOOLS: [Tool; 16] = [
    Tool {
        name: snapshot::NAME,
        order: 1,
        annotations: Annotations {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: true,
        },
        definition: snapshot::definition,
        decode: snapshot::decode,
    },
    Tool {
        name: navigate::NAME,
        order: 2,
        annotations: WRITE,
        definition: navigate::definition,
        decode: navigate::decode,
    },
    Tool {
        name: "click",
        order: 3,
        annotations: WRITE,
        definition: act::click_definition,
        decode: act::click_decode,
    },
    Tool {
        name: "fill",
        order: 4,
        annotations: WRITE,
        definition: act::fill_definition,
        decode: act::fill_decode,
    },
    Tool {
        name: "type",
        order: 5,
        annotations: WRITE,
        definition: act::type_definition,
        decode: act::type_decode,
    },
    Tool {
        name: "press",
        order: 6,
        annotations: WRITE,
        definition: act::press_definition,
        decode: act::press_decode,
    },
    Tool {
        name: "select",
        order: 7,
        annotations: WRITE,
        definition: act::select_definition,
        decode: act::select_decode,
    },
    Tool {
        name: "hover",
        order: 8,
        annotations: LOCAL_WRITE,
        definition: act::hover_definition,
        decode: act::hover_decode,
    },
    Tool {
        name: "drag",
        order: 9,
        annotations: WRITE,
        definition: act::drag_definition,
        decode: act::drag_decode,
    },
    Tool {
        name: "scroll",
        order: 10,
        annotations: LOCAL_WRITE,
        definition: act::scroll_definition,
        decode: act::scroll_decode,
    },
    Tool {
        name: logs::NAME,
        order: 11,
        annotations: Annotations {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: true,
        },
        definition: logs::definition,
        decode: logs::decode,
    },
    Tool {
        name: status::NAME,
        order: 12,
        annotations: LOCAL_READ,
        definition: status::definition,
        decode: status::decode,
    },
    Tool {
        name: settings::NAME,
        order: 13,
        annotations: LOCAL_READ,
        definition: settings::definition,
        decode: settings::decode,
    },
    Tool {
        name: plan::NAME,
        order: 14,
        annotations: LOCAL_WRITE,
        definition: plan::definition,
        decode: plan::decode,
    },
    Tool {
        name: network::NAME,
        order: 15,
        annotations: Annotations {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: true,
        },
        definition: network::definition,
        decode: network::decode,
    },
    Tool {
        name: secret::NAME,
        order: 16,
        annotations: WRITE,
        definition: secret::definition,
        decode: secret::decode,
    },
];

/// The registered tool with this name.
pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// Every definition in advertised order.
pub fn definitions() -> [Definition; 16] {
    TOOLS.each_ref().map(|tool| (tool.definition)())
}
