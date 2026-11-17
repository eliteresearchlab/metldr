use serde_json::Value;
use std::time::Duration;

pub type Result<T> = std::result::Result<T, String>;
const MAX_MESSAGE: usize = 65_536;
const CAPACITY: usize = 64;
const TIMEOUT: Duration = Duration::from_secs(8);

/// One browser command's id, unique for the life of its transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallId(u64);

#[derive(Debug, Clone, PartialEq)]
pub struct RawEvent {
    pub method: String,
    pub session: Option<String>,
    pub params: Value,
}

impl RawEvent {
    fn validate(&self) -> Result<()> {
        validate_envelope(&self.method, self.session.as_deref(), &self.params)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WireCommand {
    pub id: CallId,
    pub method: String,
    pub session: Option<String>,
    pub params: Value,
}

impl WireCommand {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_envelope(&self.method, self.session.as_deref(), &self.params)
    }
}

fn validate_envelope(method: &str, session: Option<&str>, params: &Value) -> Result<()> {
    if method.is_empty()
        || method.len() > 128
        || !method
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.')
        || session.is_some_and(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
        || !params.is_object()
    {
        return Err("invalid browser protocol envelope".into());
    }
    Ok(())
}

mod transport;
pub use transport::{Dispatch, Output, Transport};

mod context;
#[doc(hidden)]
pub use context::{AUTOMATION_WORLD, Change, ContextMap, FrameContext};
