//! Hosted decisions for the browser tools: when the user has a System One connection, its model
//! reorders the element scout's shortlist; otherwise, and on any failure, the scout's own order
//! stands. Only the step's goal, the recent actions and the shortlisted control lines are sent.

use crate::https_profile::Profile;
use serde_json::json;
use std::sync::{Mutex, atomic::AtomicBool};

static ROUTE: Mutex<Option<Profile>> = Mutex::new(None);

/// Sets or clears the connection the browser tools decide with.
pub(crate) fn publish(profile: Option<Profile>) {
    *ROUTE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = profile;
}

fn route() -> Option<Profile> {
    ROUTE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// `lines` reordered by the hosted model's probability for each, most likely first, with its
/// confidence; `None` without a connection or when the request fails.
pub(crate) async fn rerank<'a>(
    goal: &str,
    previous: &[&str],
    lines: &[&'a str],
) -> Option<(Vec<&'a str>, Option<f32>)> {
    let profile = route()?;
    if lines.len() < 2 {
        return None;
    }
    let client = profile.system_one_client().ok()?;
    let state = json!({"goal": goal, "recent actions, newest first": previous});
    let instructions = "Which page control should the browser agent act on next to reach the goal?";
    let stop = AtomicBool::new(false);
    let choice = client.choose(&state, instructions, lines, &stop).await.ok()?;
    let mut order: Vec<usize> = (0..lines.len()).collect();
    // A stable sort keeps the scout's order among options the model rates the same.
    order.sort_by(|&a, &b| choice.probabilities[b].total_cmp(&choice.probabilities[a]));
    Some((order.into_iter().map(|n| lines[n]).collect(), choice.confidence))
}
