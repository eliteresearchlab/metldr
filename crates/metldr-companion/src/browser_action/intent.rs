use super::{Binding, Control, Proposal, Scroll, Selection, bounded};
use crate::Result;
use metldr_browser_tools::valid_handle;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Intent {
    Field {
        field_id: String,
        field_label: String,
        previous_value: String,
        value: String,
    },
    Navigate {
        destination: String,
    },
    ActivateButton {
        button_id: String,
        button_label: String,
    },
    SelectOption {
        selection: Selection,
    },
    Scroll {
        scroll: Scroll,
    },
    SetNativeControl {
        control: Control,
    },
    /// One consented secret entry. The record keeps an opaque reference, never the secret.
    Secret {
        field_id: String,
        field_label: String,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireProposal {
    run: String,
    tool_use_id: String,
    binding: Binding,
    url: String,
    intent: Option<Intent>,
    field_id: Option<String>,
    field_label: Option<String>,
    previous_value: Option<String>,
    value: Option<String>,
}

impl TryFrom<WireProposal> for Proposal {
    type Error = String;

    fn try_from(wire: WireProposal) -> std::result::Result<Self, String> {
        let legacy = wire.field_id.is_some()
            || wire.field_label.is_some()
            || wire.previous_value.is_some()
            || wire.value.is_some();
        let intent = match wire.intent {
            Some(intent) if !legacy => intent,
            Some(_) => return Err("mixed browser proposal formats".into()),
            None => Intent::Field {
                field_id: wire.field_id.ok_or("missing field handle")?,
                field_label: wire.field_label.unwrap_or_else(|| "Text field".into()),
                previous_value: wire.previous_value.ok_or("missing previous value")?,
                value: wire.value.ok_or("missing field value")?,
            },
        };
        Ok(Proposal {
            run: wire.run,
            tool_use_id: wire.tool_use_id,
            binding: wire.binding,
            url: wire.url,
            intent,
        })
    }
}

impl Intent {
    pub(super) fn validate(&self) -> Result<()> {
        match self {
            Self::Field {
                field_id,
                field_label,
                previous_value,
                value,
            } if valid_handle(field_id)
                && bounded(field_label, 128)
                && previous_value.len() <= 1024
                && value.len() <= 1024 =>
            {
                Ok(())
            }
            Self::Navigate { destination }
                if crate::model::validate_page_url(destination).is_ok() =>
            {
                Ok(())
            }
            Self::ActivateButton {
                button_id,
                button_label,
            } if valid_handle(button_id) && bounded(button_label, 128) => Ok(()),
            Self::SelectOption { selection } if selection.valid() => Ok(()),
            Self::Scroll { scroll } if scroll.valid() => Ok(()),
            Self::SetNativeControl { control } if control.valid() => Ok(()),
            Self::Secret {
                field_id,
                field_label,
                reference,
                expires_at_ms,
                ..
            } if valid_handle(field_id)
                && bounded(field_label, 128)
                && valid_reference(reference)
                && *expires_at_ms > 0 =>
            {
                Ok(())
            }
            _ => Err("invalid browser action intent".into()),
        }
    }
}
