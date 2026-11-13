use super::*;

impl Kind {
    pub(super) fn handle(&self) -> Option<&str> {
        match self {
            Self::ActionScope { handle }
            | Self::PrepareChange { handle, .. }
            | Self::PrepareSecret { handle, .. }
            | Self::PrepareKey { handle, .. }
            | Self::PrepareButton { handle, .. }
            | Self::PrepareControl { handle, .. }
            | Self::PrepareSelection { handle, .. }
            | Self::PrepareScroll { handle, .. } => Some(handle),
            Self::Change(action)
            | Self::EnterSecret(action)
            | Self::PressKey(action)
            | Self::ActivateButton(action)
            | Self::SetNativeControl(action)
            | Self::SelectOption(action)
            | Self::Scroll(action) => match &action.proposal.intent {
                Intent::Field { field_id, .. }
                | Intent::Secret { field_id, .. }
                | Intent::PressKey { field_id, .. } => Some(field_id),
                Intent::ActivateButton { button_id, .. } => Some(button_id),
                Intent::SelectOption { selection } => Some(&selection.control_id),
                Intent::SetNativeControl { control } => match control {
                    crate::browser_action::Control::Checkbox { control_id, .. }
                    | crate::browser_action::Control::Radio { control_id, .. } => Some(control_id),
                },
                Intent::Scroll { scroll } => Some(&scroll.handle),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Actor {
    pub(super) fn child_action(&self) -> Result<bool> {
        let Some(snapshot) = &self.snapshot else {
            return Ok(false);
        };
        self.contexts.validate(&snapshot.context)?;
        Ok(self.contexts.main_context().as_ref() != Some(&snapshot.context))
    }
    pub(super) fn select_snapshot(&mut self, handle: &str) {
        if self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.contains(handle))
        {
            return;
        }
        if let Some(index) = self
            .snapshots
            .iter()
            .position(|snapshot| snapshot.contains(handle))
        {
            let selected = self.snapshots.swap_remove(index);
            if let Some(previous) = self.snapshot.replace(selected) {
                self.snapshots.push(previous);
            }
        }
    }

    pub(super) fn observe(
        &mut self,
        generation: u64,
        within: Option<String>,
        job: Job,
    ) -> Result<()> {
        let setup = (|| -> Result<_> {
            if generation <= self.generation {
                return Err("observation generation was reused".into());
            }
            // A child frame never inherits a grant after the main document changes.
            self.contexts.validate(
                self.granted_root
                    .as_ref()
                    .ok_or("browser grant unavailable")?,
            )?;
            if let Some(handle) = &within {
                self.select_snapshot(handle);
                let snapshot = self
                    .snapshot
                    .as_ref()
                    .ok_or("stale snapshot ref: observe the page again")?;
                self.contexts.validate(&snapshot.context)?;
                let scope = snapshot.scope(handle)?;
                Ok((
                    vec![snapshot.context.clone()],
                    Some((snapshot.root.clone(), scope)),
                    false,
                ))
            } else {
                let (contexts, truncated) = self.contexts.observation_contexts();
                Ok((contexts, None, truncated))
            }
        })();
        let (mut contexts, scope, truncated) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                self.finish(job, Err(error));
                return Ok(());
            }
        };
        if contexts.is_empty() {
            self.finish(job, Err("browser frame unavailable".into()));
            return Ok(());
        }
        self.generation = generation;
        self.frames_truncated = truncated || contexts.len() > 9;
        contexts.truncate(9);
        self.observation_results.clear();
        self.observation_bytes = 0;
        self.observation_contexts = contexts.into();
        self.retired_snapshots.append(&mut self.snapshots);
        if let Some(snapshot) = self.snapshot.take() {
            self.retired_snapshots.push(snapshot);
        }
        let context = self.observation_contexts.pop_front().unwrap();
        if let Some((root, scope)) = scope {
            let mut params = call_params(&context, &root, scripts::capture(Some(&scope)));
            params["returnByValue"] = json!(false);
            self.queue(
                "Runtime.callFunctionOn",
                session(&context),
                params,
                Work::Capture { generation, job },
                Some(context),
                None,
            )
        } else {
            self.capture(context, generation, job)
        }
    }

    pub(super) fn capture_next(&mut self, generation: u64, job: Job) -> Result<()> {
        if self.observation_bytes > 48 * 1024 {
            self.observation_results.pop();
            if let Some(snapshot) = self.snapshot.take() {
                self.retired_snapshots.push(snapshot);
            }
            self.observation_contexts.clear();
            self.frames_truncated = true;
        }
        if let Some(context) = self.observation_contexts.pop_front() {
            if let Some(snapshot) = self.snapshot.take() {
                self.snapshots.push(snapshot);
            }
            return self.capture(context, generation, job);
        }
        let mut observations = std::mem::take(&mut self.observation_results).into_iter();
        let mut result = observations
            .next()
            .ok_or("browser observation unavailable")?;
        let frames: Vec<_> = observations.collect();
        if !frames.is_empty() {
            result["frames"] = json!(frames);
        }
        if self.frames_truncated {
            result["frames_truncated"] = json!(true);
        }
        self.release_retired()?;
        self.finish(job, Ok(result));
        Ok(())
    }

    pub(super) fn abort_observation(
        &mut self,
        job: Job,
        error: Box<dyn std::error::Error + Send + Sync>,
    ) -> Result<()> {
        self.observation_contexts.clear();
        self.observation_results.clear();
        self.retired_snapshots.append(&mut self.snapshots);
        if let Some(snapshot) = self.snapshot.take() {
            self.retired_snapshots.push(snapshot);
        }
        self.release_retired()?;
        self.finish(job, Err(error));
        Ok(())
    }

    fn release_retired(&mut self) -> Result<()> {
        for snapshot in std::mem::take(&mut self.retired_snapshots) {
            if self.contexts.validate(&snapshot.context).is_ok() {
                self.queue(
                    "Runtime.releaseObject",
                    session(&snapshot.context),
                    json!({"objectId":snapshot.root}),
                    Work::Cleanup,
                    Some(snapshot.context),
                    None,
                )?;
            }
        }
        Ok(())
    }
}
