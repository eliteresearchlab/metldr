//! One module per browser tool.
//!
//! Each module owns its name, its position in the advertised order, its JSON
//! schema, its safety hints and its argument decoding. `TOOLS` is the only
//! list: adding a tool means adding a module and one entry here, and every
//! caller (provider request builders, the MCP server, the installed agents)
//! picks it up without editing their own dispatch.

pub mod activate;
pub mod control;
pub mod field;
pub mod logs;
pub mod navigate;
pub mod observe;
pub mod plan;
mod schema;
pub mod scroll;
pub mod select;
pub mod settings;
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
pub static TOOLS: [Tool; 11] = [
    Tool {
        name: observe::NAME,
        order: observe::ORDER,
        annotations: Annotations {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: true,
        },
        definition: observe::definition,
        decode: observe::decode,
    },
    Tool {
        name: field::NAME,
        order: field::ORDER,
        annotations: WRITE,
        definition: field::definition,
        decode: field::decode,
    },
    Tool {
        name: navigate::NAME,
        order: navigate::ORDER,
        annotations: WRITE,
        definition: navigate::definition,
        decode: navigate::decode,
    },
    Tool {
        name: activate::NAME,
        order: activate::ORDER,
        annotations: WRITE,
        definition: activate::definition,
        decode: activate::decode,
    },
    Tool {
        name: select::NAME,
        order: select::ORDER,
        annotations: WRITE,
        definition: select::definition,
        decode: select::decode,
    },
    Tool {
        name: scroll::NAME,
        order: scroll::ORDER,
        annotations: LOCAL_WRITE,
        definition: scroll::definition,
        decode: scroll::decode,
    },
    Tool {
        name: control::NAME,
        order: control::ORDER,
        annotations: WRITE,
        definition: control::definition,
        decode: control::decode,
    },
    Tool {
        name: logs::NAME,
        order: logs::ORDER,
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
        order: status::ORDER,
        annotations: LOCAL_READ,
        definition: status::definition,
        decode: status::decode,
    },
    Tool {
        name: settings::NAME,
        order: settings::ORDER,
        annotations: LOCAL_READ,
        definition: settings::definition,
        decode: settings::decode,
    },
    Tool {
        name: plan::NAME,
        order: plan::ORDER,
        annotations: LOCAL_WRITE,
        definition: plan::definition,
        decode: plan::decode,
    },
];

/// The registered tool with this name.
pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// Every definition in advertised order.
pub fn definitions() -> [Definition; 11] {
    TOOLS.each_ref().map(|tool| (tool.definition)())
}
