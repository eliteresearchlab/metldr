use super::*;
/// The longest an approved step may hold its act window open.
const ACT_WINDOW_MS: f64 = 60_000.0;

/// Trusted input and the scroll that brings a target into view. The extension passes these only
/// while an approved step's act window is open on the attached tab.
pub(super) const INPUT: [&str; 6] = [
    "Input.dispatchMouseEvent",
    "Input.dispatchKeyEvent",
    "Input.insertText",
    "Input.dispatchDragEvent",
    "Input.setInterceptDrags",
    "DOM.scrollIntoViewIfNeeded",
];

/// Whether the attached tab has an open act window now.
pub(super) fn open(active: &Attachment) -> bool {
    active
        .acting
        .as_ref()
        .is_some_and(|(_, until)| js_sys::Date::now() < *until)
}

/// Opens the act window of one approved step after checking that its binding, document, target
/// and root frame are unchanged. Each approved action opens a window once.
pub(super) async fn begin(
    connection: u64,
    call_id: u64,
    run: &str,
    action: String,
    revision: u64,
    binding: navigation::Binding,
    url: String,
) -> Result<Value, String> {
    if !bounded_id(&action, 128)
        || revision == 0
        || !navigation::valid_binding(&binding)
        || binding.tab < 0
        || validate_url(&url).is_err()
    {
        return Err("Invalid page action request.".into());
    }
    let session = navigation::root_session(&binding.session);
    let epoch = reserve_call(connection, call_id, run, binding.tab, session)?;
    let reserved = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(active) = state.active.as_mut().filter(|active| active.epoch == epoch) else {
            return false;
        };
        if active.document != binding.document
            || active.url != url
            || active.grant_revision != binding.grant_revision
            || active.command_count != 0
            || active.pending_navigation.is_some()
            || active.confirming_navigation
            || active.acting.is_some()
            || !active.acts.insert(action.clone())
        {
            return false;
        }
        active.command_count = 1;
        true
    });
    if !reserved {
        return Err("Page action does not match the active browser grant.".into());
    }

    let result = async {
        verify(connection, epoch).await?;
        let target = send(
            connection,
            epoch,
            binding.tab,
            session,
            "Target.getTargetInfo",
            json!({}),
        )
        .await?;
        if target
            .get("targetInfo")
            .and_then(|value| value.get("targetId"))
            .and_then(Value::as_str)
            != Some(binding.target.as_str())
        {
            return Err("Browser target changed before the page action.".into());
        }
        let tree = send(
            connection,
            epoch,
            binding.tab,
            session,
            "Page.getFrameTree",
            json!({}),
        )
        .await?;
        let root = tree
            .get("frameTree")
            .and_then(|value| value.get("frame"))
            .ok_or_else(|| "Missing root frame before the page action.".to_owned())?;
        if root.get("id").and_then(Value::as_str) != Some(binding.frame.as_str())
            || root.get("url").and_then(Value::as_str) != Some(url.as_str())
        {
            return Err("Browser root frame changed before the page action.".into());
        }
        verify(connection, epoch).await?;
        Ok(json!({"open":true}))
    }
    .await;
    STATE.with(|state| {
        if let Some(active) = state
            .borrow_mut()
            .active
            .as_mut()
            .filter(|active| active.epoch == epoch)
        {
            active.command_count = active.command_count.saturating_sub(1);
            if result.is_ok() {
                active.acting = Some((action.clone(), js_sys::Date::now() + ACT_WINDOW_MS));
            }
        }
    });
    result
}

/// Closes the act window of `action` on the attached tab.
pub(super) fn end(run: &str, action: &str) -> Result<Value, String> {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let active = state
            .active
            .as_mut()
            .filter(|active| active.run == run)
            .ok_or_else(|| "Browser attachment is stale.".to_owned())?;
        if active
            .acting
            .as_ref()
            .is_some_and(|(open, _)| open == action)
        {
            active.acting = None;
        }
        Ok(json!({"closed":true}))
    })
}

async fn send(
    connection: u64,
    epoch: u64,
    tab: i32,
    session: Option<&str>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let debugger = chrome_api("debugger").map_err(message)?;
    let target = match session {
        Some(session) => json!({"tabId":tab,"sessionId":session}),
        None => json!({"tabId":tab}),
    };
    let result = await_call(
        &debugger,
        "sendCommand",
        &[
            parse(&target).map_err(message)?,
            JsValue::from_str(method),
            parse(&params).map_err(message)?,
        ],
    )
    .await;
    ensure_epoch(connection, epoch)?;
    to_value(&result.map_err(message)?)
}
