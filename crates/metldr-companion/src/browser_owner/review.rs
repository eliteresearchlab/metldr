use crate::Result;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

#[derive(Clone)]
pub(crate) struct Budget(Arc<Mutex<Clock>>);

struct Clock {
    limit: Duration,
    spent: Duration,
    phase: Phase,
}

enum Phase {
    Working(Instant),
    Review(Instant),
}

impl Clock {
    fn elapsed(&self) -> Duration {
        self.spent
            + match self.phase {
                Phase::Working(since) => since.elapsed(),
                Phase::Review(_) => Duration::ZERO,
            }
    }
}

impl Budget {
    pub(crate) fn new(limit: Duration) -> Self {
        Self(Arc::new(Mutex::new(Clock {
            limit,
            spent: Duration::ZERO,
            phase: Phase::Working(Instant::now()),
        })))
    }

    pub(crate) fn expired(&self) -> bool {
        self.0
            .lock()
            .map_or(true, |clock| clock.elapsed() >= clock.limit)
    }

    pub(crate) fn review_elapsed(&self) -> Option<Duration> {
        let clock = self.0.lock().ok()?;
        match clock.phase {
            Phase::Review(since) => Some(since.elapsed()),
            Phase::Working(_) => None,
        }
    }

    pub(super) fn pause(&self) -> Result<Review> {
        let mut clock = self.0.lock().map_err(|_| "browser budget poisoned")?;
        let elapsed = clock.elapsed();
        if elapsed >= clock.limit || matches!(clock.phase, Phase::Review(_)) {
            return Err("browser execution budget unavailable".into());
        }
        clock.spent = elapsed;
        clock.phase = Phase::Review(Instant::now());
        Ok(Review(self.clone()))
    }
}

pub(crate) struct Review(Budget);

impl Drop for Review {
    fn drop(&mut self) {
        if let Ok(mut clock) = self.0.0.lock() {
            clock.phase = Phase::Working(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::advance;

    #[tokio::test(start_paused = true)]
    async fn review_preserves_the_remaining_work_budget_and_other_requests_expire() {
        let budget = Budget::new(Duration::from_secs(30));
        advance(Duration::from_secs(12)).await;
        let review = budget.pause().unwrap();
        let queued = Budget::new(Duration::from_secs(30));
        advance(Duration::from_secs(90)).await;
        assert!(budget.review_elapsed().is_some());
        assert!(!budget.expired());
        assert!(queued.expired());
        assert!(queued.pause().is_err());
        drop(review);
        assert!(budget.review_elapsed().is_none());
        advance(Duration::from_secs(17)).await;
        assert!(!budget.expired());
        advance(Duration::from_secs(1)).await;
        assert!(budget.expired());
        assert!(budget.pause().is_err());
    }

    #[test]
    fn failed_review_pause_leaves_no_durable_pending_action() {
        use crate::{
            browser_owner::Event,
            history::browser_checks::{saved, with_owner},
        };
        use std::sync::mpsc;

        with_owner(false, |owner, proposal| {
            let budget = Budget::new(Duration::from_secs(30));
            // Holding the review passes the early expiry check and fails the pause.
            let held = budget.pause().unwrap();
            assert!(!budget.expired());
            let before = saved(owner);
            let (ack, rx) = mpsc::sync_channel(1);
            owner
                .browser_event(Event::Proposed {
                    proposal: proposal.clone(),
                    ack,
                    budget: Some(budget.clone()),
                })
                .unwrap();
            let error = rx.recv().unwrap().unwrap_err();
            assert!(
                error.contains("browser execution budget unavailable"),
                "unexpected rejection: {error}"
            );
            assert!(owner.browser_approval.is_none());
            let after = saved(owner);
            assert!(after.browser_actions.is_empty());
            assert_eq!(after.next, before.next);
            assert_eq!(after.view.sequence, before.view.sequence);

            drop(held);
            let mut fresh = proposal;
            fresh.tool_use_id = "tool-2".into();
            let (ack, rx) = mpsc::sync_channel(1);
            owner
                .browser_event(Event::Proposed {
                    proposal: fresh,
                    ack,
                    budget: Some(budget.clone()),
                })
                .unwrap();
            assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
            assert!(owner.browser_approval.is_some());
            assert_eq!(saved(owner).browser_actions.len(), 1);
            assert!(budget.review_elapsed().is_some());
        });
    }
}
