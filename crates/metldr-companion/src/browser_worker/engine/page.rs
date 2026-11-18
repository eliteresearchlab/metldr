//! Browse's page layer on the attached tab. Its commands reach Chrome only through the actor,
//! which sends the reads below for the sessions it owns, and the input set only while an approved
//! step's act window is open.

use super::{Job, Kind, Request, TIMEOUT};
use async_trait::async_trait;
use browse_core::{
    Browser, Cdp, CdpError, CdpEvent, HandoffResult, TabId, TabInfo, TabOptions, TabState,
    ToolContext,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::Instant,
};

/// Browse's page scripts, which the extension ships as browse/<name>.js: the InjectedScript
/// (326 KB) and the filled helpers object (90 KB), each over the 64 KB message limit.
const BUNDLED: [(&str, fn() -> &'static str); 2] = [
    ("injected-script", browse_page::injected_source),
    ("helpers", browse_page::helpers),
];

/// For each bundled script, the marker sent in its place: its name and SHA-256. The extension
/// puts its own copy back after checking the hash, so both sides always run the same script.
fn markers() -> &'static [String; 2] {
    static MARKERS: OnceLock<[String; 2]> = OnceLock::new();
    MARKERS.get_or_init(|| {
        BUNDLED.map(|(name, script)| {
            let digest = Sha256::digest(script().as_bytes());
            let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("/*metldr-bundled-script:{name}:{hex}*/")
        })
    })
}

/// `params` with any bundled script in the evaluated source replaced by its marker.
fn bundled(method: &str, mut params: Value) -> Value {
    let field = match method {
        "Runtime.callFunctionOn" => "functionDeclaration",
        "Runtime.evaluate" => "expression",
        _ => return params,
    };
    if let Some(Value::String(source)) = params.get_mut(field) {
        for ((_, script), marker) in BUNDLED.iter().zip(markers()) {
            let script = script();
            if source.len() >= script.len() && source.contains(script) {
                *source = source.replacen(script, marker, 1);
            }
        }
    }
    params
}

/// Setup the actor already did for the tab; Browse's own request for it is answered here.
const DONE: [&str; 5] = [
    "Page.enable",
    "Page.setLifecycleEventsEnabled",
    "Runtime.enable",
    "Target.setAutoAttach",
    "Emulation.setFocusEmulationEnabled",
];

/// What Browse's page layer may always send: frame, node and layout reads, and calls in its own
/// isolated world.
const READ: [&str; 10] = [
    "Page.getLayoutMetrics",
    "Page.getFrameTree",
    "Page.createIsolatedWorld",
    "Runtime.evaluate",
    "Runtime.callFunctionOn",
    "Runtime.releaseObject",
    "Runtime.releaseObjectGroup",
    "DOM.describeNode",
    "DOM.resolveNode",
    "DOM.getFrameOwner",
];

/// Trusted input and the scroll that brings a target into view: sent only inside an act window.
pub(crate) const INPUT: [&str; 6] = [
    "Input.dispatchMouseEvent",
    "Input.dispatchKeyEvent",
    "Input.insertText",
    "Input.dispatchDragEvent",
    "Input.setInterceptDrags",
    "DOM.scrollIntoViewIfNeeded",
];

pub(super) enum Route {
    Done,
    Send,
    Input,
    Refused,
}

pub(super) fn route(method: &str) -> Route {
    if DONE.contains(&method) {
        Route::Done
    } else if READ.contains(&method) {
        Route::Send
    } else if INPUT.contains(&method) {
        Route::Input
    } else {
        Route::Refused
    }
}

struct ActorCdp {
    requests: mpsc::Sender<Request>,
    events: broadcast::Sender<CdpEvent>,
}

#[async_trait]
impl Cdp for ActorCdp {
    async fn send(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, CdpError> {
        let deadline = Instant::now() + TIMEOUT;
        let (reply, rx) = oneshot::channel();
        let kind = Kind::Page {
            method: method.into(),
            session: session_id.map(Into::into),
            params: bundled(method, params),
        };
        self.requests
            .try_send(Request {
                kind,
                job: Job {
                    reply,
                    deadline,
                    admission: None,
                },
            })
            .map_err(|_| CdpError::Disconnected)?;
        match tokio::time::timeout_at(deadline, rx).await {
            Err(_) => Err(CdpError::Timeout(TIMEOUT.as_millis() as u64)),
            Ok(Err(_)) => Err(CdpError::TargetClosed),
            Ok(Ok(result)) => result.map_err(CdpError::Protocol),
        }
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
}

/// The one tab metldr attached, offered to Browse's tools; opening, closing and handing off tabs stay with metldr.
struct AttachedTab {
    tab: TabId,
    cdp: Arc<ActorCdp>,
    state: Arc<TabState>,
}

#[async_trait]
impl Browser for AttachedTab {
    async fn open_tab(&self, _url: Option<&str>, _options: &TabOptions) -> anyhow::Result<TabInfo> {
        anyhow::bail!("metldr works only in the attached tab")
    }
    async fn tabs(&self) -> anyhow::Result<Vec<TabInfo>> {
        Ok(Vec::new())
    }
    async fn close_tab(&self, _tab: TabId) -> anyhow::Result<()> {
        anyhow::bail!("metldr works only in the attached tab")
    }
    async fn cdp(&self, tab: TabId) -> anyhow::Result<Arc<dyn Cdp>> {
        anyhow::ensure!(tab == self.tab, "metldr works only in the attached tab");
        Ok(self.cdp.clone())
    }
    async fn state(&self, tab: TabId) -> anyhow::Result<Arc<TabState>> {
        anyhow::ensure!(tab == self.tab, "metldr works only in the attached tab");
        Ok(self.state.clone())
    }
    async fn handoff(&self, _tab: TabId, _message: &str) -> anyhow::Result<HandoffResult> {
        anyhow::bail!("metldr works only in the attached tab")
    }
    async fn human_input(&self, _tab: TabId) -> bool {
        false
    }
}

pub(super) struct Page {
    tab: TabId,
    context: ToolContext,
}

impl Page {
    pub(super) fn new(
        tab: i32,
        requests: mpsc::Sender<Request>,
        events: broadcast::Sender<CdpEvent>,
    ) -> Self {
        let tab = TabId::from(tab);
        let browser = AttachedTab {
            tab,
            cdp: Arc::new(ActorCdp { requests, events }),
            state: Arc::default(),
        };
        Self {
            tab,
            context: ToolContext {
                browser: Arc::new(browser),
                file_roots: Vec::new(),
            },
        }
    }

    /// Runs Browse's tool `name` on the attached tab and returns its text.
    pub(super) async fn tool(&self, name: &str, mut args: Value) -> crate::Result<String> {
        let tool = browse_page::tools()
            .into_iter()
            .find(|tool| tool.spec().name == name)
            .ok_or("the browser tool is missing")?;
        args["tab"] = json!(self.tab);
        let output = tool
            .call(self.context.clone(), args)
            .await
            .map_err(|error| error.to_string())?;
        let text: Vec<&str> = output
            .content
            .iter()
            .filter_map(|content| match content {
                browse_core::Content::Text { text } => Some(text.as_str()),
                browse_core::Content::Image { .. } => None,
            })
            .collect();
        if output.is_error {
            return Err(text.join("\n").into());
        }
        Ok(text.join("\n"))
    }

    /// The page outline from Browse's snapshot tool; `args` are the snapshot tool's options.
    pub(super) async fn snapshot(&self, args: Value) -> crate::Result<Value> {
        Ok(json!({ "kind": "page_snapshot", "text": self.tool("snapshot", args).await? }))
    }

    /// The role and accessible name of each of `ids` in the latest snapshot, `None` for an id it
    /// does not hold. Looks up only those ids: an action names one or two of a page's refs.
    pub(super) async fn refs(&self, ids: &[&str]) -> crate::Result<Vec<Option<(String, String)>>> {
        Ok(browse_page::snapshot_ref_names(&self.context, self.tab, ids)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|found| found.map(|(role, name)| (role, name.unwrap_or_default())))
            .collect())
    }

    /// Focuses the password or one-time code field `reference` with its text selected, so the
    /// consented secret replaces whatever it holds.
    pub(super) async fn focus_secret_field(&self, reference: &str) -> crate::Result<()> {
        let (page, element, _) = browse_page::resolve_element(
            &self.context,
            self.tab,
            Some(reference),
            None,
            std::time::Duration::from_secs(5),
        )
        .await
        .map_err(|error| error.to_string())?;
        let call = |method: &'static str| {
            let page = page.clone();
            let element = element.clone();
            async move {
                page.call_element(&element, method, Vec::new(), true)
                    .await
                    .map_err(|error| error.to_string())
            }
        };
        if call("isSecret").await? != json!(true) {
            return Err("that field is not a password or one-time code field".into());
        }
        call("focus").await?;
        call("selectText").await?;
        if call("isFocused").await? != json!(true) {
            return Err("the secret field did not take focus".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_scripts_cross_the_native_pipe_as_markers() {
        for (name, script) in BUNDLED {
            let script = script();
            let declaration = format!("function () {{\n{script}\nreturn true;\n}}");
            let params = bundled("Runtime.callFunctionOn", json!({ "functionDeclaration": declaration }));
            let sent = params["functionDeclaration"].as_str().unwrap();
            assert!(sent.len() < 200, "{name} still crosses in full");
            assert!(sent.contains(&format!("/*metldr-bundled-script:{name}:")));
            let evaluated = bundled("Runtime.evaluate", json!({ "expression": script }));
            let marker = evaluated["expression"].as_str().unwrap();
            assert!(marker.starts_with("/*metldr-bundled-script:") && marker.ends_with("*/"));
            assert!(sent.contains(marker));
        }
        let other = json!({ "functionDeclaration": "function () { return 1; }" });
        assert_eq!(bundled("Runtime.callFunctionOn", other.clone()), other);
    }
}
