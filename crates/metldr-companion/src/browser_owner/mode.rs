//! The chat's approval mode (BR2b-2), chosen by the user and saved with the chat.
//!
//! Ask: the plan and every step ask. Auto: the plan asks; then steps that cannot submit, send,
//! buy, pay or delete run without asking. Full access: nothing asks. In every mode a secret entry
//! needs its own consent and Stop works.
use crate::browser_action::Intent;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalMode {
    #[default]
    Ask,
    Auto,
    Full,
}

impl ApprovalMode {
    pub(crate) fn byte(self) -> u8 {
        self as u8
    }

    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        [Self::Ask, Self::Auto, Self::Full].get(usize::from(byte)).copied()
    }

    /// Whether a proposed plan is approved without asking.
    pub(crate) fn approves_plans(self) -> bool {
        self == Self::Full
    }

    /// Whether a step under an approved plan runs without asking.
    pub(crate) fn approves(self, intent: &Intent) -> bool {
        match self {
            Self::Ask => false,
            Self::Auto => safe(intent),
            Self::Full => !matches!(intent, Intent::Secret { .. }),
        }
    }
}

/// Words in a target's name that mark a click as one that can submit, send, buy, pay or delete.
const RISKY: [&str; 12] = [
    "submit", "send", "buy", "purchase", "checkout", "order", "pay", "delete", "remove", "confirm",
    "transfer", "publish",
];

/// A step Auto runs without asking: it cannot submit, send, buy, pay or delete.
fn safe(intent: &Intent) -> bool {
    match intent {
        Intent::Navigate { .. } => true,
        Intent::Act { tool, args, elements } => match tool.as_str() {
            "fill" | "type" => args["submit"] != true,
            "press" => !pressed(&args["key"]).any(|key| key.ends_with("Enter")),
            "select" | "hover" | "scroll" => true,
            "click" | "drag" => !elements.iter().any(|element| {
                let name = element.name.to_lowercase();
                name.split(|c: char| !c.is_alphanumeric()).any(|word| RISKY.contains(&word))
            }),
            _ => false,
        },
        Intent::Secret { .. } => false,
    }
}

/// The keys of a press, one or a list.
fn pressed(key: &serde_json::Value) -> impl Iterator<Item = &str> {
    key.as_str()
        .into_iter()
        .chain(key.as_array().into_iter().flatten().filter_map(|k| k.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_mode_asks_for_what_it_should() {
        let click = |name| Intent::act("click", json!({"ref":"e1"}), &[("e1", "button", name)]);
        let secret = Intent::Secret {
            element: crate::browser_action::element("e2", "textbox", "Password"),
            secret: crate::browser_action::SecretKind::Password,
            reference: "0".repeat(32),
            expires_at_ms: 1,
        };
        let press = |key| Intent::act("press", json!({"key":key}), &[]);
        let fill = |submit| {
            let args = json!({"ref":"e3","value":"x","submit":submit});
            Intent::act("fill", args, &[("e3", "textbox", "Search")])
        };
        for intent in [click("Apply update"), fill(false), press(json!("Tab"))] {
            assert!(!ApprovalMode::Ask.approves(&intent));
            assert!(ApprovalMode::Auto.approves(&intent));
            assert!(ApprovalMode::Full.approves(&intent));
        }
        let risky = [click("Place order"), click("Delete account"), fill(true), press(json!(["a", "Shift+Enter"]))];
        for intent in risky {
            assert!(!ApprovalMode::Auto.approves(&intent), "{intent:?} can submit or delete");
            assert!(ApprovalMode::Full.approves(&intent));
        }
        for mode in [ApprovalMode::Ask, ApprovalMode::Auto, ApprovalMode::Full] {
            assert!(!mode.approves(&secret), "a secret needs its own consent");
            assert_eq!(ApprovalMode::from_byte(mode.byte()), Some(mode));
        }
        assert!(ApprovalMode::Full.approves_plans() && !ApprovalMode::Auto.approves_plans());
    }
}
