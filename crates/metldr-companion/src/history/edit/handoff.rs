//! The chat runs a harness has not seen yet, for its context handoff (B1.12.4).

use super::{Edit, Selection, record, storage};
use crate::{Result, agent_model::Launch, history::schema::*};
use redb::ReadableTable;

/// One finished run of the chat: the user's question and the answer of its route.
pub(crate) struct HandoffRun {
    pub number: u32,
    pub route: &'static str,
    pub question: String,
    pub answer: String,
}

impl Edit<'_> {
    /// The latest `limit` finished runs of the selected chat numbered above `after`, oldest
    /// first, leaving out the runs of `skip`, the workflow that already holds them.
    pub(crate) fn handoff_runs(
        &self,
        scope: &Selection,
        after: u64,
        skip: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HandoffRun>> {
        let ids = storage(|| {
            let order = self.write.open_table(RUN_ORDER)?;
            let chat = scope.conversation.as_str();
            let mut ids = Vec::new();
            for entry in order
                .range((chat, after.saturating_add(1))..=(chat, u64::MAX))?
                .rev()
            {
                let (key, id) = entry?;
                ids.push((key.value().1, id.value().to_owned()));
                if ids.len() == limit {
                    break;
                }
            }
            Ok(ids)
        })?;
        let mut runs = Vec::with_capacity(ids.len());
        for (number, id) in ids.into_iter().rev() {
            let Some(run) = self.saved_run(&id)? else {
                continue;
            };
            let launch = record::<Launch>(self.write, LAUNCHES, &id)?.map(|saved| saved.value);
            if !run.value.status.terminal()
                || skip
                    .is_some_and(|w| launch.as_ref().and_then(Launch::native_workflow) == Some(w))
            {
                continue;
            }
            let answer = match (&run.value.error, run.value.status) {
                (Some(error), _) => format!("{} (failed: {error})", run.value.output.text()),
                (None, crate::model::Status::Interrupted) => {
                    format!("{} (stopped before it finished)", run.value.output.text())
                }
                _ => run.value.output.text().into_owned(),
            };
            runs.push(HandoffRun {
                number: u32::try_from(number)?,
                route: launch.as_ref().map_or("Assistant", Launch::label),
                question: run.value.question.unwrap_or_default(),
                answer,
            });
        }
        Ok(runs)
    }
}
