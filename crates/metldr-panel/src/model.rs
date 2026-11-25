use crate::MAX_FRAME;
use std::{fmt, num::NonZeroU64};

use metldr_conversation::{Display, ProjectedEntry};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[path = "model/run.rs"]
mod run;
#[path = "model/source.rs"]
mod source;

const MAX_RUNS: usize = 16;
pub(crate) const MAX_ACTIVITY: usize = 64;
const MAX_BROWSER_ACTIONS: usize = 16;
const MAX_PAGE_TEXT: usize = 8_192;
/// The largest page action arguments a view carries: 32 fields of 8 KB text.
const MAX_ACT_ARGS: usize = 300_000;
const MAX_PI_UI: usize = 32_768;
const MAX_CORRECTIONS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RunId(String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceId(String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BrowserActionId(String);

macro_rules! identity {
    ($type:ty) => {
        impl $type {
            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Serialize for $type {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                parse_identity(&value).map_err(serde::de::Error::custom)?;
                Ok(Self(value))
            }
        }
    };
}

identity!(RunId);
identity!(SourceId);
identity!(BrowserActionId);

fn parse_identity(value: &str) -> Result<(&str, NonZeroU64), String> {
    if value.len() > 49_152 {
        return Err("identity exceeds its saved record bound".to_owned());
    }
    let (store, local) = value
        .split_once(':')
        .ok_or_else(|| "identity has no store separator".to_owned())?;
    if store.len() != 32 || !store.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid store identity".into());
    }
    let local = local
        .parse::<u64>()
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or_else(|| "invalid local identity".to_owned())?;
    Ok((store, local))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UserText(String);

impl TryFrom<&str> for UserText {
    type Error = FlowError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.trim().is_empty() || value.len() > 1_024 {
            return Err(FlowError::InvalidInput(
                "text must be nonempty and at most 1024 UTF-8 bytes".into(),
            ));
        }
        Ok(Self(value.into()))
    }
}

impl UserText {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunStatus {
    Running,
    StopRequested,
    Complete,
    Cancelled,
    Failed,
    Interrupted,
}

impl RunStatus {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Cancelled | Self::Failed | Self::Interrupted
        )
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Running => "running",
            Self::StopRequested => "stop_requested",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunKind {
    Summary,
    Answer,
    Agent,
    Browser,
}

impl fmt::Display for RunKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Summary => "summary",
            Self::Answer => "answer",
            Self::Agent => "agent",
            Self::Browser => "browser",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageView {
    pub(crate) sequence: u64,
    pub(crate) fixture_available: bool,
    pub(crate) store: String,
    pub(crate) selection: metldr_history_pages::Scope,
    pub(crate) source: Option<SourceView>,
    pub(crate) runs: Vec<RunView>,
    pub(crate) native_correction: Option<NativeCorrectionView>,
    pub(crate) pi_profile: Option<PiProfileView>,
    pub(crate) agent_sessions: Vec<AgentSessionView>,
    pub(crate) codex_profile: Option<CodexProfileView>,
    pub(crate) codex_sessions: Vec<AgentSessionView>,
    pub(crate) opencode_profile: Option<CodexProfileView>,
    pub(crate) opencode_sessions: Vec<AgentSessionView>,
    pub(crate) claude_sessions: Vec<AgentSessionView>,
    pub(crate) native_permissions: Vec<NativePermissionView>,
    pub(crate) pi_ui: Option<PiUiView>,
    pub(crate) codex_input: Option<CodexInputView>,
    pub(crate) browser_actions: Vec<BrowserActionView>,
    pub(crate) task_plan: Option<TaskPlanView>,
    pub(crate) approval_mode: ApprovalMode,
    pub(crate) browser_route: Option<BrowserRouteView>,
    pub(crate) chat_route: Option<ChatRouteView>,
    pub(crate) responses_route: Option<ResponsesRouteView>,
    pub(crate) google_route: Option<GoogleRouteView>,
    pub(crate) provider_settings: ProviderSettingsView,
    pub(crate) agent_settings: AgentSettingsView,
    pub(crate) source_status: SourceStatus,
    pub(crate) source_epoch: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskPlanView {
    #[serde(default)]
    pub(crate) proposer: String,
    pub(crate) run: RunId,
    pub(crate) goal: String,
    pub(crate) intended_effects: Vec<String>,
    pub(crate) scope: String,
    pub(crate) revision: u32,
    pub(crate) state: PlanState,
}

/// What a chat's browser runs may do without asking (BR2b-2). Ask: the plan and every step
/// ask. Auto: the plan asks, then steps that cannot submit, send, buy, pay or delete run. Full
/// access: nothing asks. A secret entry always asks.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalMode {
    Ask,
    Auto,
    Full,
}

#[cfg(feature = "rune-ui")]
impl ApprovalMode {
    pub(crate) const ALL: [Self; 3] = [Self::Ask, Self::Auto, Self::Full];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ask => "Ask",
            Self::Auto => "Auto",
            Self::Full => "Full access",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlanState {
    Proposed,
    Approved,
    Declined,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderSettingsView {
    pub(crate) busy: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) pending: Option<Protocol>,
    pub(crate) cleanup_pending: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) notice: Option<String>,
    pub(crate) connections: Vec<ProviderConnectionView>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentSettingsView {
    pub(crate) busy: bool,
    pub(crate) agents: Vec<AgentSettingView>,
    #[serde(default)]
    pub(crate) apps: Vec<DesktopAppView>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentSettingView {
    pub(crate) agent: AgentKind,
    pub(crate) revision: u64,
    pub(crate) state: AgentSettingState,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) profile: Option<AgentProfile>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) last_request: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) last_digest: Option<String>,
    /// The executable the companion found for this agent, before any setup.
    #[serde(default)]
    pub(crate) found: Option<String>,
}

/// A desktop app that can host the MetLDR MCP server, and its config file when present.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopAppView {
    pub(crate) app: String,
    pub(crate) label: String,
    pub(crate) config: Option<String>,
    pub(crate) local_config: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentKind {
    Pi,
    Codex,
    Opencode,
    Claude,
}

impl AgentKind {
    pub(crate) const ALL: [Self; 4] = [Self::Codex, Self::Opencode, Self::Claude, Self::Pi];
    pub(crate) fn harness(self) -> metldr_harness::HarnessId {
        match self {
            Self::Pi => metldr_harness::HarnessId::Pi,
            Self::Codex => metldr_harness::HarnessId::Codex,
            Self::Opencode => metldr_harness::HarnessId::OpenCode,
            Self::Claude => metldr_harness::HarnessId::ClaudeCode,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        self.harness().key()
    }
    pub(crate) fn label(self) -> &'static str {
        self.harness().label()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentSettingState {
    Unmanaged,
    Ready,
    Removed,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AgentProfile {
    Pi {
        executable: String,
        workspace: String,
        session_dir: String,
        provider: String,
        model: String,
        offline: bool,
    },
    Codex {
        executable: String,
        workspace: String,
    },
    Opencode {
        executable: String,
        workspace: String,
    },
    Claude {
        executable: String,
        workspace: String,
    },
}

impl AgentProfile {
    pub(crate) fn agent(&self) -> AgentKind {
        match self {
            Self::Pi { .. } => AgentKind::Pi,
            Self::Codex { .. } => AgentKind::Codex,
            Self::Opencode { .. } => AgentKind::Opencode,
            Self::Claude { .. } => AgentKind::Claude,
        }
    }
}

fn valid_agent_path(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 2_048 && !value.chars().any(char::is_control)
}

fn valid_agent_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

pub(crate) fn valid_agent_profile(profile: &AgentProfile) -> bool {
    match profile {
        AgentProfile::Pi {
            executable,
            workspace,
            session_dir,
            provider,
            model,
            ..
        } => {
            valid_agent_path(executable)
                && valid_agent_path(workspace)
                && valid_agent_path(session_dir)
                && valid_agent_label(provider)
                && valid_agent_label(model)
        }
        AgentProfile::Codex {
            executable,
            workspace,
        }
        | AgentProfile::Opencode {
            executable,
            workspace,
        }
        | AgentProfile::Claude {
            executable,
            workspace,
        } => valid_agent_path(executable) && valid_agent_path(workspace),
    }
}

/// One wire protocol list for the whole app: the gateway's.
pub(crate) use metldr_core::gateway::Protocol;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConnectionView {
    pub(crate) protocol: Protocol,
    pub(crate) revision: u64,
    pub(crate) state: ProviderConnectionState,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) endpoint: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) model: Option<String>,
    /// Chat Completions only, absent from older companions.
    #[serde(default)]
    pub(crate) keyless: bool,
    #[serde(default)]
    pub(crate) timeout_secs: Option<u16>,
}

/// How a provider save treats the API key.
pub(crate) enum ProviderKey {
    Keep,
    Replace(String),
    /// Chat Completions only: the endpoint runs without a key.
    None,
}

/// Shortest and longest response timeout, matching the companion's chat client.
pub(crate) const PROVIDER_TIMEOUT_SECS: std::ops::RangeInclusive<u16> = 5..=600;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderConnectionState {
    Unmanaged,
    Removed,
    Loading,
    Ready,
    Missing,
    Locked,
    Corrupt,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BrowserActionView {
    pub(crate) id: BrowserActionId,
    pub(crate) revision: u64,
    pub(crate) run: RunId,
    pub(crate) tab: i32,
    pub(crate) document: String,
    pub(crate) url: String,
    pub(crate) intent: BrowserActionIntent,
    pub(crate) phase: BrowserActionPhase,
    pub(crate) outcome: Option<BrowserActionOutcome>,
    pub(crate) reason: Option<String>,
    /// The browser's short account of what a settled page action changed.
    pub(crate) result: Option<String>,
    pub(crate) target_current: bool,
}

/// An element a step acts on, named as the latest snapshot showed it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserElement {
    pub(crate) reference: String,
    pub(crate) role: String,
    pub(crate) name: String,
}

impl BrowserElement {
    fn valid(&self) -> bool {
        (1..=16).contains(&self.reference.len())
            && self
                .reference
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && valid_selection_label(&self.role)
            && self.role.len() <= 64
            && self.name.len() <= 256
            && !self.name.chars().any(char::is_control)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BrowserActionIntent {
    Navigate {
        destination: String,
    },
    /// A page action run with the browser's trusted input: the tool, its checked arguments and
    /// the elements they name.
    Act {
        tool: String,
        args: serde_json::Value,
        elements: Vec<BrowserElement>,
    },
    /// A consented secret entry (B3.5). The view never carries the secret itself.
    EnterSecret {
        element: BrowserElement,
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

impl SecretKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Password => "Password",
            Self::OneTimeCode => "One-time code",
        }
    }
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBrowserAction {
    id: BrowserActionId,
    revision: u64,
    run: RunId,
    tab: i32,
    document: String,
    url: String,
    kind: String,
    #[serde(default, deserialize_with = "present")]
    destination: Option<String>,
    #[serde(default, deserialize_with = "present")]
    tool: Option<String>,
    #[serde(default, deserialize_with = "present")]
    args: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    elements: Option<Vec<BrowserElement>>,
    #[serde(default, deserialize_with = "present")]
    element: Option<BrowserElement>,
    #[serde(default, deserialize_with = "present")]
    secret: Option<SecretKind>,
    #[serde(default, deserialize_with = "present")]
    reference: Option<String>,
    #[serde(default, deserialize_with = "present")]
    expires_at_ms: Option<u64>,
    phase: BrowserActionPhase,
    #[serde(default, deserialize_with = "Option::deserialize")]
    outcome: Option<BrowserActionOutcome>,
    #[serde(default, deserialize_with = "Option::deserialize")]
    reason: Option<String>,
    #[serde(default, deserialize_with = "present")]
    result: Option<String>,
}

impl<'de> Deserialize<'de> for BrowserActionView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireBrowserAction::deserialize(deserializer)?;
        let invalid = || serde::de::Error::custom("invalid browser action intent");
        let act = wire.tool.is_some() || wire.args.is_some() || wire.elements.is_some();
        let secret = wire.element.is_some()
            || wire.secret.is_some()
            || wire.reference.is_some()
            || wire.expires_at_ms.is_some();
        let intent = match wire.kind.as_str() {
            "navigate" if !act && !secret => BrowserActionIntent::Navigate {
                destination: wire.destination.ok_or_else(invalid)?,
            },
            "act" if !secret && wire.destination.is_none() => BrowserActionIntent::Act {
                tool: wire.tool.ok_or_else(invalid)?,
                args: wire.args.ok_or_else(invalid)?,
                elements: wire.elements.ok_or_else(invalid)?,
            },
            "enter_secret" if !act && wire.destination.is_none() => {
                BrowserActionIntent::EnterSecret {
                    element: wire.element.ok_or_else(invalid)?,
                    secret: wire.secret.ok_or_else(invalid)?,
                    reference: wire.reference.ok_or_else(invalid)?,
                    expires_at_ms: wire.expires_at_ms.ok_or_else(invalid)?,
                }
            }
            _ => return Err(invalid()),
        };
        Ok(Self {
            id: wire.id,
            revision: wire.revision,
            run: wire.run,
            tab: wire.tab,
            document: wire.document,
            url: wire.url,
            intent,
            phase: wire.phase,
            outcome: wire.outcome,
            reason: wire.reason,
            result: wire.result,
            target_current: false,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserActionPhase {
    Pending,
    Approved,
    Dispatching,
    Settled,
}

impl std::fmt::Display for BrowserActionPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Dispatching => "dispatching",
            Self::Settled => "settled",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserActionOutcome {
    Verified,
    Acted,
    SecretEntered,
    Prevented,
    Uncertain,
}

impl std::fmt::Display for BrowserActionOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Verified => "verified",
            Self::Acted => "acted",
            Self::SecretEntered => "secret_entered",
            Self::Prevented => "prevented",
            Self::Uncertain => "uncertain",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceStatus {
    Checking,
    Current,
    Changed,
    Saved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceView {
    pub(crate) id: SourceId,
    pub(crate) tab: i32,
    pub(crate) document: String,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunView {
    pub(crate) id: RunId,
    pub(crate) source: SourceId,
    pub(crate) kind: RunKind,
    pub(crate) question: Option<String>,
    pub(crate) status: RunStatus,
    pub(crate) output: Display,
    pub(crate) error: Option<String>,
    pub(crate) route: RunRoute,
    pub(crate) activity: Vec<ActivityView>,
    pub(crate) corrections: CorrectionHistory,
    pub(crate) ledger: Option<metldr_conversation::Ledger>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCorrectionView {
    pub(crate) run: RunId,
    pub(crate) available: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorrectionHistory {
    pub(crate) total: u8,
    pub(crate) entries: Vec<CorrectionReceipt>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorrectionReceipt {
    pub(crate) ordinal: u8,
    pub(crate) request: String,
    pub(crate) text: crate::history::Preview,
    pub(crate) state: CorrectionState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CorrectionState {
    Pending,
    Dispatching,
    CodexAccepted,
    PiQueued,
    PiHandled,
    Rejected { message: String },
    NotSent { message: String },
    Uncertain { message: String },
}

impl CorrectionState {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Pending => "Saved and waiting to send".into(),
            Self::Dispatching => "Sending to native session".into(),
            Self::CodexAccepted => {
                "Codex accepted this correction; model use is not confirmed".into()
            }
            Self::PiQueued => "Pi queued this correction; model use is not confirmed".into(),
            Self::PiHandled => "Pi handled this correction; model use is not confirmed".into(),
            Self::Rejected { message } => format!("Rejected: {message}"),
            Self::NotSent { message } => format!("Not sent: {message}"),
            Self::Uncertain { message } => format!("Uncertain: {message}"),
        }
    }
}

pub(crate) fn valid_correction_receipt(
    receipt: &CorrectionReceipt,
    max_text: usize,
    require_complete_text: bool,
) -> bool {
    !receipt.request.is_empty()
        && receipt.request.len() <= 96
        && receipt
            .request
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
        && !receipt.text.text.trim().is_empty()
        && receipt.text.text.len() <= max_text
        && (!require_complete_text || !receipt.text.truncated)
        && !matches!(
            &receipt.state,
            CorrectionState::Rejected { message }
                | CorrectionState::NotSent { message }
                | CorrectionState::Uncertain { message }
                if message.trim().is_empty() || message.len() > 256
        )
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RunRoute {
    LegacyUnspecified,
    Fixture,
    Messages {
        model: String,
    },
    HttpsMessages {
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
    HttpsChatCompletions {
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
    HttpsResponses {
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
    HttpsGoogleInteractions {
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
    Text {
        protocol: Protocol,
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
    Pi {
        workflow: RunId,
        provider: String,
        model: String,
    },
    Codex {
        workflow: RunId,
        provider: Option<String>,
        model: Option<String>,
    },
    #[serde(rename = "opencode")]
    OpenCode {
        workflow: RunId,
        model: Option<String>,
        mode: Option<String>,
    },
    /// A harness on the generic path, by registry key.
    Harness {
        harness: String,
        workflow: RunId,
    },
}

impl RunRoute {
    /// The model the route answered with, as Stats counts it.
    pub(crate) fn model(&self) -> &str {
        match self {
            Self::Messages { model }
            | Self::HttpsMessages { model, .. }
            | Self::HttpsChatCompletions { model, .. }
            | Self::HttpsResponses { model, .. }
            | Self::HttpsGoogleInteractions { model, .. }
            | Self::Text { model, .. }
            | Self::Pi { model, .. } => model,
            Self::Codex { model, .. } | Self::OpenCode { model, .. } => {
                model.as_deref().unwrap_or_default()
            }
            Self::Harness { harness, .. } => harness,
            Self::LegacyUnspecified | Self::Fixture => "",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum NativePermissionAgent {
    Codex,
    Opencode,
    Claude,
}

impl NativePermissionAgent {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Opencode => "OpenCode",
            Self::Claude => "Claude Code",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativePermissionView {
    pub(crate) run: RunId,
    pub(crate) request: RunId,
    pub(crate) agent: NativePermissionAgent,
    pub(crate) tool: String,
    pub(crate) title: String,
    pub(crate) input: String,
    pub(crate) options: Vec<NativePermissionOption>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativePermissionOption {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PiUiView {
    pub(crate) run: RunId,
    pub(crate) pending: Vec<PiUiPending>,
    pub(crate) title: Option<String>,
    pub(crate) statuses: Vec<PiUiStatus>,
    pub(crate) widgets: Vec<PiUiWidget>,
    #[serde(default)]
    pub(crate) editor: Option<PiUiEditor>,
    pub(crate) notices: Vec<PiUiNotice>,
    #[serde(default)]
    pub(crate) notices_omitted: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PiUiPending {
    pub(crate) request: RunId,
    pub(crate) dialog: PiUiDialog,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PiUiDialog {
    Select {
        title: String,
        options: Vec<String>,
    },
    Confirm {
        title: String,
        message: String,
    },
    Input {
        title: String,
        #[serde(default)]
        placeholder: Option<String>,
    },
    Editor {
        title: String,
        #[serde(default)]
        prefill: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PiUiStatus {
    pub(crate) key: String,
    pub(crate) text: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PiUiWidget {
    pub(crate) key: String,
    pub(crate) lines: Option<Vec<String>>,
    pub(crate) placement: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PiUiEditor {
    pub(crate) revision: u64,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PiUiNotice {
    pub(crate) level: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexInputView {
    pub(crate) run: RunId,
    pub(crate) pending: Vec<CodexInputPending>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexInputPending {
    pub(crate) request: RunId,
    pub(crate) prompt: CodexInputPrompt,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CodexInputPrompt {
    Questions {
        item: String,
        is_blocking: bool,
        questions: Vec<CodexQuestion>,
    },
    McpForm {
        server: String,
        message: String,
        fields: Vec<CodexFormField>,
    },
    McpToolApproval {
        server: String,
        message: String,
        review: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexQuestion {
    pub(crate) id: String,
    pub(crate) header: String,
    pub(crate) text: String,
    pub(crate) other: bool,
    pub(crate) options: Vec<CodexQuestionOption>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexQuestionOption {
    pub(crate) label: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexFormField {
    pub(crate) name: String,
    pub(crate) title: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) description: Option<String>,
    pub(crate) min_chars: u32,
    pub(crate) max_chars: u32,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) choices: Option<Vec<String>>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) default: Option<String>,
}

fn valid_codex_prompt(prompt: &CodexInputPrompt) -> bool {
    fn bounded(value: &str, max: usize) -> bool {
        value.len() <= max
    }
    fn label(value: &str, max: usize) -> bool {
        bounded(value, max) && !value.is_empty() && !value.chars().any(char::is_control)
    }
    fn id(value: &str, max: usize) -> bool {
        label(value, max)
    }
    match prompt {
        CodexInputPrompt::Questions {
            item, questions, ..
        } => {
            let mut ids = std::collections::BTreeSet::new();
            id(item, 256)
                && (1..=4).contains(&questions.len())
                && questions.iter().all(|q| {
                    id(&q.id, 128)
                        && ids.insert(&q.id)
                        && bounded(&q.header, 256)
                        && bounded(&q.text, 4096)
                        && (1..=16).contains(&q.options.len())
                        && {
                            let mut options = std::collections::BTreeSet::new();
                            q.options.iter().all(|o| {
                                label(&o.label, 1024)
                                    && options.insert(&o.label)
                                    && bounded(&o.description, 2048)
                            })
                        }
                })
        }
        CodexInputPrompt::McpForm {
            server,
            message,
            fields,
        } => {
            let mut names = std::collections::BTreeSet::new();
            label(server, 256)
                && bounded(message, 4096)
                && fields.len() <= 8
                && fields.iter().all(|f| {
                    label(&f.name, 128)
                        && names.insert(&f.name)
                        && bounded(&f.title, 256)
                        && f.description.as_ref().is_none_or(|s| bounded(s, 2048))
                        && f.min_chars <= f.max_chars
                        && f.max_chars <= 4096
                        && f.choices.as_ref().is_none_or(|choices| {
                            (1..=16).contains(&choices.len()) && {
                                let mut values = std::collections::BTreeSet::new();
                                choices.iter().all(|s| bounded(s, 1024) && values.insert(s))
                            }
                        })
                        && f.default.as_ref().is_none_or(|s| {
                            bounded(s, 4096)
                                && s.chars().count() >= f.min_chars as usize
                                && s.chars().count() <= f.max_chars as usize
                                && f.choices.as_ref().is_none_or(|choices| choices.contains(s))
                        })
                })
        }
        CodexInputPrompt::McpToolApproval {
            server,
            message,
            review,
        } => {
            label(server, 256)
                && bounded(message, 4096)
                && bounded(review, 16384)
                && serde_json::from_str::<serde_json::Value>(review)
                    .is_ok_and(|value| value.is_object())
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BrowserRouteView {
    Messages {
        model: String,
    },
    HttpsMessages {
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ChatRouteView {
    HttpsChatCompletions {
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ResponsesRouteView {
    HttpsResponses {
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum GoogleRouteView {
    HttpsGoogleInteractions {
        endpoint: String,
        model: String,
        revision: u64,
        fingerprint: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivityView {
    pub(crate) tool: String,
    pub(crate) phase: ActivityPhase,
    #[serde(default)]
    pub(crate) outcome: Option<ActivityOutcome>,
    #[serde(default)]
    pub(crate) call: Option<ToolCallView>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivityOutcome {
    Succeeded,
    Failed,
    Declined,
    Unspecified,
}

impl fmt::Display for ActivityOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Declined => "declined",
            Self::Unspecified => "unspecified",
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolCallView {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) title: Option<ToolPreviewView>,
    #[serde(default)]
    pub(crate) input: Option<ToolPreviewView>,
    #[serde(default)]
    pub(crate) output: Option<ToolPreviewView>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolPreviewView {
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActivityPhase {
    Started,
    Completed,
    Failed,
}

impl fmt::Display for ActivityPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PiProfileView {
    pub(crate) fingerprint: String,
    pub(crate) provider: String,
    pub(crate) model: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexProfileView {
    pub(crate) fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentSessionView {
    pub(crate) workflow: RunId,
    pub(crate) source: SourceId,
    pub(crate) profile: String,
    pub(crate) revision: u64,
    pub(crate) ready: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireView {
    sequence: u64,
    #[serde(default)]
    fixture_available: bool,
    store: String,
    selection: metldr_history_pages::Scope,
    #[serde(deserialize_with = "Option::deserialize")]
    source: Option<WireSource>,
    runs: Vec<WireRun>,
    #[serde(deserialize_with = "Option::deserialize")]
    native_correction: Option<NativeCorrectionView>,
    #[serde(deserialize_with = "Option::deserialize")]
    pi_profile: Option<PiProfileView>,
    agent_sessions: Vec<AgentSessionView>,
    #[serde(default, deserialize_with = "Option::deserialize")]
    codex_profile: Option<CodexProfileView>,
    #[serde(default)]
    codex_sessions: Vec<AgentSessionView>,
    #[serde(default, deserialize_with = "Option::deserialize")]
    opencode_profile: Option<CodexProfileView>,
    #[serde(default)]
    opencode_sessions: Vec<AgentSessionView>,
    #[serde(default)]
    claude_sessions: Vec<AgentSessionView>,
    #[serde(default)]
    native_permissions: Vec<NativePermissionView>,
    #[serde(default, deserialize_with = "Option::deserialize")]
    pi_ui: Option<PiUiView>,
    #[serde(default, deserialize_with = "Option::deserialize")]
    codex_input: Option<CodexInputView>,
    #[serde(default)]
    browser_actions: Vec<BrowserActionView>,
    #[serde(default)]
    task_plan: Option<TaskPlanView>,
    approval_mode: ApprovalMode,
    #[serde(default, deserialize_with = "Option::deserialize")]
    browser_route: Option<BrowserRouteView>,
    #[serde(default, deserialize_with = "Option::deserialize")]
    chat_route: Option<ChatRouteView>,
    #[serde(default, deserialize_with = "Option::deserialize")]
    responses_route: Option<ResponsesRouteView>,
    #[serde(default, deserialize_with = "Option::deserialize")]
    google_route: Option<GoogleRouteView>,
    #[serde(default = "default_provider_settings")]
    provider_settings: ProviderSettingsView,
    #[serde(default = "default_agent_settings")]
    agent_settings: AgentSettingsView,
}

fn default_agent_settings() -> AgentSettingsView {
    AgentSettingsView {
        busy: false,
        agents: AgentKind::ALL
            .into_iter()
            .map(|agent| AgentSettingView {
                agent,
                revision: 0,
                state: AgentSettingState::Unmanaged,
                profile: None,
                last_request: None,
                last_digest: None,
                found: None,
            })
            .collect(),
        apps: Vec::new(),
    }
}

fn default_provider_settings() -> ProviderSettingsView {
    ProviderSettingsView {
        busy: false,
        pending: None,
        cleanup_pending: false,
        notice: None,
        connections: Protocol::KEYED
            .into_iter()
            .map(|protocol| ProviderConnectionView {
                protocol,
                revision: 0,
                state: ProviderConnectionState::Unmanaged,
                endpoint: None,
                model: None,
                keyless: false,
                timeout_secs: None,
            })
            .collect(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSource {
    id: SourceId,
    tab: i32,
    document: String,
    url: String,
    title: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRun {
    id: RunId,
    source: SourceId,
    kind: RunKind,
    #[serde(deserialize_with = "Option::deserialize")]
    question: Option<String>,
    status: RunStatus,
    output: String,
    conversation: Vec<ProjectedEntry>,
    #[serde(deserialize_with = "Option::deserialize")]
    error: Option<String>,
    route: RunRoute,
    activity: Vec<ActivityView>,
    corrections: CorrectionHistory,
    #[serde(default)]
    ledger: Option<metldr_conversation::Ledger>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FlowError {
    HistoryBusy,
    HistoryStale,
    NoSource,
    Disconnected,
    InvalidInput(String),
    Protocol(String),
    Host(String),
    Remote(String),
    Timeout,
}

impl FlowError {
    pub(crate) fn may_have_committed(&self) -> bool {
        match self {
            Self::HistoryBusy | Self::HistoryStale | Self::NoSource | Self::InvalidInput(_) => {
                false
            }
            Self::Disconnected
            | Self::Protocol(_)
            | Self::Host(_)
            | Self::Remote(_)
            | Self::Timeout => true,
        }
    }
}

impl fmt::Display for FlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HistoryBusy => f.write_str("History is busy. Try again shortly."),
            Self::HistoryStale => f.write_str("History changed. Refresh saved work to continue."),
            Self::NoSource => f.write_str("no saved source"),
            Self::Disconnected => f.write_str("host disconnected"),
            Self::Timeout => {
                f.write_str("Request timed out. Reconnect to check saved work before trying again.")
            }
            Self::InvalidInput(message)
            | Self::Protocol(message)
            | Self::Host(message)
            | Self::Remote(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FlowError {}

impl PageView {
    pub(crate) fn is_older_than(&self, other: &Self) -> bool {
        self.store == other.store
            && if self.selection.epoch != other.selection.epoch {
                self.selection.epoch.0 < other.selection.epoch.0
            } else {
                self.sequence < other.sequence
                    || (self.selection == other.selection
                        && self.sequence == other.sequence
                        && self.source_epoch < other.source_epoch)
            }
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, FlowError> {
        if bytes.len() > MAX_FRAME {
            return Err(FlowError::Protocol("view exceeds 65536 bytes".into()));
        }
        let wire: WireView = if bytes.len() <= MAX_PI_UI.min(32_768) {
            // Compact re-encoding never grows a value, so no projection inside a frame this
            // small can pass its 32 KiB limit: parse once, straight into the typed view.
            serde_json::from_slice(bytes).map_err(|error| FlowError::Protocol(error.to_string()))?
        } else {
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| FlowError::Protocol(error.to_string()))?;
            if value.get("pi_ui").is_some_and(|pi_ui| {
                serde_json::to_vec(pi_ui).map_or(true, |encoded| encoded.len() > MAX_PI_UI)
            }) {
                return Err(FlowError::Protocol(
                    "Pi UI projection exceeds 32768 bytes".into(),
                ));
            }
            if value.get("codex_input").is_some_and(|input| {
                serde_json::to_vec(input).map_or(true, |encoded| encoded.len() > 32_768)
            }) {
                return Err(FlowError::Protocol(
                    "Codex input projection exceeds 32768 bytes".into(),
                ));
            }
            serde_json::from_value(value).map_err(|error| FlowError::Protocol(error.to_string()))?
        };
        if wire.store.len() != 32
            || !wire.store.bytes().all(|byte| byte.is_ascii_hexdigit())
            || wire.runs.len() > MAX_RUNS
            || wire.agent_sessions.len() > MAX_RUNS
            || wire.codex_sessions.len() > MAX_RUNS
            || wire.opencode_sessions.len() > MAX_RUNS
            || wire.claude_sessions.len() > MAX_RUNS
            || wire.native_permissions.len() > 8
            || wire.pi_ui.as_ref().is_some_and(|ui| {
                ui.pending.len() > 8
                    || ui.statuses.len() > 16
                    || ui.widgets.len() > 8
                    || ui.notices.len() > 32
            })
            || wire
                .codex_input
                .as_ref()
                .is_some_and(|ui| ui.pending.len() > 8)
            || wire.browser_actions.len() > MAX_BROWSER_ACTIONS
            || wire.task_plan.as_ref().is_some_and(|plan| {
                let valid = |s: &str, max| {
                    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
                };
                plan.revision == 0
                    || !valid(&plan.goal, 1024)
                    || !valid(&plan.scope, 1024)
                    || plan.intended_effects.len() > 16
                    || plan.intended_effects.iter().any(|s| !valid(s, 512))
                    || !wire.runs.iter().any(|run| run.id == plan.run)
            })
        {
            return Err(FlowError::Protocol(
                "invalid view identity or run count".into(),
            ));
        }
        wire.selection
            .validate()
            .map_err(|_| FlowError::Protocol("invalid view selection".into()))?;
        if wire.selection.store != wire.store {
            return Err(FlowError::Protocol(
                "view selection belongs to another store".into(),
            ));
        }
        let valid_id = |id: &str| parse_identity(id).is_ok_and(|(store, _)| store == wire.store);
        let source = wire
            .source
            .map(|source| source.into_view(&wire.store))
            .transpose()?;
        let mut ids = std::collections::BTreeSet::new();
        if let Some(profile) = &wire.pi_profile
            && (!valid_fingerprint(&profile.fingerprint)
                || !valid_label(&profile.provider, 128)
                || !valid_label(&profile.model, 128))
            {
                return Err(FlowError::Protocol("invalid Pi profile".into()));
            }
        let mut sessions = std::collections::BTreeSet::new();
        for session in &wire.agent_sessions {
            if !valid_id(session.workflow.as_str())
                || !valid_id(session.source.as_str())
                || !sessions.insert(session.workflow.as_str())
                || !valid_fingerprint(&session.profile)
                || session.revision == 0
            {
                return Err(FlowError::Protocol("invalid agent session".into()));
            }
        }
        if wire
            .codex_profile
            .as_ref()
            .is_some_and(|profile| !valid_fingerprint(&profile.fingerprint))
        {
            return Err(FlowError::Protocol("invalid Codex profile".into()));
        }
        for session in &wire.codex_sessions {
            if !valid_id(session.workflow.as_str())
                || !valid_id(session.source.as_str())
                || !sessions.insert(session.workflow.as_str())
                || !valid_fingerprint(&session.profile)
                || session.revision == 0
            {
                return Err(FlowError::Protocol("invalid Codex session".into()));
            }
        }
        if wire
            .opencode_profile
            .as_ref()
            .is_some_and(|profile| !valid_fingerprint(&profile.fingerprint))
        {
            return Err(FlowError::Protocol("invalid OpenCode profile".into()));
        }
        for session in wire.opencode_sessions.iter().chain(&wire.claude_sessions) {
            if !valid_id(session.workflow.as_str())
                || !valid_id(session.source.as_str())
                || !sessions.insert(session.workflow.as_str())
                || !valid_fingerprint(&session.profile)
                || session.revision == 0
            {
                return Err(FlowError::Protocol("invalid native session".into()));
            }
        }
        let mut active = false;
        let mut runs = Vec::with_capacity(wire.runs.len());
        for run in wire.runs {
            let run = run.into_view(&wire.store, false)?;
            if !ids.insert(run.id.as_str().to_owned()) {
                return Err(FlowError::Protocol("invalid saved run".into()));
            }
            if !run.status.is_terminal() {
                if active {
                    return Err(FlowError::Protocol("multiple active runs".into()));
                }
                active = true;
            }
            runs.push(run);
        }
        if let Some(correction) = &wire.native_correction {
            let Some(run) = runs.iter().find(|run| run.id == correction.run) else {
                return Err(FlowError::Protocol(
                    "native correction run is missing".into(),
                ));
            };
            if run.status.is_terminal()
                || !matches!(&run.route, RunRoute::Codex { .. } | RunRoute::Pi { .. })
                || (correction.available
                    && (run.status != RunStatus::Running
                        || run.corrections.total >= MAX_CORRECTIONS as u8
                        || run.corrections.entries.iter().any(|entry| {
                            matches!(
                                &entry.state,
                                CorrectionState::Pending | CorrectionState::Dispatching
                            )
                        })))
            {
                return Err(FlowError::Protocol(
                    "invalid native correction availability".into(),
                ));
            }
        }
        let run_ids: std::collections::BTreeSet<_> =
            runs.iter().map(|run| run.id.as_str()).collect();
        if let Some(ui) = &wire.pi_ui {
            let mut requests = std::collections::BTreeSet::new();
            let mut status_keys = std::collections::BTreeSet::new();
            let mut widget_keys = std::collections::BTreeSet::new();
            if !valid_id(ui.run.as_str())
                || !runs
                    .iter()
                    .any(|run| run.id == ui.run && matches!(&run.route, RunRoute::Pi { .. }))
                || runs
                    .iter()
                    .any(|run| run.id == ui.run && run.status.is_terminal())
                    && !ui.pending.is_empty()
                || ui.pending.iter().any(|pending| {
                    !valid_id(pending.request.as_str())
                        || !requests.insert(pending.request.as_str())
                })
                || ui
                    .statuses
                    .iter()
                    .any(|status| !status_keys.insert(&status.key))
                || ui.widgets.iter().any(|widget| {
                    !widget_keys.insert(&widget.key)
                        || !matches!(widget.placement.as_str(), "aboveEditor" | "belowEditor")
                })
                || ui.notices.iter().any(|notice| notice.level.is_empty())
            {
                return Err(FlowError::Protocol("invalid Pi UI projection".into()));
            }
        }
        if let Some(ui) = &wire.codex_input {
            let mut requests = std::collections::BTreeSet::new();
            if !valid_id(ui.run.as_str())
                || !runs
                    .iter()
                    .any(|run| run.id == ui.run && matches!(&run.route, RunRoute::Codex { .. }))
                || runs
                    .iter()
                    .any(|run| run.id == ui.run && run.status.is_terminal())
                    && !ui.pending.is_empty()
                || ui.pending.iter().any(|pending| {
                    !valid_id(pending.request.as_str())
                        || !requests.insert(pending.request.as_str())
                        || !valid_codex_prompt(&pending.prompt)
                })
                || wire
                    .native_permissions
                    .iter()
                    .any(|permission| requests.contains(permission.request.as_str()))
            {
                return Err(FlowError::Protocol("invalid Codex input projection".into()));
            }
        }
        let mut permission_ids = std::collections::BTreeSet::new();
        for permission in &wire.native_permissions {
            let mut option_ids = std::collections::BTreeSet::new();
            if !run_ids.contains(permission.run.as_str())
                || !valid_id(permission.request.as_str())
                || !permission_ids.insert(permission.request.as_str())
                || !runs.iter().any(|run| {
                    run.id == permission.run
                        && !run.status.is_terminal()
                        && matches!(
                            (&permission.agent, &run.route),
                            (NativePermissionAgent::Codex, RunRoute::Codex { .. })
                                | (NativePermissionAgent::Opencode, RunRoute::OpenCode { .. })
                                | (NativePermissionAgent::Claude, RunRoute::Harness { .. })
                        )
                })
                || !valid_label(&permission.tool, 128)
                || !valid_label(&permission.title, 256)
                || permission.input.len() > 8_192
                || permission.options.is_empty()
                || permission.options.len() > 16
                || permission.options.iter().any(|option| {
                    !valid_ascii_label(&option.id, 128)
                        || !valid_label(&option.name, 256)
                        || !option_ids.insert(option.id.as_str())
                })
            {
                return Err(FlowError::Protocol("invalid native permission".into()));
            }
        }
        let mut action_ids = std::collections::BTreeSet::new();
        for action in &wire.browser_actions {
            let unsettled = |revision| {
                action.revision == revision
                    && action.outcome.is_none()
                    && action.reason.is_none()
                    && action.result.is_none()
            };
            let phase_revision = match action.phase {
                BrowserActionPhase::Pending => unsettled(1),
                BrowserActionPhase::Approved => unsettled(2),
                BrowserActionPhase::Dispatching => unsettled(3),
                BrowserActionPhase::Settled => match (action.outcome, &action.intent) {
                    (
                        Some(BrowserActionOutcome::Verified),
                        BrowserActionIntent::Navigate { .. },
                    )
                    | (Some(BrowserActionOutcome::Acted), BrowserActionIntent::Act { .. })
                    | (
                        Some(BrowserActionOutcome::SecretEntered),
                        BrowserActionIntent::EnterSecret { .. },
                    )
                    | (Some(BrowserActionOutcome::Uncertain), _) => action.revision == 4,
                    (Some(BrowserActionOutcome::Prevented), _) => {
                        (2..=4).contains(&action.revision)
                    }
                    _ => false,
                },
            };
            let reason_valid = match (action.outcome, action.reason.as_deref()) {
                (
                    Some(BrowserActionOutcome::Prevented | BrowserActionOutcome::Uncertain),
                    Some(reason),
                ) => {
                    !reason.is_empty()
                        && reason.len() <= 200
                        && !reason.chars().any(char::is_control)
                }
                (Some(BrowserActionOutcome::Prevented | BrowserActionOutcome::Uncertain), None) => {
                    false
                }
                (_, reason) => reason.is_none(),
            };
            let result_valid = match (action.outcome, action.result.as_deref()) {
                (Some(BrowserActionOutcome::Acted), Some(result)) => {
                    result.len() <= 4_096 && !result.chars().any(|c| c.is_control() && c != '\n')
                }
                (Some(BrowserActionOutcome::Acted), None) => false,
                (_, result) => result.is_none(),
            };
            if !valid_id(action.id.as_str())
                || !action_ids.insert(action.id.as_str().to_owned())
                || !run_ids.contains(action.run.as_str())
                || action.tab < 0
                || action.document.is_empty()
                || action.document.len() > 128
                || action.document.chars().any(char::is_control)
                || action.url.is_empty()
                || action.url.len() > 2_048
                || action.url.chars().any(char::is_control)
                || !valid_browser_intent(&action.intent)
                || !phase_revision
                || !reason_valid
                || !result_valid
            {
                return Err(FlowError::Protocol("invalid browser action".into()));
            }
        }
        if wire
            .browser_route
            .as_ref()
            .is_some_and(|route| match route {
                BrowserRouteView::Messages { model } => model != "fixture-browser-model",
                BrowserRouteView::HttpsMessages {
                    endpoint,
                    model,
                    revision,
                    fingerprint,
                } => {
                    !valid_https_endpoint(endpoint)
                        || !valid_ascii_label(model, 128)
                        || *revision == 0
                        || !valid_fingerprint(fingerprint)
                }
            })
        {
            return Err(FlowError::Protocol("invalid browser route".into()));
        }
        if wire
            .responses_route
            .as_ref()
            .is_some_and(|route| match route {
                ResponsesRouteView::HttpsResponses {
                    endpoint,
                    model,
                    revision,
                    fingerprint,
                } => {
                    !valid_https_endpoint(endpoint)
                        || !valid_ascii_label(model, 128)
                        || *revision == 0
                        || !valid_fingerprint(fingerprint)
                }
            })
        {
            return Err(FlowError::Protocol("invalid Responses route".into()));
        }
        if wire.chat_route.as_ref().is_some_and(|route| match route {
            ChatRouteView::HttpsChatCompletions {
                endpoint,
                model,
                revision,
                fingerprint,
            } => {
                !valid_chat_endpoint(endpoint)
                    || !valid_ascii_label(model, 128)
                    || *revision == 0
                    || !valid_fingerprint(fingerprint)
            }
        }) {
            return Err(FlowError::Protocol("invalid Chat Completions route".into()));
        }
        if wire.google_route.as_ref().is_some_and(|route| match route {
            GoogleRouteView::HttpsGoogleInteractions {
                endpoint,
                model,
                revision,
                fingerprint,
            } => {
                !valid_https_endpoint(endpoint)
                    || !valid_ascii_label(model, 128)
                    || *revision == 0
                    || !valid_fingerprint(fingerprint)
            }
        }) {
            return Err(FlowError::Protocol(
                "invalid Google Interactions route".into(),
            ));
        }
        let settings = &wire.provider_settings;
        let mut protocols = std::collections::BTreeSet::new();
        if (settings.pending.is_some() && !settings.busy)
            || settings.connections.len() != Protocol::KEYED.len()
            || settings
                .connections
                .iter()
                .any(|connection| !protocols.insert(connection.protocol))
            || !Protocol::KEYED.iter().all(|protocol| protocols.contains(protocol))
            || settings
                .notice
                .as_ref()
                .is_some_and(|notice| notice.len() > 256 || notice.chars().any(char::is_control))
            || settings.connections.iter().any(|connection| {
                let has_metadata = connection.endpoint.is_some() && connection.model.is_some();
                let partial_metadata = connection.endpoint.is_some() != connection.model.is_some();
                partial_metadata
                    || match connection.state {
                        ProviderConnectionState::Unmanaged => {
                            connection.revision != 0 || has_metadata
                        }
                        ProviderConnectionState::Removed => {
                            connection.revision == 0 || has_metadata
                        }
                        ProviderConnectionState::Loading
                        | ProviderConnectionState::Ready
                        | ProviderConnectionState::Missing
                        | ProviderConnectionState::Locked
                        | ProviderConnectionState::Corrupt
                        | ProviderConnectionState::Unavailable => {
                            connection.revision == 0
                                || !has_metadata
                                || !connection.endpoint.as_deref().is_some_and(|endpoint| {
                                    if connection.protocol == Protocol::ChatCompletions {
                                        valid_chat_endpoint(endpoint)
                                    } else {
                                        valid_https_endpoint(endpoint)
                                    }
                                })
                                || !connection
                                    .model
                                    .as_deref()
                                    .is_some_and(|model| valid_ascii_label(model, 128))
                        }
                    }
            })
        {
            return Err(FlowError::Protocol("invalid provider settings view".into()));
        }
        let settings = &wire.agent_settings;
        let mut agents = std::collections::BTreeSet::new();
        if settings.agents.len() != AgentKind::ALL.len()
            || settings.agents.iter().any(|row| {
                !agents.insert(row.agent)
                    || row.last_request.is_some() != row.last_digest.is_some()
                    || row
                        .profile
                        .as_ref()
                        .is_some_and(|profile| !valid_agent_profile(profile))
                    || row
                        .profile
                        .as_ref()
                        .is_some_and(|profile| profile.agent() != row.agent)
                    || row.last_request.as_ref().is_some_and(|value| {
                        value.is_empty()
                            || value.len() > 96
                            || !value.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-')
                            })
                    })
                    || row.last_digest.as_ref().is_some_and(|value| {
                        value.len() != 64
                            || !value
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    })
                    || match row.state {
                        AgentSettingState::Unmanaged => {
                            row.revision != 0
                                || row.profile.is_some()
                                || row.last_request.is_some()
                                || row.last_digest.is_some()
                        }
                        AgentSettingState::Removed => {
                            row.revision == 0 || row.profile.is_some() || row.last_request.is_none()
                        }
                        AgentSettingState::Ready | AgentSettingState::Unavailable => {
                            row.revision == 0 || row.profile.is_none() || row.last_request.is_none()
                        }
                    }
            })
            || agents.len() != AgentKind::ALL.len()
        {
            return Err(FlowError::Protocol("invalid agent settings view".into()));
        }
        Ok(Self {
            sequence: wire.sequence,
            fixture_available: wire.fixture_available,
            store: wire.store,
            selection: wire.selection,
            source,
            runs,
            native_correction: wire.native_correction,
            pi_profile: wire.pi_profile,
            agent_sessions: wire.agent_sessions,
            codex_profile: wire.codex_profile,
            codex_sessions: wire.codex_sessions,
            opencode_profile: wire.opencode_profile,
            opencode_sessions: wire.opencode_sessions,
            claude_sessions: wire.claude_sessions,
            native_permissions: wire.native_permissions,
            pi_ui: wire.pi_ui,
            codex_input: wire.codex_input,
            browser_actions: wire.browser_actions,
            task_plan: wire.task_plan,
            approval_mode: wire.approval_mode,
            browser_route: wire.browser_route,
            chat_route: wire.chat_route,
            responses_route: wire.responses_route,
            google_route: wire.google_route,
            provider_settings: wire.provider_settings,
            agent_settings: wire.agent_settings,
            source_status: SourceStatus::Checking,
            source_epoch: 0,
        })
    }

    pub(crate) fn can_approve(&self, action: &BrowserActionView, epoch: u64) -> bool {
        self.source_epoch == epoch
            && action.phase == BrowserActionPhase::Pending
            && action.target_current
            && self.source.as_ref().is_some_and(|source| {
                self.runs
                    .iter()
                    .any(|run| run.id == action.run && run.source == source.id)
            })
    }

    pub(crate) fn source(&self) -> Option<&SourceId> {
        self.source.as_ref().map(|source| &source.id)
    }

    pub(crate) fn active_run(&self) -> Option<&RunId> {
        self.runs
            .iter()
            .find(|run| !run.status.is_terminal())
            .map(|run| &run.id)
    }
}

fn valid_browser_intent(intent: &BrowserActionIntent) -> bool {
    match intent {
        BrowserActionIntent::EnterSecret {
            element,
            reference,
            expires_at_ms,
            ..
        } => {
            element.valid()
                && reference.len() == 32
                && reference.bytes().all(|b| b.is_ascii_hexdigit())
                && *expires_at_ms > 0
        }
        BrowserActionIntent::Act {
            tool,
            args,
            elements,
        } => {
            matches!(
                tool.as_str(),
                "click" | "fill" | "type" | "press" | "select" | "hover" | "drag" | "scroll"
            ) && args.is_object()
                && serde_json::to_vec(args).is_ok_and(|bytes| bytes.len() <= MAX_ACT_ARGS)
                && elements.len() <= 33
                && elements.iter().all(BrowserElement::valid)
                && elements
                    .iter()
                    .map(|element| element.reference.as_str())
                    .eq(named_refs(args))
        }
        BrowserActionIntent::Navigate { destination } => {
            !destination.is_empty()
                && destination.len() <= 2_048
                && !destination.chars().any(char::is_control)
        }
    }
}

/// The refs a page action's arguments name, in order: `ref`, `from`, `to`, then each field's.
pub(crate) fn named_refs(args: &serde_json::Value) -> Vec<&str> {
    let mut refs: Vec<&str> = ["ref", "from", "to"]
        .into_iter()
        .filter_map(|key| args[key].as_str())
        .collect();
    if let Some(fields) = args["fields"].as_array() {
        refs.extend(fields.iter().filter_map(|field| field["ref"].as_str()));
    }
    refs
}

fn valid_selection_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn valid_label(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}

fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn valid_ascii_label(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|byte| byte.is_ascii_graphic())
}

pub(crate) fn valid_https_endpoint(value: &str) -> bool {
    value.len() <= 1_024
        && value
            .strip_prefix("https://")
            .is_some_and(|rest| endpoint_host(rest).is_some())
}

/// The host of what follows the scheme, when it has no query, fragment, whitespace or control
/// characters and a valid optional port.
fn endpoint_host(rest: &str) -> Option<&str> {
    // One pass; ASCII, the usual case, is checked byte by byte.
    let clean = if rest.is_ascii() {
        rest.bytes()
            .all(|b| b > b' ' && b != 0x7f && b != b'?' && b != b'#')
    } else {
        !rest
            .chars()
            .any(|c| matches!(c, '?' | '#') || c.is_whitespace() || c.is_control())
    };
    if !clean {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or_default();
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, suffix) = bracketed.split_once(']')?;
        (host.parse::<std::net::Ipv6Addr>().is_ok()
            && (suffix.is_empty()
                || valid_url_port(suffix.strip_prefix(':')) && suffix.starts_with(':')))
        .then_some(host)
    } else {
        let (host, valid_port) = match authority.split_once(':') {
            Some((host, port)) => (host, valid_url_port(Some(port))),
            None => (authority, true),
        };
        (!host.is_empty() && !host.contains(['@', '[', ']', ':']) && valid_port).then_some(host)
    }
}

/// An endpoint `protocol` may reach: loopback HTTP for Chat Completions, else HTTPS only.
pub(crate) fn valid_endpoint(protocol: Protocol, value: &str) -> bool {
    match protocol {
        Protocol::ChatCompletions => valid_chat_endpoint(value),
        _ => valid_https_endpoint(value),
    }
}

/// A Chat Completions endpoint: HTTPS, or plain HTTP to a loopback server such as Ollama.
pub(crate) fn valid_chat_endpoint(value: &str) -> bool {
    valid_https_endpoint(value)
        || (value.len() <= 1_024
            && value
                .strip_prefix("http://")
                .and_then(endpoint_host)
                .is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || host
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                }))
}

fn valid_url_port(port: Option<&str>) -> bool {
    port.is_none_or(|port| {
        !port.is_empty()
            && port.bytes().all(|byte| byte.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|port| port > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_endpoints_allow_plain_http_only_on_loopback() {
        for valid in [
            "https://api.example/v1/chat/completions",
            "http://127.0.0.1:11434/v1/chat/completions",
            "http://localhost:8080/v1/chat/completions",
            "http://[::1]:8080/v1",
        ] {
            assert!(valid_chat_endpoint(valid), "{valid}");
        }
        for invalid in [
            "http://example.com/v1",
            "http://10.0.0.1:11434/v1",
            "http://127.0.0.1.example.com/v1",
            "http://user@127.0.0.1/v1",
        ] {
            assert!(!valid_chat_endpoint(invalid), "{invalid}");
        }
        assert!(!valid_https_endpoint("http://127.0.0.1:11434/v1"));
        for route in metldr_core::gateway::chat_routes() {
            let endpoint = &route.endpoint();
            assert!(valid_chat_endpoint(endpoint), "{endpoint}");
        }
    }

    const STORE: &str = "0123456789abcdef0123456789abcdef";
    const VALID: &str = r#"{"approval_mode":"ask","sequence":2,"store":"0123456789abcdef0123456789abcdef","selection":{"store":"0123456789abcdef0123456789abcdef","conversation":"0123456789abcdef0123456789abcdef:100","epoch":"1"},"source":{"id":"0123456789abcdef0123456789abcdef:1","tab":4,"document":"doc-1","url":"http://127.0.0.1:43123/article","title":"North report","text":"The North project reports an amount of 17 units."},"runs":[{"id":"0123456789abcdef0123456789abcdef:2","source":"0123456789abcdef0123456789abcdef:1","kind":"summary","question":null,"status":"complete","output":"North reports 17 units.","conversation":[],"error":null,"route":{"kind":"fixture"},"activity":[],"corrections":{"total":0,"entries":[]}}],"native_correction":null,"pi_profile":null,"agent_sessions":[],"browser_route":null}"#;

    #[test]
    fn task_plan_decodes_only_for_a_current_run() {
        assert!(
            PageView::decode(VALID.as_bytes())
                .unwrap()
                .task_plan
                .is_none()
        );
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["task_plan"] = serde_json::json!({"run":"0123456789abcdef0123456789abcdef:2", "revision":7,
            "goal":"Find information", "intended_effects":["Open a page"], "scope":"One site", "state":"proposed"});
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert_eq!(decoded.task_plan.unwrap().revision, 7);
        view["task_plan"]["run"] = serde_json::json!("0123456789abcdef0123456789abcdef:3");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn decodes_host_view_and_rejects_invalid_boundary_data() {
        let view = PageView::decode(VALID.as_bytes()).unwrap();
        assert_eq!(view.sequence, 2);
        assert_eq!(view.source().unwrap().as_str(), format!("{STORE}:1"));
        assert_eq!(view.runs[0].kind.to_string(), "summary");
        assert!(view.active_run().is_none());

        for invalid in [
            VALID.replace(
                "\"selection\":{\"store\":\"0123456789abcdef0123456789abcdef\",\"conversation\":\"0123456789abcdef0123456789abcdef:100\",\"epoch\":\"1\"},",
                "",
            ),
            VALID.replace(
                "\"selection\":{\"store\":\"0123456789abcdef0123456789abcdef\",\"conversation\":\"0123456789abcdef0123456789abcdef:100\",\"epoch\":\"1\"}",
                "\"selection\":null",
            ),
            VALID.replace(
                "\"selection\":{\"store\":\"0123456789abcdef0123456789abcdef\",\"conversation\":\"0123456789abcdef0123456789abcdef:100\",\"epoch\":\"1\"}",
                "\"selection\":{\"store\":\"ffffffffffffffffffffffffffffffff\",\"conversation\":\"ffffffffffffffffffffffffffffffff:100\",\"epoch\":\"1\"}",
            ),
            VALID.replace("\"conversation\":[],", ""),
            VALID.replace(&format!("{STORE}:1"), "ffffffffffffffffffffffffffffffff:1"),
            VALID.replace("\"kind\":\"summary\"", "\"kind\":\"unknown\""),
            VALID.replace("\"status\":\"complete\"", "\"status\":\"unknown\""),
            VALID.replace(
                "\"route\":{\"kind\":\"fixture\"}",
                "\"route\":{\"kind\":\"unknown\"}",
            ),
            VALID.replace(
                "\"activity\":[]",
                "\"activity\":[{\"tool\":\"read\",\"phase\":\"unknown\"}]",
            ),
            VALID.replace("\"sequence\":2", "\"sequence\":2,\"extra\":true"),
            VALID.replace(&format!("{STORE}:2"), &format!("{STORE}:2 ")),
        ] {
            assert!(PageView::decode(invalid.as_bytes()).is_err());
        }
        for alias in ["02", "+2"] {
            let saved = VALID.replace(&format!("{STORE}:2"), &format!("{STORE}:{alias}"));
            let view = PageView::decode(saved.as_bytes()).unwrap();
            assert_eq!(view.runs[0].id.as_str(), format!("{STORE}:{alias}"));
        }
        let mut duplicate: serde_json::Value = serde_json::from_str(VALID).unwrap();
        let run = duplicate["runs"][0].clone();
        duplicate["runs"].as_array_mut().unwrap().push(run);
        assert!(PageView::decode(&serde_json::to_vec(&duplicate).unwrap()).is_err());
        let mut absent: serde_json::Value = serde_json::from_str(VALID).unwrap();
        absent.as_object_mut().unwrap().remove("source");
        assert!(PageView::decode(&serde_json::to_vec(&absent).unwrap()).is_err());
        for field in ["pi_profile", "agent_sessions"] {
            let mut absent: serde_json::Value = serde_json::from_str(VALID).unwrap();
            absent.as_object_mut().unwrap().remove(field);
            assert!(PageView::decode(&serde_json::to_vec(&absent).unwrap()).is_err());
        }
        assert!(PageView::decode(&vec![b' '; MAX_FRAME + 1]).is_err());
    }

    #[test]
    fn native_correction_history_enforces_its_receipt_limit() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["runs"][0]["corrections"]["total"] = serde_json::json!(17);
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn native_correction_outcomes_match_the_saved_agent_route() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["runs"][0]["kind"] = serde_json::json!("agent");
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"codex", "workflow":format!("{STORE}:3"), "provider":null, "model":null
        });
        view["runs"][0]["corrections"] = serde_json::json!({
            "total":1,
            "entries":[{
                "ordinal":1, "request":"correction-1",
                "text":{"text":"Exact text","truncated":false},
                "state":{"kind":"pi_queued"}
            }]
        });
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
        view["runs"][0]["corrections"]["entries"][0]["state"] =
            serde_json::json!({"kind":"codex_accepted"});
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_ok());
    }

    #[test]
    fn selection_epoch_orders_views_before_global_sequence() {
        let older = PageView::decode(
            VALID
                .replace("\"sequence\":2", "\"sequence\":99")
                .as_bytes(),
        )
        .unwrap();
        let newer = PageView::decode(
            VALID
                .replace("\"sequence\":2", "\"sequence\":3")
                .replace("\"epoch\":\"1\"", "\"epoch\":\"2\"")
                .as_bytes(),
        )
        .unwrap();
        assert!(older.is_older_than(&newer));
        assert!(!newer.is_older_than(&older));
    }

    #[test]
    fn decodes_bounded_pi_ui_without_normalizing_choice_values() {
        let mut frame: serde_json::Value = serde_json::from_str(VALID).unwrap();
        frame["runs"][0]["kind"] = serde_json::json!("agent");
        frame["runs"][0]["status"] = serde_json::json!("running");
        frame["runs"][0]["route"] = serde_json::json!({
            "kind":"pi", "workflow":format!("{STORE}:3"), "provider":"fixture", "model":"fixture"
        });
        frame["pi_ui"] = serde_json::json!({
            "run":format!("{STORE}:2"),
            "pending":[{"request":format!("{STORE}:4"),"dialog":{"kind":"select","title":"Pick","options":["","同じ 🪷","同じ 🪷"]}}],
            "title":"Native title", "statuses":[{"key":"","text":null}],
            "widgets":[{"key":"\u{0000}", "lines":["first","second"],"placement":"belowEditor"}],
            "editor":{"revision":1,"text":"suggestion"},
            "notices":[{"level":"info","message":"Ready"}], "notices_omitted":1
        });
        let view = PageView::decode(&serde_json::to_vec(&frame).unwrap()).unwrap();
        let ui = view.pi_ui.unwrap();
        let PiUiDialog::Select { options, .. } = &ui.pending[0].dialog else {
            panic!("select dialog expected")
        };
        assert_eq!(options, &["", "同じ 🪷", "同じ 🪷"]);
        assert_eq!(ui.statuses[0].key, "");
        assert_eq!(ui.statuses[0].text, None);
        assert_eq!(ui.widgets[0].key, "\u{0}");
        assert_eq!(ui.widgets[0].placement, "belowEditor");
        assert_eq!(ui.notices_omitted, 1);

        frame["pi_ui"] = serde_json::Value::Null;
        assert!(
            PageView::decode(&serde_json::to_vec(&frame).unwrap())
                .unwrap()
                .pi_ui
                .is_none()
        );
        frame["pi_ui"] = serde_json::json!({
            "run":format!("{STORE}:2"), "pending":[], "title":null,
            "statuses":[], "widgets":[], "editor":null, "notices":[], "extra":true
        });
        assert!(PageView::decode(&serde_json::to_vec(&frame).unwrap()).is_err());
        frame["pi_ui"].as_object_mut().unwrap().remove("extra");
        frame["pi_ui"]["pending"] = serde_json::json!([]);
        frame["runs"][0]["status"] = serde_json::json!("complete");
        assert!(PageView::decode(&serde_json::to_vec(&frame).unwrap()).is_ok());
    }

    #[test]
    fn decodes_bounded_codex_input_and_rejects_foreign_or_malformed_requests() {
        let mut frame: serde_json::Value = serde_json::from_str(VALID).unwrap();
        let run = format!("{STORE}:2");
        frame["runs"][0]["kind"] = serde_json::json!("agent");
        frame["runs"][0]["status"] = serde_json::json!("running");
        frame["runs"][0]["route"] =
            serde_json::json!({"kind":"codex","workflow":run.clone(),"provider":null,"model":null});
        frame["codex_input"] = serde_json::json!({"run":run.clone(),"pending":[{"request":format!("{STORE}:3"),"prompt":{"kind":"questions","item":"setup","is_blocking":true,"questions":[{"id":"choice","header":"","text":"Pick one\nexactly","other":false,"options":[{"label":" A ","description":"Exact\nvalue"}]}]}}]});
        let decoded = PageView::decode(&serde_json::to_vec(&frame).unwrap()).unwrap();
        let CodexInputPrompt::Questions { questions, .. } =
            &decoded.codex_input.as_ref().unwrap().pending[0].prompt
        else {
            panic!("questions prompt expected")
        };
        assert_eq!(questions[0].options[0].label, " A ");
        assert_eq!(questions[0].header, "");
        assert_eq!(questions[0].text, "Pick one\nexactly");
        assert_eq!(questions[0].options[0].description, "Exact\nvalue");

        frame["codex_input"]["run"] = serde_json::json!(format!("{STORE}:99"));
        assert!(PageView::decode(&serde_json::to_vec(&frame).unwrap()).is_err());
        frame["codex_input"]["run"] = serde_json::json!(run);
        frame["codex_input"]["pending"][0]["prompt"]["unexpected"] = serde_json::json!(true);
        assert!(PageView::decode(&serde_json::to_vec(&frame).unwrap()).is_err());
        frame["codex_input"] =
            serde_json::json!({"run":run.clone(),"pending":[],"padding":"x".repeat(32_769)});
        assert!(PageView::decode(&serde_json::to_vec(&frame).unwrap()).is_err());
    }

    #[test]
    fn decodes_optional_bounded_tool_call_previews() {
        let mut frame: serde_json::Value = serde_json::from_str(VALID).unwrap();
        frame["runs"][0]["kind"] = serde_json::json!("agent");
        frame["runs"][0]["route"] = serde_json::json!({
            "kind": "pi",
            "workflow": format!("{STORE}:2"),
            "provider": "fixture",
            "model": "fixture"
        });
        frame["runs"][0]["activity"] = serde_json::json!([{
            "tool": "read_file",
            "phase": "completed",
            "call": {
                "id": "native-call-1",
                "title": { "text": "Read a file", "truncated": false },
                "input": { "text": "{\"path\":\"notes.txt\"}", "truncated": false },
                "output": { "text": "contents", "truncated": true }
            }
        }]);
        let decoded = PageView::decode(&serde_json::to_vec(&frame).unwrap()).unwrap();
        let call = decoded.runs[0].activity[0].call.as_ref().unwrap();
        assert_eq!(call.id, "native-call-1");
        assert_eq!(call.output.as_ref().unwrap().text, "contents");
        assert!(call.output.as_ref().unwrap().truncated);

        for (field, invalid) in [
            ("id", serde_json::json!("")),
            ("id", serde_json::json!("line\nbreak")),
            (
                "title",
                serde_json::json!({ "text": "x".repeat(257), "truncated": false }),
            ),
            (
                "input",
                serde_json::json!({ "text": "x".repeat(1_025), "truncated": false }),
            ),
            (
                "output",
                serde_json::json!({ "text": "x".repeat(2_049), "truncated": false }),
            ),
        ] {
            let mut invalid_frame = frame.clone();
            invalid_frame["runs"][0]["activity"][0]["call"][field] = invalid;
            assert!(PageView::decode(&serde_json::to_vec(&invalid_frame).unwrap()).is_err());
        }
    }

    #[test]
    fn decodes_optional_activity_outcome_and_rejects_phase_mismatch() {
        let mut frame: serde_json::Value = serde_json::from_str(VALID).unwrap();
        frame["runs"][0]["kind"] = serde_json::json!("agent");
        frame["runs"][0]["route"] = serde_json::json!({"kind":"codex", "workflow":format!("{STORE}:2"), "provider":null, "model":null});
        frame["runs"][0]["activity"] = serde_json::json!([
            {"tool":"read", "phase":"completed", "outcome":"succeeded"},
            {"tool":"legacy", "phase":"completed"}
        ]);
        let decoded = PageView::decode(&serde_json::to_vec(&frame).unwrap()).unwrap();
        assert_eq!(
            decoded.runs[0].activity[0].outcome,
            Some(ActivityOutcome::Succeeded)
        );
        assert_eq!(decoded.runs[0].activity[1].outcome, None);
        for (phase, outcome) in [
            ("started", "failed"),
            ("completed", "declined"),
            ("failed", "succeeded"),
            ("completed", "other"),
        ] {
            frame["runs"][0]["activity"] =
                serde_json::json!([{"tool":"read", "phase":phase, "outcome":outcome}]);
            assert!(PageView::decode(&serde_json::to_vec(&frame).unwrap()).is_err());
        }
    }

    #[test]
    fn decodes_provider_settings_and_rejects_malformed_present_views() {
        let legacy = PageView::decode(VALID.as_bytes()).unwrap();
        assert_eq!(
            legacy.provider_settings.connections.len(),
            Protocol::KEYED.len()
        );
        assert!(
            legacy.provider_settings.connections.iter().all(|row| {
                row.state == ProviderConnectionState::Unmanaged && row.revision == 0
            })
        );

        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["provider_settings"] = serde_json::json!({
            "busy":false,"pending":null,"cleanup_pending":false,"notice":null,
            "connections":[
                {"protocol":"messages","revision":2,"state":"locked","endpoint":"https://api.example/v1","model":"model-a"},
                {"protocol":"chat_completions","revision":3,"state":"ready","endpoint":"https://llm.example/v1/chat/completions","model":"local","keyless":true,"timeout_secs":90},
                {"protocol":"responses","revision":0,"state":"unmanaged","endpoint":null,"model":null},
                {"protocol":"google_interactions","revision":1,"state":"removed","endpoint":null,"model":null},
                {"protocol":"system_one","revision":0,"state":"unmanaged","endpoint":null,"model":null}
            ]
        });
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert_eq!(
            decoded.provider_settings.connections.len(),
            Protocol::KEYED.len()
        );
        assert_eq!(
            decoded.provider_settings.connections[0].state,
            ProviderConnectionState::Locked
        );
        // Rows from an older companion omit the Chat Completions fields.
        let chat = &decoded.provider_settings.connections[1];
        assert!(chat.keyless && chat.timeout_secs == Some(90));
        assert!(!decoded.provider_settings.connections[0].keyless);

        view["provider_settings"]["connections"][1]["protocol"] = serde_json::json!("messages");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
        view["provider_settings"] = serde_json::Value::Null;
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn decodes_agent_settings_and_rejects_invalid_present_rows() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["agent_settings"] = serde_json::json!({"busy":false,"agents":[
            {"agent":"codex","revision":0,"state":"unmanaged","profile":null,"last_request":null,"last_digest":null},
            {"agent":"opencode","revision":0,"state":"unmanaged","profile":null,"last_request":null,"last_digest":null},
            {"agent":"claude","revision":0,"state":"unmanaged","profile":null,"last_request":null,"last_digest":null},
            {"agent":"pi","revision":0,"state":"unmanaged","profile":null,"last_request":null,"last_digest":null}
        ]});
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert_eq!(decoded.agent_settings.agents.len(), AgentKind::ALL.len());
        view["agent_settings"]["agents"][2]["state"] = serde_json::json!("unknown");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());

        let mut configured: serde_json::Value = serde_json::from_str(VALID).unwrap();
        configured["agent_settings"] = serde_json::json!({"busy":false,"agents":[
            {"agent":"codex","revision":1,"state":"ready","profile":{"kind":"codex","executable":"/tool/codex","workspace":"/output"},"last_request":"req-01","last_digest":"a".repeat(64)},
            {"agent":"opencode","revision":0,"state":"unmanaged","profile":null,"last_request":null,"last_digest":null},
            {"agent":"claude","revision":0,"state":"unmanaged","profile":null,"last_request":null,"last_digest":null},
            {"agent":"pi","revision":0,"state":"unmanaged","profile":null,"last_request":null,"last_digest":null}
        ]});
        assert!(PageView::decode(&serde_json::to_vec(&configured).unwrap()).is_ok());
        configured["agent_settings"]["agents"][0]["profile"]["executable"] =
            serde_json::json!("x".repeat(2_049));
        assert!(PageView::decode(&serde_json::to_vec(&configured).unwrap()).is_err());
        configured["agent_settings"]["agents"][0]["profile"] = serde_json::json!({
            "kind":"opencode","executable":"/tool/opencode","workspace":"/output"
        });
        assert!(PageView::decode(&serde_json::to_vec(&configured).unwrap()).is_err());
    }

    #[test]
    fn user_text_bounds_utf8_bytes_and_rejects_blank_input() {
        let at_limit = "é".repeat(512);
        assert!(UserText::try_from(at_limit.as_str()).is_ok());
        assert!(UserText::try_from(format!("{at_limit}é").as_str()).is_err());
        assert!(UserText::try_from(" \n ").is_err());
    }

    #[test]
    fn decodes_bounded_native_permission_choices_for_matching_agents() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["runs"][0]["kind"] = serde_json::json!("agent");
        view["runs"][0]["status"] = serde_json::json!("running");
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"opencode", "workflow":format!("{STORE}:3"), "model":"model-a", "mode":"default"
        });
        view["opencode_profile"] = serde_json::json!({"fingerprint":"a".repeat(64)});
        view["opencode_sessions"] = serde_json::json!([{
            "workflow":format!("{STORE}:3"), "source":format!("{STORE}:1"),
            "profile":"a".repeat(64), "revision":1, "ready":true
        }]);
        view["native_permissions"] = serde_json::json!([{
            "agent":"opencode",
            "run":format!("{STORE}:2"), "request":format!("{STORE}:4"),
            "tool":"browser.read", "title":"Read page", "input":"{\"url\":\"https://example.test\"}",
            "options":[{"id":"allow_once","name":"Allow once"}]
        }]);
        view["runs"][0]["activity"] = serde_json::json!(vec![
            serde_json::json!({"tool":"execute","phase":"completed"});
            64
        ]);
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert_eq!(decoded.runs[0].activity.len(), 64);
        assert!(matches!(decoded.runs[0].route, RunRoute::OpenCode { .. }));
        assert_eq!(decoded.native_permissions[0].options[0].name, "Allow once");

        view["native_permissions"][0]["options"] = serde_json::json!([
            {"id":"same","name":"Allow once"},
            {"id":"same","name":"Always"}
        ]);
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());

        view["native_permissions"][0]["agent"] = serde_json::json!("codex");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());

        view["native_permissions"][0]["options"] = serde_json::json!([
            {"id":"allow_once","name":"Allow once"}
        ]);
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"codex", "workflow":format!("{STORE}:3"), "provider":null, "model":null
        });
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_ok());
    }

    #[test]
    fn decodes_pi_route_and_rejects_native_handle_in_public_view() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["runs"][0]["kind"] = serde_json::json!("agent");
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"pi", "workflow":format!("{STORE}:3"), "provider":"probe", "model":"probe"
        });
        view["runs"][0]["activity"] = serde_json::json!([{"tool":"read","phase":"completed"}]);
        let fingerprint = "a".repeat(64);
        view["pi_profile"] =
            serde_json::json!({"fingerprint":fingerprint,"provider":"probe","model":"probe"});
        view["agent_sessions"] = serde_json::json!([{
            "workflow":format!("{STORE}:3"),"source":format!("{STORE}:1"),
            "profile":fingerprint,"revision":1,"ready":true
        }]);
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert!(matches!(decoded.runs[0].route, RunRoute::Pi { .. }));
        assert_eq!(decoded.agent_sessions[0].revision, 1);
        view["agent_sessions"][0]["native_path"] = serde_json::json!("/private/session.jsonl");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn decodes_codex_route_and_sessions_without_native_handles() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        let fingerprint = "b".repeat(64);
        view["codex_profile"] = serde_json::json!({"fingerprint":fingerprint});
        view["codex_sessions"] = serde_json::json!([{
            "workflow":format!("{STORE}:3"),"source":format!("{STORE}:1"),
            "profile":fingerprint,"revision":1,"ready":true
        }]);
        view["runs"][0]["kind"] = serde_json::json!("agent");
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"codex", "workflow":format!("{STORE}:3"), "provider":null, "model":null
        });
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert!(matches!(
            decoded.runs[0].route,
            RunRoute::Codex {
                provider: None,
                model: None,
                ..
            }
        ));
        assert_eq!(decoded.codex_sessions[0].revision, 1);
        view["runs"][0]["route"]["provider"] = serde_json::json!("codex");
        view["runs"][0]["route"]["model"] = serde_json::json!("gpt-x");
        assert!(matches!(
            PageView::decode(&serde_json::to_vec(&view).unwrap())
                .unwrap()
                .runs[0]
                .route,
            RunRoute::Codex {
                provider: Some(_),
                model: Some(_),
                ..
            }
        ));
        view["runs"][0]["route"]["model"] = serde_json::Value::Null;
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
        view["runs"][0]["route"]["provider"] = serde_json::Value::Null;
        view["codex_sessions"][0]["native_session"] = serde_json::json!("private");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
        view["codex_sessions"][0]
            .as_object_mut()
            .unwrap()
            .remove("native_session");
        view["codex_sessions"][0]["workflow"] = serde_json::json!("thread-private");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn decodes_page_actions_strictly_and_rejects_the_old_field_form() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        let action = |extra: serde_json::Value| {
            let mut action = serde_json::json!({
                "id":format!("{STORE}:4"), "revision":1,
                "run":format!("{STORE}:2"), "tab":4, "document":"doc-1",
                "url":"http://127.0.0.1:43123/article", "kind":"act", "tool":"fill",
                "args":{"ref":"e2","value":"North"},
                "elements":[{"reference":"e2","role":"textbox","name":"Email address"}],
                "phase":"pending"
            });
            for (key, value) in extra.as_object().unwrap() {
                action[key] = value.clone();
            }
            action
        };
        let decode = |view: &mut serde_json::Value, action: serde_json::Value| {
            view["browser_actions"] = serde_json::json!([action]);
            PageView::decode(&serde_json::to_vec(view).unwrap())
        };
        let decoded = decode(&mut view, action(serde_json::json!({}))).unwrap();
        assert!(matches!(
            &decoded.browser_actions[0].intent,
            BrowserActionIntent::Act { tool, elements, .. }
                if tool == "fill" && elements[0].name == "Email address"
        ));
        assert_eq!(
            decoded.browser_actions[0].phase,
            BrowserActionPhase::Pending
        );
        let settled = serde_json::json!({"revision":4,"phase":"settled","outcome":"acted","result":"{\"filled\":1}"});
        let decoded = decode(&mut view, action(settled.clone())).unwrap();
        assert_eq!(
            decoded.browser_actions[0].result.as_deref(),
            Some("{\"filled\":1}")
        );
        for invalid in [
            serde_json::json!({"revision":2}),
            serde_json::json!({"tool":"observe"}),
            serde_json::json!({"elements":[{"reference":"e3","role":"textbox","name":"Email address"}]}),
            serde_json::json!({"elements":[]}),
            serde_json::json!({"result":"early"}),
            serde_json::json!({"revision":4,"phase":"settled","outcome":"acted"}),
            serde_json::json!({"revision":4,"phase":"settled","outcome":"verified","result":"x"}),
            serde_json::json!({"revision":4,"phase":"settled","outcome":"prevented"}),
            serde_json::json!({"destination":"http://127.0.0.1:43123/next"}),
        ] {
            assert!(
                decode(&mut view, action(invalid.clone())).is_err(),
                "accepted {invalid}"
            );
        }
        let mut old = action(serde_json::json!({}));
        for key in ["kind", "tool", "args", "elements"] {
            old.as_object_mut().unwrap().remove(key);
        }
        old["field_id"] = serde_json::json!("recipient_email");
        old["value"] = serde_json::json!("North");
        assert!(decode(&mut view, old).is_err());
    }

    #[test]
    fn decodes_configured_browser_route_and_rejects_invalid_configuration() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["browser_route"] =
            serde_json::json!({"kind":"messages","model":"fixture-browser-model"});
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert!(matches!(
            decoded.browser_route,
            Some(BrowserRouteView::Messages { .. })
        ));

        let fingerprint = "a".repeat(64);
        view["browser_route"] = serde_json::json!({
            "kind":"https_messages", "endpoint":"https://model.example/v1", "model":"model-1",
            "revision":1, "fingerprint":fingerprint
        });
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert!(matches!(
            decoded.browser_route,
            Some(BrowserRouteView::HttpsMessages { .. })
        ));
        view["runs"][0]["kind"] = serde_json::json!("browser");
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"https_messages", "endpoint":"https://model.example/v1", "model":"model-1",
            "revision":1, "fingerprint":fingerprint
        });
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert!(matches!(
            decoded.runs[0].route,
            RunRoute::HttpsMessages { .. }
        ));
        view["runs"][0]["route"]["extra"] = serde_json::json!(true);
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
        view["runs"][0]["route"] = serde_json::json!({"kind":"fixture"});
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
        view["runs"][0]["kind"] = serde_json::json!("summary");
        for (field, invalid) in [
            ("endpoint", "http://model.example"),
            ("endpoint", "https://user@model.example"),
            ("endpoint", "https://model.example?query=1"),
            ("endpoint", "https://model.example#fragment"),
            ("endpoint", "https://model.example:0/v1"),
            ("model", "model\n1"),
            ("model", "model name"),
        ] {
            view["browser_route"][field] = serde_json::json!(invalid);
            assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
        }
        view["browser_route"]["endpoint"] = serde_json::json!("https://model.example/v1");
        view["browser_route"]["revision"] = serde_json::json!(0);
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
        view["browser_route"]["revision"] = serde_json::json!(1);
        view["browser_route"]["extra"] = serde_json::json!(true);
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn decodes_configured_responses_route_and_rejects_invalid_configuration() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        let fingerprint = "a".repeat(64);
        view["responses_route"] = serde_json::json!({
            "kind":"https_responses", "endpoint":"https://api.example/v1", "model":"gpt-model",
            "revision":1, "fingerprint":fingerprint
        });
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert!(matches!(
            decoded.responses_route,
            Some(ResponsesRouteView::HttpsResponses { .. })
        ));

        view["runs"][0]["kind"] = serde_json::json!("browser");
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"https_responses", "endpoint":"https://api.example/v1", "model":"gpt-model",
            "revision":1, "fingerprint":fingerprint
        });
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert!(matches!(
            decoded.runs[0].route,
            RunRoute::HttpsResponses { .. }
        ));

        for (field, invalid) in [
            ("endpoint", "http://api.example/v1"),
            ("endpoint", "https://user@api.example/v1"),
            ("model", "gpt model"),
            ("fingerprint", "short"),
        ] {
            view["responses_route"][field] = serde_json::json!(invalid);
            assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
            view["responses_route"][field] = match field {
                "endpoint" => serde_json::json!("https://api.example/v1"),
                "model" => serde_json::json!("gpt-model"),
                _ => serde_json::json!(fingerprint),
            };
        }
        view["responses_route"]["revision"] = serde_json::json!(0);
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn accepts_flat_and_native_text_runs_with_strict_routes() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"text", "protocol":"responses", "endpoint":"https://api.example/v1",
            "model":"gpt-model", "revision":1, "fingerprint":"a".repeat(64)
        });
        assert!(matches!(
            PageView::decode(&serde_json::to_vec(&view).unwrap())
                .unwrap()
                .runs[0]
                .route,
            RunRoute::Text {
                protocol: Protocol::Responses,
                ..
            }
        ));

        let mut native = view.clone();
        native["runs"][0]["conversation"] = serde_json::json!([{
            "kind":"message", "key":{"kind":"observed","ordinal":1},
            "start":0, "end":native["runs"][0]["output"].as_str().unwrap().len(),
            "completion":"final_snapshot"
        }]);
        let decoded = PageView::decode(&serde_json::to_vec(&native).unwrap()).unwrap();
        let run = &decoded.runs[0];
        assert_eq!(run.output.entries().count(), 1);
        assert_eq!(
            RunView::decode_record(&serde_json::to_vec(&native["runs"][0]).unwrap(), &run.id)
                .unwrap()
                .output,
            run.output
        );

        for (path, value) in [
            (
                "runs/0/route/endpoint",
                serde_json::json!("http://api.example/v1"),
            ),
            ("runs/0/route/model", serde_json::json!("bad model")),
            ("runs/0/route/revision", serde_json::json!(0)),
            ("runs/0/route/fingerprint", serde_json::json!("short")),
            ("runs/0/route/protocol", serde_json::json!("unknown")),
            ("runs/0/question", serde_json::json!("unexpected")),
            (
                "runs/0/conversation",
                serde_json::json!([{"kind":"tool","id":"tool"}]),
            ),
            (
                "runs/0/activity",
                serde_json::json!([{"tool":"read","phase":"started"}]),
            ),
        ] {
            let mut invalid = view.clone();
            *invalid.pointer_mut(&format!("/{path}")).unwrap() = value;
            assert!(
                PageView::decode(&serde_json::to_vec(&invalid).unwrap()).is_err(),
                "{path}"
            );
        }
        let mut invalid = view.clone();
        invalid["runs"][0]["route"]["extra"] = serde_json::json!(true);
        assert!(PageView::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());

        view["runs"][0]["kind"] = serde_json::json!("answer");
        view["runs"][0]["question"] = serde_json::json!("What is reported?");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_ok());
        view["runs"][0]["question"] = serde_json::json!("   ");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn decodes_configured_chat_completions_route_and_browser_run() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        let route = serde_json::json!({
            "kind":"https_chat_completions", "endpoint":"https://model.example/v1",
            "model":"model-1", "revision":1, "fingerprint":"a".repeat(64)
        });
        view["chat_route"] = route.clone();
        assert!(matches!(
            PageView::decode(&serde_json::to_vec(&view).unwrap())
                .unwrap()
                .chat_route,
            Some(ChatRouteView::HttpsChatCompletions { .. })
        ));
        view["runs"][0]["kind"] = serde_json::json!("browser");
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"https_chat_completions", "endpoint":"https://model.example/v1",
            "model":"model-1", "revision":1, "fingerprint":"a".repeat(64)
        });
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_ok());
        view["chat_route"]["fingerprint"] = serde_json::json!("bad");
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }

    #[test]
    fn decodes_google_route_and_historical_run_without_current_configuration() {
        let mut view: serde_json::Value = serde_json::from_str(VALID).unwrap();
        let fingerprint = "b".repeat(64);
        view["google_route"] = serde_json::json!({
            "kind":"https_google_interactions", "endpoint":"https://generativelanguage.googleapis.com/v1/interactions",
            "model":"gemini-model", "revision":1, "fingerprint":fingerprint
        });
        assert!(matches!(
            PageView::decode(&serde_json::to_vec(&view).unwrap())
                .unwrap()
                .google_route,
            Some(GoogleRouteView::HttpsGoogleInteractions { .. })
        ));

        view["runs"][0]["kind"] = serde_json::json!("browser");
        view["runs"][0]["route"] = serde_json::json!({
            "kind":"https_google_interactions", "endpoint":"https://generativelanguage.googleapis.com/v1/interactions",
            "model":"gemini-model", "revision":1, "fingerprint":fingerprint
        });
        view.as_object_mut().unwrap().remove("google_route");
        let decoded = PageView::decode(&serde_json::to_vec(&view).unwrap()).unwrap();
        assert!(decoded.google_route.is_none());
        assert!(matches!(
            decoded.runs[0].route,
            RunRoute::HttpsGoogleInteractions { .. }
        ));

        view["google_route"] = serde_json::json!({
            "kind":"https_google_interactions", "endpoint":"https://generativelanguage.googleapis.com/v1/interactions",
            "model":"gemini-model", "revision":1, "fingerprint":fingerprint
        });
        for (field, invalid) in [
            (
                "endpoint",
                "http://generativelanguage.googleapis.com/v1/interactions",
            ),
            (
                "endpoint",
                "https://user@generativelanguage.googleapis.com/v1/interactions",
            ),
            ("model", "gemini model"),
            ("fingerprint", "short"),
        ] {
            view["google_route"][field] = serde_json::json!(invalid);
            assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
            view["google_route"][field] = match field {
                "endpoint" => {
                    serde_json::json!("https://generativelanguage.googleapis.com/v1/interactions")
                }
                "model" => serde_json::json!("gemini-model"),
                _ => serde_json::json!(fingerprint),
            };
        }
        view["google_route"]["revision"] = serde_json::json!(0);
        assert!(PageView::decode(&serde_json::to_vec(&view).unwrap()).is_err());
    }
}
