#![doc = include_str!("../README.md")]

pub mod handoff;

/// Every agent harness MetLDR can drive. Adding one means one variant here, one entry in
/// [`HARNESSES`] and one adapter in the companion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HarnessId {
    Pi,
    Codex,
    OpenCode,
    ClaudeCode,
}

/// What a harness supports. MetLDR offers only the controls a harness can honour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// A later turn continues the same native session.
    pub resume: bool,
    /// A running turn can be stopped without ending the session.
    pub interrupt: bool,
    /// Text can be added to a running turn.
    pub steering: bool,
    /// Its own tool permission prompts come back to MetLDR for approval.
    pub native_approvals: bool,
    /// It can ask the user structured questions mid-turn.
    pub input_forms: bool,
    /// It uses MetLDR browser tools through MCP.
    pub browser_tools: bool,
}

pub struct Harness {
    pub id: HarnessId,
    /// The stable wire and storage key.
    pub key: &'static str,
    pub label: &'static str,
    /// Executable names searched on `PATH`, first match wins.
    pub executables: &'static [&'static str],
    pub capabilities: Capabilities,
}

pub static HARNESSES: [Harness; 4] = [
    Harness {
        id: HarnessId::Pi,
        key: "pi",
        label: "Pi",
        executables: &["pi"],
        capabilities: Capabilities {
            resume: true,
            interrupt: true,
            steering: true,
            native_approvals: false,
            input_forms: true,
            browser_tools: true,
        },
    },
    Harness {
        id: HarnessId::Codex,
        key: "codex",
        label: "Codex",
        executables: &["codex"],
        capabilities: Capabilities {
            resume: true,
            interrupt: true,
            steering: true,
            native_approvals: true,
            input_forms: true,
            browser_tools: true,
        },
    },
    Harness {
        id: HarnessId::OpenCode,
        key: "opencode",
        label: "OpenCode",
        executables: &["opencode"],
        capabilities: Capabilities {
            resume: true,
            interrupt: true,
            steering: false,
            native_approvals: true,
            input_forms: false,
            browser_tools: true,
        },
    },
    Harness {
        id: HarnessId::ClaudeCode,
        key: "claude",
        label: "Claude Code",
        executables: &["claude"],
        capabilities: Capabilities {
            resume: true,
            interrupt: true,
            steering: false,
            native_approvals: true,
            input_forms: false,
            browser_tools: true,
        },
    },
];

impl HarnessId {
    pub fn harness(self) -> &'static Harness {
        // The registry is ordered by variant, so this is a direct index.
        &HARNESSES[self as usize]
    }

    pub fn key(self) -> &'static str {
        self.harness().key
    }

    pub fn label(self) -> &'static str {
        self.harness().label
    }

    pub fn from_key(key: &str) -> Option<Self> {
        HARNESSES.iter().find(|h| h.key == key).map(|h| h.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_ordered_unique_and_round_trips() {
        for (index, harness) in HARNESSES.iter().enumerate() {
            assert_eq!(harness.id as usize, index, "registry order matches the enum");
            assert_eq!(HarnessId::from_key(harness.key), Some(harness.id));
            assert!(!harness.executables.is_empty());
        }
        assert_eq!(HarnessId::from_key("unknown"), None);
    }
}
