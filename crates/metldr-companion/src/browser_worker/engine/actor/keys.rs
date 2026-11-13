use super::*;

impl Actor {
    /// Proposes one key press in an observed text field.
    pub(super) fn prepare_key(
        &mut self,
        tool: String,
        handle: String,
        key: metldr_browser_tools::Key,
        job: Job,
    ) -> Result<()> {
        let proposal = (|| -> Result<Proposal> {
            let snapshot = self
                .snapshot
                .as_ref()
                .ok_or("observe the page before pressing a key")?;
            self.contexts.validate(&snapshot.context)?;
            let field = snapshot.field(&handle)?;
            Ok(Proposal {
                run: self.client.run.clone(),
                tool_use_id: tool,
                url: self.source.url.clone(),
                binding: self.binding(&snapshot.context),
                intent: Intent::PressKey {
                    field_id: handle,
                    field_label: field.label.clone(),
                    key: key.as_str().into(),
                },
            })
        })();
        match proposal {
            Ok(proposal) => self.value(job, proposal),
            Err(error) => {
                self.finish(job, Err(error));
                Ok(())
            }
        }
    }

    /// Focuses the approved field and arms the keydown receipt before the trusted key.
    pub(super) fn arm_key(&mut self, action: Action, job: Job) -> Result<()> {
        let setup = (|| -> Result<_> {
            let snapshot = self
                .snapshot
                .as_ref()
                .ok_or("browser snapshot unavailable")?;
            self.validate_action(&action, &snapshot.context)?;
            let Intent::PressKey {
                field_id,
                field_label,
                key,
            } = &action.proposal.intent
            else {
                return Err("not a key action".into());
            };
            let field = snapshot.field(field_id)?;
            if &field.label != field_label {
                return Err("approved field label changed".into());
            }
            if self.seen.len() == 64 || !self.seen.insert((action.id.clone(), action.revision)) {
                return Err("browser action was reused or action capacity reached".into());
            }
            Ok((
                snapshot.context.clone(),
                snapshot.root.clone(),
                scripts::arm_key(field.slot, &field.check_label, key),
            ))
        })();
        match setup {
            Ok((context, root, script)) => self.queue(
                "Runtime.callFunctionOn",
                session(&context),
                call_params(&context, &root, script),
                Work::ArmKey { action, job },
                Some(context),
                Some(root),
            ),
            Err(error) => self.value(
                job,
                Outcome::Prevented {
                    reason: error.to_string(),
                },
            ),
        }
    }

    pub(super) fn key_armed(&mut self, action: Action, job: Job, value: &Value) -> Result<()> {
        if !results::evaluation_value(value).is_ok_and(|v| v == Value::Bool(true)) {
            return self.value(
                job,
                Outcome::Prevented {
                    reason: "field changed or could not take focus before the key".into(),
                },
            );
        }
        let Intent::PressKey { key, .. } = &action.proposal.intent else {
            return Err("invalid key intent".into());
        };
        let snapshot = self.snapshot.as_ref().ok_or("key snapshot disappeared")?;
        let (context, root) = (snapshot.context.clone(), snapshot.root.clone());
        let params = json!({ "key": key });
        self.queue(
            "Input.dispatchKeyEvent",
            session(&context),
            params,
            Work::KeyGesture { action, job },
            Some(context),
            Some(root),
        )
    }

    pub(super) fn key_pressed(&mut self, job: Job) -> Result<()> {
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or("key snapshot disappeared after input")?;
        let (context, root) = (snapshot.context.clone(), snapshot.root.clone());
        self.queue(
            "Runtime.callFunctionOn",
            session(&context),
            call_params(&context, &root, scripts::key_receipt()),
            Work::KeyReceipt { job },
            Some(context),
            Some(root),
        )
    }
}
