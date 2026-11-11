//! MCP protocol revision negotiation.
//!
//! The specification asks a server to answer with a revision it supports, not
//! the one the client asked for, and to require that revision on every later
//! request. This build supports one revision; the seam exists so a later
//! revision is a change to `SUPPORTED` alone, and the header check follows it.

/// Revisions this build can answer, newest first.
pub(super) const SUPPORTED: [&str; 1] = ["2025-11-25"];

/// The newest revision this build supports.
pub(super) fn latest() -> &'static str {
    SUPPORTED[0]
}

/// The revision to answer an `initialize` request with.
pub(super) fn negotiate(requested: &str) -> &'static str {
    SUPPORTED
        .iter()
        .copied()
        .find(|revision| *revision == requested)
        .unwrap_or_else(latest)
}

/// Whether a later request carries the negotiated revision.
pub(super) fn accepts_header(value: &str) -> bool {
    value == latest()
}
