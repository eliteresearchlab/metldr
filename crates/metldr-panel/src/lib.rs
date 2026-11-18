use metldr_history_pages::WIRE_VERSION;

use std::{cell::RefCell, collections::BTreeSet};

use js_sys::{Array, Function, Object, Promise, Reflect};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::*};
use wasm_bindgen_futures::JsFuture;

// The service worker build (no `rune-ui`) validates native frames with the protocol decoder in
// `flow`, `history` and `model`; the rest of those modules is the chat client, built with Rune.
#[cfg(feature = "rune-ui")]
mod ag_ui;
#[cfg(feature = "rune-ui")]
mod browser_action_display;
mod browser_background;
#[cfg(feature = "rune-ui")]
mod composer;
#[cfg(feature = "rune-ui")]
mod execution_preference;
#[cfg_attr(not(feature = "rune-ui"), allow(dead_code))]
mod flow;
#[cfg_attr(not(feature = "rune-ui"), allow(dead_code))]
mod history;
#[cfg(feature = "rune-ui")]
mod interaction_audio;
#[cfg_attr(not(feature = "rune-ui"), allow(dead_code))]
mod model;
#[cfg(feature = "rune-ui")]
mod display;
#[cfg(feature = "rune-ui")]
mod preferences;
mod profile;
mod readiness;
mod reading;
#[cfg(feature = "rune-ui")]
mod run_display;
mod site;
#[cfg(feature = "rune-ui")]
mod ui_bridge;
use model::RunId;

const HOST: &str = "io.metldr.first_flow";
const PANEL_PORT: &str = "metldr-first-flow";
const MAX_FRAME: usize = 65_536;
const MAX_PAGE: usize = 8_192;

thread_local! {
    static BACKGROUND: RefCell<Background> = RefCell::new(Background::default());
}

#[derive(Default)]
struct Background {
    native: Option<JsValue>,
    readiness: readiness::Gate,
    panels: Vec<JsValue>,
    watches: Vec<PanelWatch>,
    next_cleanup: u64,
    stop_requests: BTreeSet<String>,
    stop_badge_epoch: u64,
}

#[wasm_bindgen]
pub fn stop_all_runs() {
    if let Err(error) = request_stop_all() {
        stop_report(
            &format!("Stop could not reach the companion: {}", js_message(&error)),
            "ERR",
        );
    }
}

fn request_stop_all() -> Result<(), JsValue> {
    if BACKGROUND.with(|state| state.borrow().native.is_none()) {
        connect_native()?;
    }
    let request = next_cleanup_request().ok_or_else(|| err("Stop request identity exhausted."))?;
    let frame = js_sys::JSON::parse(
        &json!({"v":WIRE_VERSION,"request":request,"command":{"op":"stop_all"}}).to_string(),
    )?;
    let (native, ready) = BACKGROUND
        .with(|state| {
            let mut state = state.borrow_mut();
            let native = state.native.clone().ok_or(())?;
            let ready = state.readiness.queue(frame.clone())?;
            state.stop_requests.insert(request.clone());
            Ok::<_, ()>((native, ready))
        })
        .map_err(|_| err("Companion request queue is unavailable."))?;
    if ready && let Err(error) = call(&native, "postMessage", &[frame]) {
        BACKGROUND.with(|state| {
            state.borrow_mut().stop_requests.remove(&request);
        });
        return Err(error);
    }
    stop_report("Sending stop request.", "...");
    let timeout = Closure::once_into_js(move || {
        if BACKGROUND.with(|state| state.borrow_mut().stop_requests.remove(&request)) {
            stop_report("Stop delivery was not confirmed.", "ERR");
        }
    });
    call(&js_sys::global(), "setTimeout", &[timeout, 6000.into()])?;
    Ok(())
}

fn stop_report(message: &str, badge: &str) {
    let epoch = BACKGROUND.with(|state| {
        let mut state = state.borrow_mut();
        state.stop_badge_epoch = state.stop_badge_epoch.wrapping_add(1);
        state.stop_badge_epoch
    });
    if let Ok(action) = chrome_api("action") {
        let _ = call(
            &action,
            "setBadgeText",
            &[js_sys::JSON::parse(&json!({"text":badge}).to_string()).unwrap_or(JsValue::NULL)],
        );
        let _ = call(
            &action,
            "setTitle",
            &[js_sys::JSON::parse(&json!({"title":message}).to_string()).unwrap_or(JsValue::NULL)],
        );
    }
    forward_panel(
        &json!({"v":WIRE_VERSION,"request":"stop-notice","kind":"stop_notice","message":message}),
    );
    if badge != "..." {
        let timeout = Closure::once_into_js(move || {
            if BACKGROUND.with(|state| state.borrow().stop_badge_epoch == epoch)
                && let Ok(action) = chrome_api("action")
            {
                let _ = call(
                    &action,
                    "setBadgeText",
                    &[js_sys::JSON::parse(r#"{"text":""}"#).unwrap()],
                );
                let _ = call(
                    &action,
                    "setTitle",
                    &[js_sys::JSON::parse(r#"{"title":"Open metldr"}"#).unwrap()],
                );
            }
        });
        let _ = call(&js_sys::global(), "setTimeout", &[timeout, 5000.into()]);
    }
}

struct PanelWatch {
    panel: JsValue,
    request: String,
    scope: metldr_history_pages::Scope,
    run: RunId,
    queued: bool,
}

#[wasm_bindgen]
pub fn accept_panel(port: JsValue) {
    let result = (|| -> Result<(), JsValue> {
        if string_property(&port, "name")? != PANEL_PORT {
            return Ok(());
        }
        let runtime = chrome_api("runtime")?;
        let extension_id = string_property(&runtime, "id")?;
        let origin = format!("chrome-extension://{extension_id}");
        let sender = get(&port, "sender")?;
        if string_property(&sender, "id")? != extension_id
            || string_property(&sender, "origin")? != origin
            || !panel_url_allowed(&origin, &string_property(&sender, "url")?)
        {
            let _ = call(&port, "disconnect", &[]);
            return Ok(());
        }
        if BACKGROUND.with(|state| state.borrow().native.is_none()) {
            connect_native()?;
        }
        if BACKGROUND.with(|state| state.borrow().panels.len() >= 16) {
            let _ = call(&port, "disconnect", &[]);
            return Ok(());
        }
        BACKGROUND.with(|state| state.borrow_mut().panels.push(port.clone()));
        let reply_port = port.clone();
        listen(&port, "onMessage", move |message| {
            let request = get(&message, "request").ok().and_then(|v| v.as_string());
            if let Err(error) = forward_request(&reply_port, message) {
                if let Some(request) = request {
                    let message: String = js_message(&error).chars().take(64).collect();
                    if let Ok(raw) = serde_json::to_string(
                        &json!({"v":crate::WIRE_VERSION,"request":request,"kind":"error","message":message}),
                    ) {
                        if let Ok(value) = js_sys::JSON::parse(&raw) {
                            let _ = call(&reply_port, "postMessage", &[value]);
                        }
                    }
                }
            }
        })?;
        let disconnected_port = port.clone();
        listen(&port, "onDisconnect", move |_| {
            remove_panel(&disconnected_port)
        })?;
        Ok(())
    })();
    if let Err(error) = result {
        web_sys::console::error_1(&error);
    }
}

fn forward_request(panel: &JsValue, message: JsValue) -> Result<(), JsValue> {
    let raw = js_sys::JSON::stringify(&message)?
        .as_string()
        .ok_or_else(|| err("Panel sent a non JSON message."))?;
    if raw.len() > MAX_FRAME {
        return Err(err("Panel request exceeds the 65536 byte frame limit."));
    }
    let parsed = js_sys::JSON::parse(&raw)?;
    let command: Value = serde_json::from_str(&raw).map_err(|_| err("Invalid panel request."))?;
    let watch =
        panel_subscription(&command, "watch").map_err(|_| err("Invalid panel watch request."))?;
    let unwatch = panel_subscription(&command, "unwatch")
        .map_err(|_| err("Invalid panel unwatch request."))?;
    let op = get(&get(&parsed, "command")?, "op")?.as_string();
    if matches!(op.as_deref(), Some("browser_reply" | "browser_event")) {
        return Err(err(
            "Browser transport messages cannot originate in the panel.",
        ));
    }
    if watch.as_ref().is_some_and(|(request, _, _)| {
        BACKGROUND.with(|b| {
            let watches = &b.borrow().watches;
            watches.len() >= 16
                || watch_id_in_use(watches.iter().map(|watch| watch.request.as_str()), request)
        })
    }) {
        return Err(err(
            "Native watch capacity reached or watch ID is already active.",
        ));
    }
    let native = match BACKGROUND.with(|b| b.borrow().native.clone()) {
        Some(native) => native,
        None => {
            let command = Reflect::get(&parsed, &JsValue::from_str("command"))?;
            let op = Reflect::get(&command, &JsValue::from_str("op"))?.as_string();
            if op.as_deref() != Some("view") {
                return Err(err(
                    "Native host is disconnected. Reconnect before starting work.",
                ));
            }
            connect_native()?;
            BACKGROUND
                .with(|b| b.borrow().native.clone())
                .ok_or_else(|| err("Native host did not connect."))?
        }
    };
    let queued = BACKGROUND.with(|b| {
        let mut background = b.borrow_mut();
        if background
            .native
            .as_ref()
            .is_none_or(|current| !Object::is(current, &native))
        {
            return Err(());
        }
        background.readiness.queue(parsed.clone())
    });
    let ready = match queued {
        Ok(ready) => ready,
        Err(()) => {
            return Err(err(
                "Native host is not ready or its request queue is full.",
            ));
        }
    };
    if ready {
        call(&native, "postMessage", &[parsed])?;
    }
    if let Some((request, scope, run)) = watch {
        BACKGROUND.with(|b| {
            let mut background = b.borrow_mut();
            background.watches.push(PanelWatch {
                panel: panel.clone(),
                request,
                scope,
                run,
                queued: !ready,
            });
        });
    }
    if let Some((request, scope, run)) = unwatch {
        BACKGROUND.with(|b| {
            b.borrow_mut().watches.retain(|entry| {
                !(Object::is(&entry.panel, panel)
                    && entry.request == request
                    && entry.scope == scope
                    && entry.run == run)
            })
        });
    }
    Ok(())
}
fn forward_native(message: JsValue) {
    readiness::native_message(message);
}
fn forward_panel(message: &Value) {
    let Ok(raw) = serde_json::to_string(message) else {
        return;
    };
    let Ok(value) = js_sys::JSON::parse(&raw) else {
        return;
    };
    for port in BACKGROUND.with(|b| b.borrow().panels.clone()) {
        let _ = call(&port, "postMessage", &[value.clone()]);
    }
}
fn connect_native() -> Result<(), JsValue> {
    let runtime = chrome_api("runtime")?;
    let native = call(&runtime, "connectNative", &[JsValue::from_str(HOST)])?;
    browser_background::connect(native.clone())?;
    set_background_native(native.clone());
    let sending = native.clone();
    listen(&native, "onMessage", move |message| {
        if BACKGROUND.with(|b| {
            b.borrow()
                .native
                .as_ref()
                .is_some_and(|current| Object::is(current, &sending))
        }) {
            forward_native(message);
        }
    })?;
    let disconnected = native.clone();
    listen(&native, "onDisconnect", move |_| {
        if !BACKGROUND.with(|b| {
            b.borrow()
                .native
                .as_ref()
                .is_some_and(|current| Object::is(current, &disconnected))
        }) {
            return;
        }
        browser_background::disconnect();
        forward_panel(&json!({"v":crate::WIRE_VERSION,"request":null,"kind":"disconnected"}));
        set_background_native(JsValue::NULL);
    })?;
    Ok(())
}
fn set_background_native(native: JsValue) {
    BACKGROUND.with(|b| {
        let mut background = b.borrow_mut();
        background.native = (!native.is_null()).then_some(native);
        if background.native.is_some() {
            background.readiness.connect();
        } else {
            background.readiness.disconnect();
            background.watches.clear();
        }
    });
}
fn remove_panel(port: &JsValue) {
    let (watches, native, ready) = BACKGROUND.with(|b| {
        let mut background = b.borrow_mut();
        background.panels.retain(|p| !Object::is(p, port));
        let watches: Vec<_> = background
            .watches
            .iter()
            .filter(|watch| Object::is(&watch.panel, port))
            .map(|watch| PanelWatch {
                panel: watch.panel.clone(),
                request: watch.request.clone(),
                scope: watch.scope.clone(),
                run: watch.run.clone(),
                queued: watch.queued,
            })
            .collect();
        background
            .watches
            .retain(|watch| !Object::is(&watch.panel, port));
        (
            watches,
            background.native.clone(),
            background.readiness.is_ready(),
        )
    });
    for watch in watches {
        if watch.queued {
            BACKGROUND.with(|b| b.borrow_mut().readiness.discard_request(&watch.request));
        } else if ready {
            if let Some(native) = native.as_ref() {
                let Some(id) = next_cleanup_request() else {
                    continue;
                };
                let scope = serde_json::to_value(watch.scope).unwrap_or(Value::Null);
                let frame = json!({"v":WIRE_VERSION,"request":id,"command":{"op":"conversation","scope":scope,"action":{"op":"unwatch","run":watch.run,"watch":watch.request}}});
                if let Ok(value) = js_sys::JSON::parse(&frame.to_string()) {
                    let _ = call(native, "postMessage", &[value]);
                }
            }
        }
    }
}

fn panel_subscription(
    command: &Value,
    operation: &str,
) -> Result<Option<(String, metldr_history_pages::Scope, RunId)>, ()> {
    let Some(conversation) = command.get("command") else {
        return Ok(None);
    };
    let Some(action) = conversation.get("action") else {
        return Ok(None);
    };
    if action.get("op").and_then(Value::as_str) != Some(operation) {
        return Ok(None);
    }
    let identity = if operation == "watch" {
        command.get("request")
    } else {
        action.get("watch")
    };
    let request = identity
        .and_then(Value::as_str)
        .filter(|id| valid_panel_request_id(id))
        .ok_or(())?;
    if conversation.get("op").and_then(Value::as_str) != Some("conversation") {
        return Err(());
    }
    let scope: metldr_history_pages::Scope =
        serde_json::from_value(conversation.get("scope").ok_or(())?.clone()).map_err(|_| ())?;
    scope.validate().map_err(|_| ())?;
    let run = serde_json::from_value(action.get("run").ok_or(())?.clone()).map_err(|_| ())?;
    Ok(Some((request.to_owned(), scope, run)))
}

fn next_cleanup_request() -> Option<String> {
    BACKGROUND.with(|b| {
        let mut background = b.borrow_mut();
        background.next_cleanup = background.next_cleanup.checked_add(1)?;
        Some(format!("cleanup-{}", background.next_cleanup))
    })
}

fn watch_id_in_use<'a>(requests: impl IntoIterator<Item = &'a str>, request: &str) -> bool {
    requests.into_iter().any(|active| active == request)
}

fn valid_panel_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
}

#[cfg(test)]
mod panel_watch_tests {
    use super::{panel_subscription, valid_panel_request_id, watch_id_in_use};
    use serde_json::json;

    #[test]
    fn panel_subscription_validates_identity_and_rejects_duplicate_watch_ids() {
        let store = "0123456789abcdef0123456789abcdef";
        let scope = json!({"store":store,"conversation":format!("{store}:1"),"epoch":"1"});
        let request = json!({"request":"panel-abc-1","command":{"op":"conversation","scope":scope,"action":{"op":"watch","run":format!("{store}:2")}}});
        let watch = panel_subscription(&request, "watch").unwrap().unwrap();
        assert_eq!(watch.0, "panel-abc-1");
        assert_eq!(watch.1.store, store);
        assert_eq!(watch.2.to_string(), format!("{store}:2"));
        assert!(watch_id_in_use([watch.0.as_str()], "panel-abc-1"));

        let malformed = json!({"request":"bad id","command":{"op":"conversation","scope":{"store":"bad"},"action":{"op":"watch","run":"bad"}}});
        assert!(panel_subscription(&malformed, "watch").is_err());
        assert!(!valid_panel_request_id("bad id"));

        let malformed_run = json!({"request":"panel-abc-2","command":{"op":"conversation","scope":scope,"action":{"op":"watch","run":"bad"}}});
        assert!(panel_subscription(&malformed_run, "watch").is_err());
    }
}
fn chrome_api(name: &str) -> Result<JsValue, JsValue> {
    let chrome = Reflect::get(&js_sys::global(), &JsValue::from_str("chrome"))?;
    Reflect::get(&chrome, &JsValue::from_str(name))
}
fn get(object: &JsValue, key: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &JsValue::from_str(key))
}
fn string_property(object: &JsValue, key: &str) -> Result<String, JsValue> {
    get(object, key)?
        .as_string()
        .ok_or_else(|| err(&format!("Missing string field {key}.")))
}
fn call(object: &JsValue, method: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let function = get(object, method)?.dyn_into::<Function>()?;
    let js_args = Array::new();
    for value in args {
        js_args.push(value);
    }
    function.apply(object, &js_args)
}
fn promise(value: JsValue) -> Result<JsFuture, JsValue> {
    Ok(JsFuture::from(value.dyn_into::<Promise>()?))
}
async fn await_call(object: &JsValue, method: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    promise(call(object, method, args)?)?.await
}
fn listen(
    target: &JsValue,
    event: &str,
    mut callback: impl FnMut(JsValue) + 'static,
) -> Result<(), JsValue> {
    let event = get(target, event)?;
    let add = get(&event, "addListener")?.dyn_into::<Function>()?;
    let listener = Closure::<dyn FnMut(JsValue)>::new(move |value| callback(value));
    add.call1(&event, listener.as_ref())?;
    listener.forget();
    Ok(())
}
fn err(message: &str) -> JsValue {
    JsValue::from_str(message)
}
fn js_message(error: &JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

fn panel_url_allowed(origin: &str, url: &str) -> bool {
    url.strip_prefix(origin) == Some("/chat.html")
}

#[cfg(test)]
mod panel_url_checks {
    #[test]
    fn only_exact_owned_application_pages_are_allowed() {
        let origin = "chrome-extension://owned";
        assert!(super::panel_url_allowed(origin, "chrome-extension://owned/chat.html"));
        for url in [
            "chrome-extension://owned/panel.html",
            "chrome-extension://owned/rune-chat.html",
            "chrome-extension://other/chat.html",
            "chrome-extension://owned/sandbox.html",
            "chrome-extension://owned/chat.html?route=panel",
            "chrome-extension://owned/chat.html#answer",
            "https://owned/chat.html",
        ] {
            assert!(!super::panel_url_allowed(origin, url));
        }
    }
}

#[cfg(test)]
mod leftovers {
    //! Measurement markers and debug prints stay out of the sources: a `buildperf` marker once
    //! reached a commit in theme.rs, and a `DBG` print nearly did in the Rune pointer code.
    const MARKERS: [&str; 4] = ["buildperf ", "eprintln!(\"DBG", "dbg!(", "console.log('DBG"];
    const ROOTS: [&str; 4] = ["crates", "vendor/rune/crates/rune/src", "scripts", "extension"];
    const SKIP: [&str; 4] = ["target", "node_modules", "pkg", "dist"];

    fn walk(dir: &std::path::Path, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for path in entries.flatten().map(|e| e.path()) {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if !SKIP.contains(&name) {
                    walk(&path, found);
                }
            } else if matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "js" | "mjs" | "ts")) {
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for (n, line) in text.lines().enumerate() {
                    // This check names the markers; it is the one place they may appear.
                    if MARKERS.iter().any(|m| line.contains(m)) && !line.contains("MARKERS") && !path.ends_with("src/lib.rs") {
                        found.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        }
    }

    #[test]
    fn no_measurement_markers_or_debug_prints_in_the_sources() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut found = Vec::new();
        for root in ROOTS {
            walk(&repo.join(root), &mut found);
        }
        assert!(found.is_empty(), "remove these measurement markers or debug prints before committing (measure in an ignored test instead):\n{}", found.join("\n"));
    }
}
