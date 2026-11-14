use serde::{Deserialize, Serialize};

/// Token counts that one model response reported. `input` is the whole prompt with cached reads
/// included, and `output` includes reasoning. A count the provider did not report stays `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
}

impl Usage {
    /// Adds counts field by field; a field stays `None` only when both sides lack it.
    pub fn add(self, other: Self) -> Self {
        let sum = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) => Some(a.saturating_add(b)),
            (a, b) => a.or(b),
        };
        Self {
            input: sum(self.input, other.input),
            cached: sum(self.cached, other.cached),
            output: sum(self.output, other.output),
            reasoning: sum(self.reasoning, other.reasoning),
        }
    }
}

/// Steps beyond this bound are folded into the last one, so a saved ledger stays small.
pub const MAX_LEDGER_STEPS: usize = 16;

/// Per-run usage ledger (phase R1): tokens and timings of each model step, saved with the run.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub steps: Vec<Step>,
    /// Wall time of the run in milliseconds, model and tools together.
    pub ms: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub usage: Usage,
    /// Time from the request to the first streamed text, when the step streamed text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_ms: Option<u32>,
    /// Time from the request to the completed response.
    pub ms: u32,
    /// Whether the step ended in a tool call.
    pub call: bool,
}

impl Ledger {
    pub fn valid(&self) -> bool {
        !self.steps.is_empty() && self.steps.len() <= MAX_LEDGER_STEPS
    }

    pub fn total(&self) -> Usage {
        self.steps
            .iter()
            .fold(Usage::default(), |sum, step| sum.add(step.usage))
    }
}
