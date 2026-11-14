//! Measures the per-run usage ledger (phase R1) while a run executes.

use metldr_conversation::{Ledger, MAX_LEDGER_STEPS, Step, Usage};
use std::time::Instant;

/// Measures one run while it executes.
pub(crate) struct Recorder {
    started: Instant,
    step: Instant,
    first: Option<u32>,
    ledger: Ledger,
}

fn ms(since: Instant) -> u32 {
    since.elapsed().as_millis().try_into().unwrap_or(u32::MAX)
}

impl Recorder {
    pub(crate) fn start() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            step: now,
            first: None,
            ledger: Ledger::default(),
        }
    }

    pub(crate) fn begin_step(&mut self) {
        self.step = Instant::now();
        self.first = None;
    }

    /// Marks streamed text; only the first mark of a step counts.
    pub(crate) fn text(&mut self) {
        if self.first.is_none() {
            self.first = Some(ms(self.step));
        }
    }

    pub(crate) fn end_step(&mut self, usage: Option<Usage>, call: bool) {
        let step = Step {
            usage: usage.unwrap_or_default(),
            first_ms: self.first,
            ms: ms(self.step),
            call,
        };
        let full = self.ledger.steps.len() == MAX_LEDGER_STEPS;
        match self.ledger.steps.last_mut() {
            Some(last) if full => {
                last.usage = last.usage.add(step.usage);
                last.ms = last.ms.saturating_add(step.ms);
                last.call |= step.call;
            }
            _ => self.ledger.steps.push(step),
        }
    }

    /// The ledger, or `None` when no model step completed.
    pub(crate) fn finish(mut self) -> Option<Ledger> {
        self.ledger.ms = ms(self.started);
        (!self.ledger.steps.is_empty()).then_some(self.ledger)
    }
}
