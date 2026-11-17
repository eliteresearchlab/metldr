use super::*;
use crate::browser_action::{Element, SECRET_TTL_MS, SecretKind};
use crate::browser_worker::engine::page::{Route, route};

/// The longest an approved step may hold its act window open.
const ACT_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

impl Actor {
    pub(super) fn request(&mut self, request: Request) -> Result<()> {
        let Request { kind, job } = request;
        if let Kind::Page {
            method,
            session,
            params,
        } = kind
        {
            return self.page_command(method, session, params, job);
        }
        if self.busy {
            let _ = job
                .reply
                .send(Err("another browser operation is pending".into()));
            return Ok(());
        }
        self.busy = true;
        match kind {
            Kind::Page { .. } => Err("browser page command took the action path".into()),
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
            Kind::PrepareAct {
                tool,
                act,
                args,
                elements,
            } => {
                let proposal = self.proposal(
                    tool,
                    Intent::Act {
                        tool: act,
                        args,
                        elements,
                    },
                );
                self.reply(job, proposal)
            }
            Kind::PrepareSecret {
                tool,
                element,
                kind,
            } => {
                let proposal =
                    secret_intent(element, kind).and_then(|intent| self.proposal(tool, intent));
                self.reply(job, proposal)
            }
            Kind::ActBegin(action) => self.act_begin(action, job),
            Kind::ActEnd(action) => self.act_end(action, job),
            Kind::EnterSecret(action) => self.enter_secret(action, job),
            Kind::PrepareNavigation { tool, destination } => {
                let proposal = (|| -> Result<Proposal> {
                    if crate::model::protected_page(&destination) {
                        return Err("protected browser page is not accessible".into());
                    }
                    if crate::model::validate_page_url(&destination).is_err() {
                        return Err("navigation destination is not permitted".into());
                    }
                    self.proposal(tool, Intent::Navigate { destination })
                })();
                self.reply(job, proposal)
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
                    self.claim(&action)?;
                    Ok((context, destination.clone()))
                })();
                match setup {
                    Ok((context, destination)) => self.queue(
                        "Page.navigate",
                        None,
                        json!({"url":destination}),
                        Work::Navigate {
                            action,
                            old: context.clone(),
                            job,
                        },
                        Some(context),
                    ),
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

    /// A proposal on the current main frame for the attached page.
    fn proposal(&self, tool: String, intent: Intent) -> Result<Proposal> {
        if crate::model::protected_page(&self.source.url) {
            return Err("protected browser page is not accessible".into());
        }
        let context = self.context()?;
        let proposal = Proposal {
            run: self.client.run.clone(),
            tool_use_id: tool,
            url: self.source.url.clone(),
            binding: self.binding(&context),
            intent,
        };
        proposal.validate()?;
        Ok(proposal)
    }

    fn reply(&mut self, job: Job, value: Result<Proposal>) -> Result<()> {
        match value {
            Ok(proposal) => self.value(job, proposal),
            Err(error) => {
                self.finish(job, Err(error));
                Ok(())
            }
        }
    }

    /// Uses an approved action once.
    fn claim(&mut self, action: &Action) -> Result<()> {
        if self.seen.len() == 64 || !self.seen.insert((action.id.clone(), action.revision)) {
            return Err("browser action was reused or action capacity reached".into());
        }
        Ok(())
    }

    /// Opens the act window of an approved step in the extension; its reply finishes the job.
    fn act_begin(&mut self, action: Action, job: Job) -> Result<()> {
        let setup = (|| -> Result<_> {
            let context = self.context()?;
            self.validate_action(&action, &context)?;
            if !matches!(
                &action.proposal.intent,
                Intent::Act { .. } | Intent::Secret { .. }
            ) {
                return Err("not a page action".into());
            }
            if self.acting.is_some() {
                return Err("another act window is open".into());
            }
            self.claim(&action)?;
            let admission = job
                .admission
                .as_ref()
                .ok_or("browser action admission absent")?;
            admission.begin(
                &self.client,
                Operation::ActBegin {
                    action: action.id.clone(),
                    revision: action.revision,
                    binding: action.proposal.binding.clone(),
                    url: action.proposal.url.clone(),
                },
            )
        })();
        match setup {
            Ok(reply) => {
                self.acting = Some((action.id, Instant::now() + ACT_WINDOW));
                let deadline = job.deadline;
                self.inflight.push(
                    async move {
                        let result = wait_reply(deadline, reply).await.and_then(|_| {
                            serde_json::to_value(Outcome::Acted {
                                result: String::new(),
                            })
                            .map_err(|e| e.to_string())
                        });
                        (Completion::Window(job), result)
                    }
                    .boxed(),
                );
                Ok(())
            }
            Err(error) => self.value(
                job,
                Outcome::Prevented {
                    reason: error.to_string(),
                },
            ),
        }
    }

    /// Closes the act window of `action` here and in the extension.
    fn act_end(&mut self, action: String, job: Job) -> Result<()> {
        if self.acting.as_ref().is_none_or(|(open, _)| *open != action) {
            return self.value(job, json!({}));
        }
        self.acting = None;
        match self.client.begin(Operation::ActEnd { action }) {
            Ok(reply) => {
                let deadline = job.deadline;
                self.inflight.push(
                    async move { (Completion::Window(job), wait_reply(deadline, reply).await) }
                        .boxed(),
                );
                Ok(())
            }
            Err(error) => {
                self.finish(job, Err(error));
                Ok(())
            }
        }
    }

    /// Queues the approved entry as one trusted text insertion into the focused field. The owner
    /// adds the secret to this command; the actor never holds it.
    fn enter_secret(&mut self, action: Action, job: Job) -> Result<()> {
        let setup = (|| -> Result<_> {
            let context = self.context()?;
            self.validate_action(&action, &context)?;
            if !matches!(&action.proposal.intent, Intent::Secret { .. }) {
                return Err("not a secret entry".into());
            }
            if self
                .acting
                .as_ref()
                .is_none_or(|(open, _)| *open != action.id)
            {
                return Err("the secret entry has no open act window".into());
            }
            Ok(context)
        })();
        match setup {
            Ok(context) => {
                // Page listeners may echo the secret to the console, so capture stops before
                // the entry and stays off until the next navigation.
                self.page_logs.pause_after_secret();
                self.queue(
                    "Input.insertText",
                    None,
                    json!({"text":""}),
                    Work::EnterSecret { action, job },
                    Some(context),
                )
            }
            Err(error) => self.value(
                job,
                Outcome::Prevented {
                    reason: error.to_string(),
                },
            ),
        }
    }

    /// Sends one command from Browse's page layer for a session the actor owns, without taking
    /// the actor. Input passes only while an approved step's act window is open.
    fn page_command(
        &mut self,
        method: String,
        session: Option<String>,
        params: Value,
        job: Job,
    ) -> Result<()> {
        if !self.contexts.owns(session.as_deref()) {
            let _ = job.reply.send(Err("the session is not this tab's".into()));
            return Ok(());
        }
        let open = self
            .acting
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() < *until);
        match route(&method) {
            Route::Done => {
                let _ = job.reply.send(Ok(json!({})));
                Ok(())
            }
            Route::Input if !open => {
                let _ = job
                    .reply
                    .send(Err("input is allowed only during an approved step".into()));
                Ok(())
            }
            Route::Refused => {
                let _ = job
                    .reply
                    .send(Err(format!("{method} is not available in metldr")));
                Ok(())
            }
            Route::Send | Route::Input => {
                self.queue(method, session, params, Work::Page(job), None)
            }
        }
    }
}

/// The intent of one secret entry, with a fresh opaque reference and its expiry.
fn secret_intent(element: Element, kind: SecretKind) -> Result<Intent> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock is before 1970")?;
    Ok(Intent::Secret {
        element,
        secret: kind,
        reference: random.iter().map(|b| format!("{b:02x}")).collect(),
        expires_at_ms: u64::try_from(now.as_millis())? + SECRET_TTL_MS,
    })
}
