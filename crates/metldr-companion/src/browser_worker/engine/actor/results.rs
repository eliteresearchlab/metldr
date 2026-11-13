use super::*;
use chromiumoxide::cdp::js_protocol::runtime::{
    RemoteObject, RemoteObjectSubtype, RemoteObjectType,
};
impl Actor {
    pub(super) fn result(&mut self, id: CallId, result: runtime::Result<Value>) -> Result<()> {
        let pending = self.pending.remove(&id).ok_or("unknown actor response")?;
        let context = pending.context;
        let guarded = (|| -> Result<Value> {
            let value =
                result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            if !matches!(pending.work, Work::Navigate { .. }) {
                if let Some(context) = &context {
                    self.contexts.validate(context)?;
                }
            }
            if let Some(object) = &pending.object {
                if self.snapshot.as_ref().is_none_or(|s| &s.root != object) {
                    return Err("browser snapshot changed during request".into());
                }
            }
            Ok(value)
        })();
        if let Some(reason) = pending.prevented {
            return self.failed(pending.work, reason, true);
        }
        let value = match guarded {
            Ok(value) => value,
            Err(e) => return self.failed(pending.work, e.to_string(), false),
        };
        if let Some(reason) = ref_error(&value) {
            return self.failed(
                pending.work,
                format!("{reason}; observe the page again"),
                true,
            );
        }
        match pending.work {
            Work::Init(init) => {
                if init.page && init.step == 1 {
                    self.contexts
                        .tree(init.session.as_deref(), &value["frameTree"])?;
                }
                self.initialized(init)
            }
            Work::CreateWorld => Ok(()),
            Work::Capture { generation, job } => {
                let context = context.ok_or("capture lost its context")?;
                let root = match evaluation(&value).and_then(|o| {
                    o.object_id
                        .ok_or_else(|| "browser observation did not retain its snapshot".into())
                }) {
                    Ok(root) => root,
                    Err(e) => return self.abort_observation(job, e),
                };
                self.snapshot = Some(Snapshot {
                    context: context.clone(),
                    root: root.clone(),
                    fields: BTreeMap::new(),
                    buttons: BTreeMap::new(),
                    selects: BTreeMap::new(),
                    scroll_targets: BTreeMap::new(),
                    controls: BTreeMap::new(),
                });
                self.queue(
                    "Runtime.callFunctionOn",
                    session(&context),
                    call_params(&context, &root, scripts::read_summary()),
                    Work::Census { generation, job },
                    Some(context),
                    Some(root),
                )
            }
            Work::Census { generation, job } => {
                let provisional = self
                    .snapshot
                    .take()
                    .ok_or("snapshot disappeared during census")?;
                let parsed = evaluation_value(&value).and_then(|value| {
                    Snapshot::from_census(
                        provisional.context.clone(),
                        provisional.root.clone(),
                        &if self.contexts.main_context().as_ref() == Some(&provisional.context) {
                            self.client.run.clone()
                        } else {
                            format!("{}:{}", self.client.run, provisional.context.frame_id())
                        },
                        generation,
                        value,
                    )
                });
                match parsed {
                    Ok((snapshot, observation)) => {
                        self.snapshot = Some(snapshot);
                        self.observation_bytes += observation.to_string().len();
                        self.observation_results.push(observation);
                        self.capture_next(generation, job)
                    }
                    Err(e) => {
                        if self.contexts.validate(&provisional.context).is_ok() {
                            self.queue(
                                "Runtime.releaseObject",
                                session(&provisional.context),
                                json!({"objectId":provisional.root}),
                                Work::Cleanup,
                                Some(provisional.context),
                                None,
                            )?;
                        }
                        self.abort_observation(job, e)
                    }
                }
            }
            Work::ReadButton { tool, handle, job } => self.button_read(tool, handle, job, value),
            Work::ReadSelection {
                tool,
                handle,
                option,
                job,
            } => self.selection_read(tool, handle, option, job, value),
            Work::ReadScroll {
                tool,
                handle,
                direction,
                pixels,
                job,
            } => self.scroll_read(tool, handle, direction, pixels, job, value),
            Work::WriteSelection { action, job } => self.selection_written(action, job, value),
            Work::VerifySelection { action, job } => self.selection_verified(action, job, value),
            Work::WriteScroll { action, job } => self.scroll_written(action, job, value),
            Work::VerifyScroll { after, job } => self.scroll_verified(after, job, value),
            Work::ReadControl {
                tool,
                handle,
                checked,
                job,
            } => self.control_read(tool, handle, checked, job, value),
            Work::ArmButton { action, job } | Work::ArmControl { action, job } => {
                self.tap_armed(action, job, value)
            }
            Work::Gesture { action, job }
                if matches!(&action.proposal.intent, Intent::SetNativeControl { .. }) =>
            {
                self.control_gestured(action, job)
            }
            Work::Gesture { job, .. } => self.button_gestured(job),
            Work::ArmKey { action, job } => self.key_armed(action, job, &value),
            Work::KeyGesture { job, .. } => self.key_pressed(job),
            Work::KeyReceipt { job } => {
                let delivered = evaluation_value(&value).is_ok_and(|v| v == Value::Bool(true));
                self.value(
                    job,
                    if delivered {
                        Outcome::KeyPressObserved
                    } else {
                        Outcome::Uncertain {
                            reason: "approved key press could not be verified".into(),
                        }
                    },
                )
            }
            Work::ControlReceipt { action, job } => self.control_receipt(action, job, value),
            Work::ButtonReceipt { job } => {
                let delivered = evaluation_value(&value).is_ok_and(|v| v == Value::Bool(true));
                self.value(
                    job,
                    if delivered {
                        Outcome::ButtonClickObserved
                    } else {
                        Outcome::Uncertain {
                            reason: "approved button click could not be verified".into(),
                        }
                    },
                )
            }
            Work::ReadField {
                tool,
                handle,
                value: desired,
                job,
            } => {
                let proposal = (|| -> Result<_> {
                    let previous: Option<String> =
                        serde_json::from_value(evaluation_value(&value)?)?;
                    let previous = previous
                        .filter(|s| s.len() <= 1024)
                        .ok_or("field is unavailable or oversized")?;
                    let snapshot = self.snapshot.as_ref().ok_or("snapshot unavailable")?;
                    let field = snapshot.field(&handle)?;
                    Ok(Proposal {
                        run: self.client.run.clone(),
                        tool_use_id: tool,
                        url: self.source.url.clone(),
                        binding: self.binding(&snapshot.context),
                        intent: Intent::Field {
                            field_id: handle,
                            field_label: field.label.clone(),
                            previous_value: previous,
                            value: desired,
                        },
                    })
                })();
                match proposal {
                    Ok(proposal) => self.value(job, proposal),
                    Err(e) => {
                        self.finish(job, Err(e));
                        Ok(())
                    }
                }
            }
            Work::WriteSecret { action, job } => self.secret_written(action, job, &value),
            Work::Write { action, job } => {
                let written =
                    evaluation_value(&value).and_then(|v| Ok(serde_json::from_value::<bool>(v)?));
                match written {
                    Ok(false) => self.value(
                        job,
                        Outcome::Prevented {
                            reason: "field changed before dispatch".into(),
                        },
                    ),
                    Ok(true) => {
                        let snapshot = self
                            .snapshot
                            .as_ref()
                            .ok_or("snapshot disappeared after write")?;
                        let Intent::Field {
                            field_id, value, ..
                        } = &action.proposal.intent
                        else {
                            return Err("invalid field verification intent".into());
                        };
                        let field = snapshot.field(field_id)?;
                        let context = snapshot.context.clone();
                        let root = snapshot.root.clone();
                        self.queue(
                            "Runtime.callFunctionOn",
                            session(&context),
                            call_params(
                                &context,
                                &root,
                                scripts::verify_value(field.slot, &field.check_label, value),
                            ),
                            Work::Verify { action, job },
                            Some(context),
                            Some(root),
                        )
                    }
                    Err(_) => self.value(
                        job,
                        Outcome::Uncertain {
                            reason: "field effect could not be verified".into(),
                        },
                    ),
                }
            }
            Work::Verify { action, job } => {
                let observed = evaluation_value(&value)
                    .and_then(|v| Ok(serde_json::from_value::<Option<String>>(v)?));
                let expected = match &action.proposal.intent {
                    Intent::Field { value, .. } => value,
                    _ => return Err("invalid verification intent".into()),
                };
                match observed {
                    Ok(Some(observed)) if &observed == expected => {
                        self.value(job, Outcome::Verified { observed })
                    }
                    _ => self.value(
                        job,
                        Outcome::Uncertain {
                            reason: "field effect could not be verified".into(),
                        },
                    ),
                }
            }
            Work::Navigate { action, old, job } => self.navigated(action, old, job, value),
            Work::FrameTree(nav) => {
                self.navigation_tree(nav, context.ok_or("destination context absent")?, value)
            }
            Work::Cleanup => Ok(()),
        }
    }
    fn failed(&mut self, work: Work, reason: String, prevented: bool) -> Result<()> {
        // A dispatched effect whose reply is lost is uncertain; one stopped before dispatch is
        // prevented. Unverified receipts are always uncertain.
        let lost = |lost: &str| {
            if prevented {
                Outcome::Prevented {
                    reason: reason.clone(),
                }
            } else {
                Outcome::Uncertain {
                    reason: lost.into(),
                }
            }
        };
        let unverified = |why: &str| Outcome::Uncertain { reason: why.into() };
        match work {
            Work::CreateWorld if prevented => Ok(()),
            Work::Init(_) | Work::CreateWorld => {
                Err(format!("browser session initialization failed: {reason}").into())
            }
            Work::Gesture { action, job } => {
                let outcome = lost(
                    if matches!(&action.proposal.intent, Intent::SetNativeControl { .. }) {
                        "native control input result was lost"
                    } else {
                        "button input result was lost"
                    },
                );
                self.value(job, outcome)
            }
            Work::KeyGesture { job, .. } => self.value(job, lost("key input result was lost")),
            Work::WriteSelection { job, .. } => {
                self.value(job, lost("selection dispatch result was lost"))
            }
            Work::WriteScroll { job, .. } => {
                self.value(job, lost("scroll dispatch result was lost"))
            }
            Work::WriteSecret { job, .. } => self.value(job, lost("secret entry result was lost")),
            Work::Write { job, .. } => self.value(job, lost("field dispatch result was lost")),
            Work::ButtonReceipt { job } => self.value(
                job,
                unverified("approved button click could not be verified"),
            ),
            Work::KeyReceipt { job } => {
                self.value(job, unverified("approved key press could not be verified"))
            }
            Work::ControlReceipt { job, .. } => self.value(
                job,
                unverified("native control state could not be verified"),
            ),
            Work::VerifyScroll { job, .. } => {
                self.value(job, unverified("scroll position could not be verified"))
            }
            Work::VerifySelection { job, .. } => self.value(
                job,
                unverified("selection could not be observed after dispatch"),
            ),
            Work::Verify { job, .. } => {
                self.value(job, unverified("field effect could not be verified"))
            }
            Work::ArmKey { job, .. } => self.value(job, Outcome::Prevented { reason }),
            Work::Navigate { job, .. } if prevented => {
                self.value(job, Outcome::Prevented { reason })
            }
            Work::FrameTree(nav) => {
                self.finish(nav.job, Err(reason.into()));
                Ok(())
            }
            Work::Capture { job, .. } | Work::Census { job, .. } => {
                self.abort_observation(job, reason.into())
            }
            Work::ReadField { job, .. }
            | Work::ReadButton { job, .. }
            | Work::ReadControl { job, .. }
            | Work::ReadSelection { job, .. }
            | Work::ReadScroll { job, .. }
            | Work::ArmButton { job, .. }
            | Work::ArmControl { job, .. }
            | Work::Navigate { job, .. } => {
                self.finish(job, Err(reason.into()));
                Ok(())
            }
            Work::Cleanup => Ok(()),
        }
    }
}
fn ref_error(value: &Value) -> Option<&'static str> {
    let description = value["exceptionDetails"]["exception"]["description"].as_str()?;
    [
        "removed snapshot ref",
        "renamed snapshot ref",
        "stale snapshot ref",
    ]
    .into_iter()
    .find(|reason| description.lines().next() == Some(format!("Error: {reason}").as_str()))
}

pub(super) fn evaluation(value: &Value) -> Result<RemoteObject> {
    if value.get("exceptionDetails").is_some() {
        return Err("browser script failed".into());
    }
    Ok(serde_json::from_value(value["result"].clone())?)
}
pub(super) fn evaluation_value(value: &Value) -> Result<Value> {
    let object = evaluation(value)?;
    if object.r#type == RemoteObjectType::Object
        && object.subtype == Some(RemoteObjectSubtype::Null)
    {
        return Ok(Value::Null);
    }
    Ok(chromiumoxide::js::EvaluationResult::new(object).into_value::<Value>()?)
}

#[cfg(test)]
mod checks {
    use super::*;

    #[test]
    fn ref_failure_reports_only_known_reasons_without_page_text() {
        for reason in ["removed", "renamed", "stale"] {
            let value = json!({"exceptionDetails":{"exception":{"description":format!("Error: {reason} snapshot ref\nsecret page stack")}}});
            assert_eq!(
                ref_error(&value),
                Some(format!("{reason} snapshot ref").as_str())
            );
        }
        assert_eq!(
            ref_error(
                &json!({"exceptionDetails":{"exception":{"description":"Error: password=secret"}}})
            ),
            None
        );
    }

    #[test]
    fn unavailable_control_keeps_cdp_null_distinct_from_missing_value() {
        assert_eq!(
            evaluation_value(&json!({"result":{"type":"object","subtype":"null","value":null}}))
                .unwrap(),
            Value::Null
        );
        assert!(evaluation_value(&json!({"result":{"type":"undefined"}})).is_err());
        assert!(evaluation_value(&json!({"result":{"type":"object"}})).is_err());
        assert_eq!(
            evaluation_value(&json!({"result":{"type":"boolean","value":false}})).unwrap(),
            Value::Bool(false)
        );
    }
}
