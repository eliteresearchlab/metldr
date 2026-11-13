use super::*;
use crate::browser_action::{SECRET_TTL_MS, SecretKind};

impl Actor {
    /// Proposes one secret entry for an observed password or one-time code field.
    pub(super) fn prepare_secret(
        &mut self,
        tool: String,
        handle: String,
        kind: SecretKind,
        job: Job,
    ) -> Result<()> {
        let proposal = (|| -> Result<Proposal> {
            if crate::model::protected_page(&self.source.url) {
                return Err("protected browser page is not accessible".into());
            }
            let snapshot = self
                .snapshot
                .as_ref()
                .ok_or("observe the page before entering a secret")?;
            self.contexts.validate(&snapshot.context)?;
            let (field, observed) = snapshot.secret_field(&handle)?;
            if observed != kind {
                return Err("the field asks for a different kind of secret".into());
            }
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|e| e.to_string())?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "system clock is before 1970")?;
            Ok(Proposal {
                run: self.client.run.clone(),
                tool_use_id: tool,
                url: self.source.url.clone(),
                binding: self.binding(&snapshot.context),
                intent: Intent::Secret {
                    field_id: handle,
                    field_label: field.label.clone(),
                    secret: kind,
                    reference: random.iter().map(|b| format!("{b:02x}")).collect(),
                    expires_at_ms: u64::try_from(now.as_millis())? + SECRET_TTL_MS,
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

    /// Queues the approved entry. The owner adds the secret to this command; the actor never
    /// holds it.
    pub(super) fn enter_secret(&mut self, action: Action, job: Job) -> Result<()> {
        let setup = (|| -> Result<_> {
            let snapshot = self
                .snapshot
                .as_ref()
                .ok_or("browser snapshot unavailable")?;
            self.validate_action(&action, &snapshot.context)?;
            let Intent::Secret {
                field_id,
                field_label,
                secret,
                ..
            } = &action.proposal.intent
            else {
                return Err("not a secret entry".into());
            };
            let (field, observed) = snapshot.secret_field(field_id)?;
            if &field.label != field_label || observed != *secret {
                return Err("approved secret field changed".into());
            }
            if self.seen.len() == 64 || !self.seen.insert((action.id.clone(), action.revision)) {
                return Err("browser action was reused or action capacity reached".into());
            }
            let kind = match secret {
                SecretKind::Password => "password",
                SecretKind::OneTimeCode => "one_time_code",
            };
            Ok((
                snapshot.context.clone(),
                snapshot.root.clone(),
                scripts::write_secret(field.slot, &field.check_label, kind),
            ))
        })();
        match setup {
            Ok((context, root, script)) => {
                // Page listeners may echo the secret to the console, so capture stops before
                // the entry and stays off until the next navigation.
                self.page_logs.pause_after_secret();
                self.queue(
                    "Runtime.callFunctionOn",
                    session(&context),
                    call_params(&context, &root, script),
                    Work::WriteSecret { action, job },
                    Some(context),
                    Some(root),
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

    /// Settles the entry from the page's yes or no.
    pub(super) fn secret_written(&mut self, action: Action, job: Job, value: &Value) -> Result<()> {
        let Intent::Secret { reference, .. } = &action.proposal.intent else {
            return Err("invalid secret entry intent".into());
        };
        let outcome = match results::evaluation_value(value)
            .and_then(|v| Ok(serde_json::from_value::<bool>(v)?))
        {
            Ok(true) => Outcome::SecretEntered {
                reference: reference.clone(),
            },
            Ok(false) => Outcome::Prevented {
                reason: "secret field changed before entry".into(),
            },
            Err(_) => Outcome::Uncertain {
                reason: "secret entry could not be confirmed".into(),
            },
        };
        self.value(job, outcome)
    }
}
