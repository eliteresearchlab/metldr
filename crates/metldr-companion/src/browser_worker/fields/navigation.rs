use super::*;

impl Tools {
    pub(super) async fn navigate(
        &mut self,
        tool_use_id: &str,
        destination: &str,
        stop: &AtomicBool,
        budget: Option<&browser_owner::Budget>,
    ) -> Result<Value> {
        if crate::model::validate_page_url(destination).is_err() {
            return Err("invalid browser navigation URL".into());
        }
        let proposal = self
            .engine
            .as_ref()
            .ok_or("browser is not attached")?
            .prepare_navigation(tool_use_id, destination)
            .await?;
        let approved = action(&self.client, Some(stop), |ack| {
            browser_owner::Event::Proposed {
                proposal,
                ack,
                budget: budget.cloned(),
            }
        })
        .await?;
        let settled = match &approved.state {
            Phase::Approved => {
                let dispatched = action(&self.client, Some(stop), |ack| {
                    browser_owner::Event::Dispatch {
                        action: approved.id.clone(),
                        revision: approved.revision,
                        ack,
                    }
                })
                .await?;
                let outcome = self
                    .engine
                    .as_ref()
                    .ok_or("browser is not attached")?
                    .navigate(&dispatched)
                    .await;
                action_observed(&self.client, &dispatched, outcome).await?
            }
            Phase::Settled { .. } => approved,
            _ => return Err("browser navigation was not approved or settled".into()),
        };
        match settled.state {
            Phase::Settled {
                outcome:
                    Outcome::Navigated {
                        binding,
                        url,
                        evidence,
                        ..
                    },
            } => {
                self.current.document = binding.document;
                self.current.url = url;
                let mut observation = self.attached()?.snapshot(json!({})).await?;
                observation["url"] = json!(self.current.url);
                if !evidence.is_empty() {
                    let mut report = serde_json::to_value(&evidence)?;
                    // A 2xx page headed like an error is only a labelled suspicion, never a failure.
                    if matches!(evidence.status, Some(200..=299))
                        && observation["text"]
                            .as_str()
                            .and_then(first_heading)
                            .is_some_and(soft_404_title)
                    {
                        report["soft_404_suspected"] = true.into();
                    }
                    observation["navigation"] = report;
                }
                Ok(observation)
            }
            Phase::Settled {
                outcome: outcome @ Outcome::Prevented { .. },
            } => Ok(serde_json::to_value(outcome)?),
            _ => Err("navigation effect is uncertain; automatic continuation stopped".into()),
        }
    }
}

/// The name of the first heading in a snapshot, e.g. `- heading "Not found" [level=1]`.
fn first_heading(text: &str) -> Option<&str> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("- heading \""))?;
    let start = line.find('"')? + 1;
    let end = start + line[start..].find('"')?;
    Some(&line[start..end])
}

/// A page title or heading that usually means a missing page served with a success status.
fn soft_404_title(title: &str) -> bool {
    let title = title.to_ascii_lowercase();
    title.starts_with("404")
        || [
            "not found",
            "page does not exist",
            "page doesn't exist",
            "no longer available",
        ]
        .iter()
        .any(|phrase| title.contains(phrase))
}

#[cfg(test)]
mod tests {
    #[test]
    fn soft_404_titles_are_only_error_shaped_titles() {
        for title in [
            "404 - Missing",
            "Page Not Found | Shop",
            "This page doesn't exist",
        ] {
            assert!(super::soft_404_title(title), "{title}");
        }
        for title in [
            "Example Domain",
            "Found a bug? Report it",
            "Error handling guide",
        ] {
            assert!(!super::soft_404_title(title), "{title}");
        }
    }
}
