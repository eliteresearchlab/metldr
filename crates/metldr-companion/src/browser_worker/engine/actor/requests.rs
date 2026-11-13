use super::*;
impl Actor {
    pub(super) fn request(&mut self, request: Request) -> Result<()> {
        let Request { kind, job } = request;
        if self.busy {
            let _ = job
                .reply
                .send(Err("another browser operation is pending".into()));
            return Ok(());
        }
        self.busy = true;
        if let Some(handle) = kind.handle() {
            self.select_snapshot(handle);
        }
        if matches!(
            &kind,
            Kind::Change(_)
                | Kind::EnterSecret(_)
                | Kind::PressKey(_)
                | Kind::ActivateButton(_)
                | Kind::SetNativeControl(_)
                | Kind::SelectOption(_)
                | Kind::Scroll(_)
        ) {
            match self.child_action() {
                Ok(true) => {
                    return self.value(
                        job,
                        Outcome::Prevented {
                            reason: "actions inside frames are not supported yet".into(),
                        },
                    );
                }
                Err(error) => {
                    return self.value(
                        job,
                        Outcome::Prevented {
                            reason: error.to_string(),
                        },
                    );
                }
                Ok(false) => {}
            }
        }
        match kind {
            Kind::ActionScope { handle } => match if self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.contains(&handle))
            {
                self.child_action()
            } else {
                Ok(false)
            } {
                Ok(child) => self.value(job, child),
                Err(error) => {
                    self.finish(job, Err(error));
                    Ok(())
                }
            },
            Kind::PageLogs => match self.page_logs.snapshot() {
                Ok(snapshot) => self.value(job, snapshot),
                Err(error) => {
                    self.finish(job, Err(error));
                    Ok(())
                }
            },
            Kind::NetworkRequests => match self.network.snapshot() {
                Ok(snapshot) => self.value(job, snapshot),
                Err(error) => {
                    self.finish(job, Err(error));
                    Ok(())
                }
            },
            Kind::PrepareControl {
                tool,
                handle,
                checked,
            } => self.prepare_control(tool, handle, checked, job),
            Kind::SetNativeControl(action) => self.arm_control(action, job),
            Kind::PrepareSecret { tool, handle, kind } => {
                self.prepare_secret(tool, handle, kind, job)
            }
            Kind::EnterSecret(action) => self.enter_secret(action, job),
            Kind::PrepareKey { tool, handle, key } => self.prepare_key(tool, handle, key, job),
            Kind::PressKey(action) => self.arm_key(action, job),
            Kind::PrepareSelection {
                tool,
                handle,
                option,
            } => self.prepare_selection(tool, handle, option, job),
            Kind::SelectOption(action) => self.select_option(action, job),
            Kind::PrepareScroll {
                tool,
                handle,
                direction,
                pixels,
            } => self.prepare_scroll(tool, handle, direction, pixels, job),
            Kind::Scroll(action) => self.scroll(action, job),
            Kind::PrepareButton { tool, handle } => self.prepare_button(tool, handle, job),
            Kind::ActivateButton(action) => self.arm_button(action, job),
            Kind::Observe { generation, within } => self.observe(generation, within, job),
            Kind::PrepareChange {
                tool,
                handle,
                value,
            } => {
                let setup = (|| -> Result<_> {
                    let snapshot = self
                        .snapshot
                        .as_ref()
                        .ok_or("observe the page before changing a field")?;
                    self.contexts.validate(&snapshot.context)?;
                    let field = snapshot.field(&handle)?;
                    Ok((
                        snapshot.context.clone(),
                        snapshot.root.clone(),
                        scripts::read_value(field.slot, &field.check_label),
                    ))
                })();
                match setup {
                    Ok((context, root, script)) => self.queue(
                        "Runtime.callFunctionOn",
                        session(&context),
                        call_params(&context, &root, script),
                        Work::ReadField {
                            tool,
                            handle,
                            value,
                            job,
                        },
                        Some(context),
                        Some(root),
                    ),
                    Err(e) => {
                        self.finish(job, Err(e));
                        Ok(())
                    }
                }
            }
            Kind::Change(action) => {
                let setup = (|| -> Result<_> {
                    let snapshot = self
                        .snapshot
                        .as_ref()
                        .ok_or("browser snapshot unavailable")?;
                    self.validate_action(&action, &snapshot.context)?;
                    let Intent::Field {
                        field_id,
                        field_label,
                        previous_value,
                        value,
                    } = &action.proposal.intent
                    else {
                        return Err("not a field action".into());
                    };
                    let field = snapshot.field(field_id)?;
                    if &field.label != field_label {
                        return Err("approved field label changed".into());
                    }
                    if self.seen.len() == 64
                        || !self.seen.insert((action.id.clone(), action.revision))
                    {
                        return Err("browser action was reused or action capacity reached".into());
                    }
                    Ok((
                        snapshot.context.clone(),
                        snapshot.root.clone(),
                        scripts::write_value(field.slot, &field.check_label, previous_value, value),
                    ))
                })();
                match setup {
                    Ok((context, root, script)) => self.queue(
                        "Runtime.callFunctionOn",
                        session(&context),
                        call_params(&context, &root, script),
                        Work::Write { action, job },
                        Some(context),
                        Some(root),
                    ),
                    Err(e) => self.value(
                        job,
                        Outcome::Prevented {
                            reason: e.to_string(),
                        },
                    ),
                }
            }
            Kind::PrepareNavigation { tool, destination } => {
                let result = (|| -> Result<Value> {
                    let context = self.context()?;
                    if crate::model::protected_page(&destination) {
                        return Err("protected browser page is not accessible".into());
                    }
                    if crate::model::validate_page_url(&destination).is_err() {
                        return Err("navigation destination is not permitted".into());
                    }
                    Ok(serde_json::to_value(Proposal {
                        run: self.client.run.clone(),
                        tool_use_id: tool,
                        url: self.source.url.clone(),
                        binding: self.binding(&context),
                        intent: Intent::Navigate { destination },
                    })?)
                })();
                self.finish(job, result);
                Ok(())
            }
            Kind::Navigate(action) => {
                let setup = (|| -> Result<_> {
                    let context = self.context()?;
                    self.validate_action(&action, &context)?;
                    let Intent::Navigate { destination } = &action.proposal.intent else {
                        return Err("not a navigation action".into());
                    };
                    if crate::model::protected_page(destination) {
                        return Err("protected browser page is not accessible".into());
                    }
                    if crate::model::validate_page_url(destination).is_err() {
                        return Err("navigation destination is not permitted".into());
                    }
                    if self.seen.len() == 64
                        || !self.seen.insert((action.id.clone(), action.revision))
                    {
                        return Err("browser action was reused or action capacity reached".into());
                    }
                    Ok((context, destination.clone()))
                })();
                match setup {
                    Ok((context, destination)) => {
                        self.snapshot = None;
                        self.queue(
                            "Page.navigate",
                            None,
                            json!({"url":destination}),
                            Work::Navigate {
                                action,
                                old: context.clone(),
                                job,
                            },
                            Some(context),
                            None,
                        )
                    }
                    Err(e) => self.value(
                        job,
                        Outcome::Prevented {
                            reason: e.to_string(),
                        },
                    ),
                }
            }
        }
    }
    pub(super) fn capture(
        &mut self,
        context: FrameContext,
        generation: u64,
        job: Job,
    ) -> Result<()> {
        self.queue("Runtime.evaluate",session(&context),json!({"expression":format!("({})()",scripts::capture(None)),"returnByValue":false,"awaitPromise":true,"uniqueContextId":context.unique_context_id()}),Work::Capture{generation,job},Some(context),None)
    }
}
