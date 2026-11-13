//! Session auto-approval (decision 09, B3.2).
//!
//! A grant is opt-in per run, lives only in memory, and is never restored after a restart.
//! It covers Tier 1 steps only: field entry, navigation, scrolling, option selection and native
//! controls. Button activation is Tier 2 and always asks. A new grant is unbound: the first Tier 1
//! step still asks, and approving it binds the grant to that step's browser session and target.
//! The grant ends on Stop, revoke, expiry, a new plan revision or a session or target change.
use crate::browser_action::{Intent, Proposal};
use std::time::{Duration, Instant};

pub(crate) const GRANT_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionGrant {
    pub plan_revision: u32,
    pub expires: Instant,
    pub bound: Option<(String, String)>,
}

impl SessionGrant {
    pub(crate) fn new(plan_revision: u32, now: Instant) -> Self {
        Self {
            plan_revision,
            expires: now + GRANT_TTL,
            bound: None,
        }
    }

    /// Whether `proposal` may be approved without asking, under plan revision `current`.
    pub(crate) fn covers(&self, proposal: &Proposal, current: u32, now: Instant) -> bool {
        now < self.expires
            && self.plan_revision == current
            && tier_one(&proposal.intent)
            && self.bound.as_ref().is_some_and(|(session, target)| {
                *session == proposal.binding.session && *target == proposal.binding.target
            })
    }

    /// Binds an unbound grant to the first manually approved Tier 1 step.
    pub(crate) fn bind(&mut self, proposal: &Proposal, now: Instant) {
        if self.bound.is_none() && now < self.expires && tier_one(&proposal.intent) {
            self.bound = Some((
                proposal.binding.session.clone(),
                proposal.binding.target.clone(),
            ));
        }
    }
}

/// Tier 1 kinds from decision 09. Anything that can submit, send, delete or buy is Tier 2.
/// A secret entry always needs its own consent, so no grant covers it.
pub(crate) fn tier_one(intent: &Intent) -> bool {
    match intent {
        Intent::Field { .. }
        | Intent::Navigate { .. }
        | Intent::SelectOption { .. }
        | Intent::Scroll { .. }
        | Intent::SetNativeControl { .. } => true,
        // Enter can submit a form, so only the other keys are Tier 1.
        Intent::PressKey { key, .. } => key != "Enter",
        Intent::ActivateButton { .. } | Intent::Secret { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_action::Binding;

    fn proposal(session: &str, intent: Intent) -> Proposal {
        Proposal {
            run: "run".into(),
            tool_use_id: "tool".into(),
            binding: Binding {
                tab: 1,
                document: "document".into(),
                target: "target".into(),
                frame: "frame".into(),
                session: session.into(),
                unique_context: "context".into(),
                grant_revision: 1,
            },
            url: "http://127.0.0.1/".into(),
            intent,
        }
    }

    fn navigate() -> Intent {
        Intent::Navigate {
            destination: "http://127.0.0.1/next".into(),
        }
    }

    #[test]
    fn grant_covers_only_bound_unexpired_tier_one_steps_of_its_revision() {
        let now = Instant::now();
        let mut grant = SessionGrant::new(2, now);
        let step = proposal("session-1", navigate());
        assert!(!grant.covers(&step, 2, now), "an unbound grant asks first");
        grant.bind(&step, now);
        assert!(grant.covers(&step, 2, now));
        assert!(
            !grant.covers(&step, 3, now),
            "a new plan revision ends the grant"
        );
        assert!(!grant.covers(&proposal("session-2", navigate()), 2, now));
        let button = proposal(
            "session-1",
            Intent::ActivateButton {
                button_id: "b".into(),
                button_label: "Buy".into(),
            },
        );
        assert!(!grant.covers(&button, 2, now));
        let secret = proposal(
            "session-1",
            Intent::Secret {
                field_id: "field_1".into(),
                field_label: "Password".into(),
                secret: crate::browser_action::SecretKind::Password,
                reference: "0".repeat(32),
                expires_at_ms: 1,
            },
        );
        assert!(!grant.covers(&secret, 2, now), "a secret needs its own consent");
        let key = |key: &str| {
            proposal(
                "session-1",
                Intent::PressKey {
                    field_id: "field_1".into(),
                    field_label: "Search".into(),
                    key: key.into(),
                },
            )
        };
        assert!(grant.covers(&key("Tab"), 2, now));
        assert!(!grant.covers(&key("Enter"), 2, now), "Enter can submit, so it always asks");
        assert!(!grant.covers(&step, 2, now + GRANT_TTL));
    }
}
