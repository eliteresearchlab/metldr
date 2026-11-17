use super::bounded;
use crate::Result;
use metldr_browser_tools::{ActTool, Call, valid_ref};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An element a step acts on, named as the latest snapshot showed it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Element {
    pub reference: String,
    pub role: String,
    pub name: String,
}

impl Element {
    fn valid(&self) -> bool {
        valid_ref(&self.reference)
            && bounded(&self.role, 64)
            && self.name.len() <= 256
            && !self.name.chars().any(char::is_control)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Intent {
    Navigate {
        destination: String,
    },
    /// A page action run with the browser's trusted input. `args` are the tool's checked
    /// arguments; `elements` are the refs they name, in the order they name them.
    Act {
        tool: String,
        args: Value,
        elements: Vec<Element>,
    },
    /// One consented secret entry. The record keeps an opaque reference, never the secret.
    Secret {
        element: Element,
        secret: SecretKind,
        reference: String,
        expires_at_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SecretKind {
    Password,
    OneTimeCode,
}

impl From<metldr_browser_tools::SecretKind> for SecretKind {
    fn from(kind: metldr_browser_tools::SecretKind) -> Self {
        match kind {
            metldr_browser_tools::SecretKind::Password => Self::Password,
            metldr_browser_tools::SecretKind::OneTimeCode => Self::OneTimeCode,
        }
    }
}

/// How long the user has to type and enter a requested secret.
pub(crate) const SECRET_TTL_MS: u64 = 120_000;

pub(crate) fn valid_reference(reference: &str) -> bool {
    reference.len() == 32
        && reference
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The refs a page action's arguments name, in order: `ref`, `from`, `to`, then each field's.
pub(crate) fn named_refs(args: &Value) -> Vec<&str> {
    let mut refs: Vec<&str> = ["ref", "from", "to"]
        .into_iter()
        .filter_map(|key| args[key].as_str())
        .collect();
    if let Some(fields) = args["fields"].as_array() {
        refs.extend(fields.iter().filter_map(|field| field["ref"].as_str()));
    }
    refs
}

impl Intent {
    pub(super) fn validate(&self) -> Result<()> {
        match self {
            Self::Navigate { destination }
                if crate::model::validate_page_url(destination).is_ok() =>
            {
                Ok(())
            }
            Self::Act {
                tool,
                args,
                elements,
            } if ActTool::parse(tool).is_some()
                && matches!(
                    metldr_browser_tools::decode_call(tool, args.clone()),
                    Ok(Call::Act { .. })
                )
                && elements.len() <= 33
                && elements.iter().all(Element::valid)
                && elements
                    .iter()
                    .map(|element| element.reference.as_str())
                    .eq(named_refs(args)) =>
            {
                Ok(())
            }
            Self::Secret {
                element,
                reference,
                expires_at_ms,
                ..
            } if element.valid() && valid_reference(reference) && *expires_at_ms > 0 => Ok(()),
            _ => Err("invalid browser action intent".into()),
        }
    }
}

#[cfg(test)]
impl Intent {
    /// A page action on `elements` (ref, role, name), for checks.
    pub(crate) fn act(tool: &str, args: Value, elements: &[(&str, &str, &str)]) -> Self {
        Self::Act {
            tool: tool.into(),
            args,
            elements: elements
                .iter()
                .map(|(r, role, name)| element(r, role, name))
                .collect(),
        }
    }
}

#[cfg(test)]
pub(crate) fn element(reference: &str, role: &str, name: &str) -> Element {
    Element {
        reference: reference.into(),
        role: role.into(),
        name: name.into(),
    }
}
