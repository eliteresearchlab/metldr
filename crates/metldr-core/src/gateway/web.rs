//! The gateway in the extension's service worker: the `metldr-gateway` port, route detection,
//! the encrypted key store and the replies streamed from each route.
//!
//! Keys are encrypted with AES-GCM under a non-extractable key that stays in IndexedDB; only the
//! ciphertext, bound to its route, sits in `chrome.storage.local`. Nothing here logs, syncs or
//! returns a key.

use super::{Kind, Message, Protocol, ROUTES, Request, Role, Route, RouteState};
use metldr_conversation::TextEvent;
use metldr_sse::Turn;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::{JsFuture, spawn_local};

const SETTINGS: &str = "metldr.gateway";
const KEYS: &str = "metldr.gateway.keys";
const PROBE_MS: u32 = 600;
/// Chrome's on-device model check can stay unresolved (headless Chrome, a model component that
/// is still registering); past this it counts as absent so `detect` always answers.
const CHROME_PROBE_MS: i32 = 3_000;
const CONNECT_MS: u32 = 10_000;
const LOCAL_TIMEOUT_SECS: u16 = 300;
const MODELS_BOUND: usize = 4 << 20;

#[wasm_bindgen]
extern "C" {
    pub type Port;
    #[wasm_bindgen(method, catch, js_name = postMessage)]
    fn post_message(this: &Port, message: &JsValue) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = onMessage)]
    fn on_message(this: &Port) -> ChromeEvent;
    #[wasm_bindgen(method, getter, js_name = onDisconnect)]
    fn on_disconnect(this: &Port) -> ChromeEvent;

    type ChromeEvent;
    #[wasm_bindgen(method, js_name = addListener)]
    fn add_listener(this: &ChromeEvent, listener: &js_sys::Function);

    #[wasm_bindgen(catch, js_namespace = ["chrome", "storage", "local"], js_name = get)]
    fn storage_get(keys: &JsValue) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, js_namespace = ["chrome", "storage", "local"], js_name = set)]
    fn storage_set(items: &JsValue) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(js_name = fetch)]
    fn fetch(input: &str, init: &web_sys::RequestInit) -> js_sys::Promise;
}

type Error = String;

fn err(error: JsValue) -> Error {
    error
        .as_string()
        .unwrap_or_else(|| "The browser refused the request.".into())
}

fn to_js(value: &Value) -> JsValue {
    js_sys::JSON::parse(&value.to_string()).unwrap_or(JsValue::NULL)
}

fn from_js(value: &JsValue) -> Value {
    js_sys::JSON::stringify(value)
        .ok()
        .and_then(|text| text.as_string())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

fn get(target: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(target, &name.into()).unwrap_or(JsValue::UNDEFINED)
}

// ---------- settings in chrome.storage.local ----------

async fn read(key: &str) -> Result<Value, Error> {
    let found = JsFuture::from(storage_get(&key.into()).map_err(err)?)
        .await
        .map_err(err)?;
    Ok(from_js(&found)[key].clone())
}

async fn write(key: &str, value: Value) -> Result<(), Error> {
    let items = to_js(&json!({ key: value }));
    JsFuture::from(storage_set(&items).map_err(err)?)
        .await
        .map_err(err)?;
    Ok(())
}

/// The saved endpoint and model of every route, and the route page tools use.
async fn settings() -> Value {
    let mut settings = read(SETTINGS)
        .await
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if settings["preferred"].is_null() {
        // The settings of the earlier page tools, read in place and left as they are.
        let legacy = from_js(
            &JsFuture::from(
                storage_get(&to_js(&json!(["preferredProvider", "selectedModel"])))
                    .unwrap_or_else(|_| js_sys::Promise::resolve(&JsValue::NULL)),
            )
            .await
            .unwrap_or(JsValue::NULL),
        );
        if legacy["preferredProvider"] == "ollama" {
            settings["preferred"] = json!({"route": "ollama", "model": legacy["selectedModel"]});
        } else if legacy["preferredProvider"] == "chrome-ai" {
            settings["preferred"] = json!({"route": "chrome-ai"});
        }
    }
    settings
}

fn saved<'a>(settings: &'a Value, route: &str, field: &str) -> Option<&'a str> {
    settings["routes"][route][field]
        .as_str()
        .filter(|value| !value.is_empty())
}

mod dictionary;
mod language;
mod model;
mod stats;
mod global_chat;
mod page_summary;
mod search_store;
mod summary_gate;
mod word;

// ---------- the key store ----------

mod keys {
    use super::*;
    use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbRequest, IdbTransactionMode};

    const DATABASE: &str = "metldr-gateway";
    const STORE: &str = "crypto";
    const KEY: &str = "aes";

    pub(super) async fn done(request: &IdbRequest) -> Result<JsValue, Error> {
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            let ok = request.clone();
            let success = Closure::once_into_js(move || {
                let _ = resolve.call1(&JsValue::NULL, &ok.result().unwrap_or(JsValue::UNDEFINED));
            });
            let failure = Closure::once_into_js(move || {
                let _ = reject.call1(&JsValue::NULL, &"The key store could not be opened.".into());
            });
            request.set_onsuccess(Some(success.unchecked_ref()));
            request.set_onerror(Some(failure.unchecked_ref()));
        });
        JsFuture::from(promise).await.map_err(err)
    }

    async fn database() -> Result<IdbDatabase, Error> {
        let factory: web_sys::IdbFactory = get(&js_sys::global(), "indexedDB")
            .dyn_into()
            .map_err(|_| "No IndexedDB here.")?;
        let open: IdbOpenDbRequest = factory.open_with_u32(DATABASE, 1).map_err(err)?;
        let upgrade = Closure::once_into_js({
            let open = open.clone();
            move || {
                if let Ok(result) = open.result()
                    && let Ok(db) = result.dyn_into::<IdbDatabase>()
                    && !db.object_store_names().contains(STORE)
                {
                    let _ = db.create_object_store(STORE);
                }
            }
        });
        open.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
        done(&open)
            .await?
            .dyn_into()
            .map_err(|_| "The key store could not be opened.".into())
    }

    fn subtle() -> Result<web_sys::SubtleCrypto, Error> {
        let crypto: web_sys::Crypto = get(&js_sys::global(), "crypto")
            .dyn_into()
            .map_err(|_| "No WebCrypto here.")?;
        Ok(crypto.subtle())
    }

    /// The device key: generated once, never extractable, kept in IndexedDB.
    async fn device_key() -> Result<web_sys::CryptoKey, Error> {
        let db = database().await?;
        let store = |mode| {
            db.transaction_with_str_and_mode(STORE, mode)
                .and_then(|tx| tx.object_store(STORE))
                .map_err(err)
        };
        if let Ok(key) = done(
            &store(IdbTransactionMode::Readonly)?
                .get(&KEY.into())
                .map_err(err)?,
        )
        .await?
        .dyn_into()
        {
            return Ok(key);
        }
        let usages = js_sys::Array::of2(&"encrypt".into(), &"decrypt".into());
        let algorithm = to_js(&json!({"name": "AES-GCM", "length": 256}));
        let generate = subtle()?
            .generate_key_with_object(algorithm.unchecked_ref(), false, &usages)
            .map_err(err)?;
        let key: web_sys::CryptoKey = JsFuture::from(generate)
            .await
            .map_err(err)?
            .dyn_into()
            .map_err(|_| "No key.")?;
        // `add` refuses a second key, so two first connects cannot leave a key that sealed
        // nothing; the loser reads the winner's.
        let added = store(IdbTransactionMode::Readwrite)?
            .add_with_key(&key, &KEY.into())
            .map_err(err)?;
        if done(&added).await.is_ok() {
            return Ok(key);
        }
        done(
            &store(IdbTransactionMode::Readonly)?
                .get(&KEY.into())
                .map_err(err)?,
        )
        .await?
        .dyn_into()
        .map_err(|_| "The key store could not be opened.".into())
    }

    fn params(route: &str, iv: &[u8]) -> js_sys::Object {
        let params = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&params, &"name".into(), &"AES-GCM".into());
        let _ = js_sys::Reflect::set(&params, &"iv".into(), &js_sys::Uint8Array::from(iv));
        // The route is bound into the ciphertext, so a key cannot be replayed on another route.
        let _ = js_sys::Reflect::set(
            &params,
            &"additionalData".into(),
            &js_sys::Uint8Array::from(route.as_bytes()),
        );
        params
    }

    pub async fn save(route: &str, secret: &str) -> Result<(), Error> {
        let key = device_key().await?;
        let mut iv = [0u8; 12];
        let crypto: web_sys::Crypto = get(&js_sys::global(), "crypto").unchecked_into();
        crypto
            .get_random_values_with_u8_array(&mut iv)
            .map_err(err)?;
        let data = js_sys::Uint8Array::from(secret.as_bytes());
        let sealed = subtle()?
            .encrypt_with_object_and_buffer_source(&params(route, &iv), &key, &data)
            .map_err(err)?;
        let sealed = js_sys::Uint8Array::new(&JsFuture::from(sealed).await.map_err(err)?).to_vec();
        let mut all = read(KEYS)
            .await
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        all[route] = json!({"iv": base64(&iv), "data": base64(&sealed)});
        write(KEYS, all).await
    }

    pub async fn load(route: &str) -> Result<Option<String>, Error> {
        let all = read(KEYS).await?;
        let (Some(iv), Some(data)) = (all[route]["iv"].as_str(), all[route]["data"].as_str())
        else {
            return Ok(None);
        };
        let (iv, data) = (
            unbase64(iv).ok_or("A saved key is damaged.")?,
            unbase64(data).ok_or("A saved key is damaged.")?,
        );
        let key = device_key().await?;
        let opened = subtle()?
            .decrypt_with_object_and_u8_array(&params(route, &iv), &key, &data)
            .map_err(err)?;
        let opened = js_sys::Uint8Array::new(
            &JsFuture::from(opened)
                .await
                .map_err(|_| "A saved key could not be opened; connect it again.")?,
        )
        .to_vec();
        String::from_utf8(opened)
            .map(Some)
            .map_err(|_| "A saved key is damaged.".into())
    }

    pub async fn has(route: &str) -> bool {
        read(KEYS).await.is_ok_and(|all| all[route].is_object())
    }

    pub async fn forget(route: &str) -> Result<(), Error> {
        let mut all = read(KEYS).await?;
        if let Some(map) = all.as_object_mut()
            && map.remove(route).is_some()
        {
            write(KEYS, all).await?;
        }
        Ok(())
    }

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn base64(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for group in bytes.chunks(3) {
            let n = group
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= group.len() {
                    ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    }

    fn unbase64(text: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(text.len() / 4 * 3);
        for group in text.as_bytes().chunks(4) {
            let mut n = 0u32;
            let mut len = 0usize;
            for &c in group.iter().filter(|&&c| c != b'=') {
                n |= (ALPHABET.iter().position(|&a| a == c)? as u32) << (18 - 6 * len);
                len += 1;
            }
            out.extend((0..len.saturating_sub(1)).map(|i| (n >> (16 - 8 * i)) as u8));
        }
        Some(out)
    }
}

// ---------- reaching a route ----------

/// A route with what it needs to answer: base address, model and key.
#[derive(Clone)]
struct Target {
    route: &'static Route,
    base: String,
    model: String,
    key: Option<String>,
}

fn base(route: &Route, settings: &Value) -> Option<String> {
    match (route.kind, route.id) {
        (_, "chat") | (Kind::Local, _) => saved(settings, route.id, "endpoint")
            .map(str::to_owned)
            .or_else(|| (!route.base.is_empty()).then(|| route.base.into())),
        _ => Some(route.base.into()),
    }
}

fn auth_headers(route: &Route, key: Option<&str>) -> Vec<(&'static str, String)> {
    let Some(key) = key else { return Vec::new() };
    match route.protocol {
        Protocol::Messages => vec![
            ("x-api-key", key.into()),
            ("anthropic-version", "2023-06-01".into()),
        ],
        Protocol::GoogleInteractions => vec![("x-goog-api-key", key.into())],
        _ => vec![("authorization", format!("Bearer {key}"))],
    }
}

/// GETs a JSON document within `ms`; any failure is the route being unreachable.
async fn get_json(
    url: &str,
    headers: &[(&'static str, String)],
    ms: u32,
    bound: usize,
) -> Result<Vec<u8>, u16> {
    let init = web_sys::RequestInit::new();
    init.set_method("GET");
    let list = web_sys::Headers::new().map_err(|_| 0u16)?;
    for (name, value) in headers {
        list.set(name, value).map_err(|_| 0u16)?;
    }
    init.set_headers(&list);
    init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(ms)));
    init.set_redirect(web_sys::RequestRedirect::Manual);
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_cache(web_sys::RequestCache::NoStore);
    let response: web_sys::Response = JsFuture::from(fetch(url, &init))
        .await
        .map_err(|_| 0u16)?
        .unchecked_into();
    if response.status() != 200 {
        return Err(response.status());
    }
    let body = JsFuture::from(response.array_buffer().map_err(|_| 0u16)?)
        .await
        .map_err(|_| 0u16)?;
    let body = js_sys::Uint8Array::new(&body);
    if body.length() as usize > bound {
        return Err(0);
    }
    Ok(body.to_vec())
}

async fn models(route: &Route, base: &str, key: Option<&str>, ms: u32) -> Result<Vec<String>, u16> {
    let body = get_json(
        &format!("{base}{}", route.models),
        &auth_headers(route, key),
        ms,
        MODELS_BOUND,
    )
    .await?;
    Ok(super::model_names(&body))
}

fn language_model() -> Option<JsValue> {
    Some(get(&js_sys::global(), "LanguageModel")).filter(|value| value.is_function())
}

async fn chrome_ai_state() -> &'static str {
    let Some(model) = language_model() else {
        return "absent";
    };
    let Ok(check) = js_sys::Reflect::get(&model, &"availability".into())
        .and_then(|f| f.unchecked_into::<js_sys::Function>().call0(&model))
    else {
        return "absent";
    };
    let timer = js_sys::Promise::new(&mut |resolve, _| {
        if let Ok(set_timeout) = get(&js_sys::global(), "setTimeout").dyn_into::<js_sys::Function>()
        {
            let _ = set_timeout.call2(&JsValue::UNDEFINED, &resolve, &CHROME_PROBE_MS.into());
        }
    });
    let check = js_sys::Promise::race(&js_sys::Array::of2(&check, &timer));
    match JsFuture::from(check)
        .await
        .ok()
        .and_then(|v| v.as_string())
        .as_deref()
    {
        Some("available") => "ready",
        Some("downloadable") => "downloadable",
        Some("downloading") => "downloading",
        _ => "absent",
    }
}

/// Picks a default model when the user has not: a known small, fast one if offered, else the
/// first listed.
fn default_model(route: &Route, models: &[String]) -> Option<String> {
    const LIKED: &[&str] = &[
        "gpt-5-mini",
        "gpt-4.1-mini",
        "claude-haiku",
        "gemini-2.5-flash",
        "gemini-flash",
    ];
    let liked = LIKED
        .iter()
        .find_map(|prefix| models.iter().find(|m| m.starts_with(prefix)));
    // Embedding models are listed beside chat models on local servers but cannot answer.
    let chat = models.iter().find(|m| {
        !["embed", "minilm", "bge-"]
            .iter()
            .any(|part| m.contains(part))
    });
    match route.protocol {
        Protocol::ChromeAi => None,
        _ => liked.or(chat).cloned(),
    }
}

async fn state(route: &'static Route, settings: &Value) -> RouteState {
    let base = base(route, settings);
    let mut state = RouteState {
        route: route.id,
        kind: route.kind,
        state: "absent",
        destination: super::destination(route, base.as_deref().unwrap_or_default()),
        endpoint: base.clone().filter(|_| route.kind != Kind::Builtin),
        models: Vec::new(),
        model: saved(settings, route.id, "model").map(str::to_owned),
    };
    state.state = match route.kind {
        Kind::Builtin => chrome_ai_state().await,
        Kind::Local => {
            match models(route, base.as_deref().unwrap_or_default(), None, PROBE_MS).await {
                Ok(found) => {
                    state.models = found;
                    "ready"
                }
                Err(_) => "absent",
            }
        }
        Kind::Key if base.is_none() => "needs-key",
        Kind::Key if route.id == "chat" || keys::has(route.id).await => "ready",
        Kind::Key => "needs-key",
    };
    if state.model.is_none() && route.kind == Kind::Local {
        state.model = default_model(route, &state.models);
    }
    state
}

async fn detect() -> Value {
    let settings = Rc::new(settings().await);
    let probes = ROUTES.iter().filter(|route| route.protocol.answers()).map(|route| {
        let settings = settings.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            Ok(to_js(
                &serde_json::to_value(state(route, &settings).await).unwrap_or(Value::Null),
            ))
        })
    });
    let all = js_sys::Promise::all(&probes.collect::<js_sys::Array>());
    let routes = JsFuture::from(all)
        .await
        .map(|found| from_js(&found))
        .unwrap_or(Value::Null);
    json!({"routes": routes, "preferred": settings["preferred"]})
}

async fn connect(
    id: &str,
    key: Option<String>,
    endpoint: Option<String>,
    model: Option<String>,
) -> Result<RouteState, Error> {
    let route = super::route(id).ok_or("Unknown route.")?;
    let mut settings = settings().await;
    let base = match (route.kind, endpoint.as_deref()) {
        (Kind::Builtin, _) => String::new(),
        (_, Some(endpoint)) if route.id == "chat" || route.kind == Kind::Local => {
            super::custom_base(endpoint)?
        }
        _ => base(route, &settings).ok_or("Add the address of your endpoint.")?,
    };
    let key = key
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
    if route.kind == Kind::Key && route.id != "chat" && key.is_none() {
        return Err("Paste your API key.".into());
    }
    if key
        .as_deref()
        .is_some_and(|key| key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err("That key has characters a key cannot have.".into());
    }
    let listed = match route.kind {
        Kind::Builtin if chrome_ai_state().await == "absent" => {
            return Err("Chrome's built-in AI is not available in this browser.".into());
        }
        Kind::Builtin => Vec::new(),
        _ => match models(route, &base, key.as_deref(), CONNECT_MS).await {
            Ok(found) => found,
            Err(401 | 403) => return Err("The service did not accept this key.".into()),
            Err(_) if route.kind == Kind::Local => {
                return Err("Nothing answered at that address. Is the server running?".into());
            }
            Err(_) if route.id == "chat" && model.is_some() => Vec::new(),
            Err(_) => return Err("The service could not be reached.".into()),
        },
    };
    let model = model.or_else(|| default_model(route, &listed));
    if let Some(key) = &key {
        keys::save(route.id, key).await?;
    }
    let custom = route.id == "chat" || route.kind == Kind::Local;
    let entry = json!({"endpoint": custom.then_some(&base), "model": model});
    settings["routes"][route.id] = entry;
    if settings["preferred"].is_null() || settings["preferred"]["route"] == route.id {
        settings["preferred"] = json!({"route": route.id, "model": model});
    }
    write(SETTINGS, settings.clone()).await?;
    let mut state = state(route, &settings).await;
    if state.models.is_empty() {
        state.models = listed;
    }
    Ok(state)
}

async fn forget(id: &str) -> Result<(), Error> {
    let route = super::route(id).ok_or("Unknown route.")?;
    keys::forget(route.id).await?;
    let mut settings = settings().await;
    if let Some(routes) = settings["routes"].as_object_mut() {
        routes.remove(route.id);
    }
    if settings["preferred"]["route"] == route.id {
        settings["preferred"] = Value::Null;
    }
    write(SETTINGS, settings).await
}

async fn prefer(id: &str, model: String) -> Result<(), Error> {
    let route = super::route(id).ok_or("Unknown route.")?;
    if model.len() > 128 || !model.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("That model name is not valid.".into());
    }
    let mut settings = settings().await;
    settings["routes"][route.id]["model"] = json!(model);
    settings["preferred"] = json!({"route": route.id, "model": model});
    write(SETTINGS, settings).await
}

/// With nothing chosen yet, whatever already works: Chrome's built-in AI, then a local server.
async fn first_ready(settings: &Value) -> Option<&'static Route> {
    for route in ROUTES.iter().filter(|route| route.kind != Kind::Key) {
        if state(route, settings).await.state == "ready" {
            return Some(route);
        }
    }
    None
}

async fn target(id: Option<&str>, model: Option<String>) -> Result<Target, Error> {
    let settings = settings().await;
    let route = match id.or_else(|| settings["preferred"]["route"].as_str()) {
        Some(id) => super::route(id).ok_or("Unknown route.")?,
        None => first_ready(&settings)
            .await
            .ok_or("Set up where metldr's AI runs first.")?,
    };
    let model = model
        .or_else(|| saved(&settings, route.id, "model").map(str::to_owned))
        .or_else(|| {
            (settings["preferred"]["route"] == route.id)
                .then(|| settings["preferred"]["model"].as_str().map(str::to_owned))
                .flatten()
        });
    let base = base(route, &settings).unwrap_or_default();
    let model = match (route.kind, model) {
        (Kind::Builtin, _) => String::new(),
        (_, Some(model)) => model,
        // A local server that was found but never set up answers with the model detect showed.
        (Kind::Local, None) => {
            let listed = models(route, &base, None, CONNECT_MS)
                .await
                .map_err(|_| "Nothing answered at that address. Is the server running?")?;
            default_model(route, &listed).ok_or("This server has no models yet.")?
        }
        (Kind::Key, None) => return Err("Choose a model first.".into()),
    };
    let key = match route.kind {
        Kind::Key => keys::load(route.id).await?,
        _ => None,
    };
    if route.kind == Kind::Key && route.id != "chat" && key.is_none() {
        return Err("Add your API key for this service first.".into());
    }
    Ok(Target {
        route,
        base,
        model,
        key,
    })
}

/// Streams one reply, calling `emit` with each piece of text; returns the full text and usage.
/// A JSON `schema` constrains Chrome's model directly and is stated in the instructions for
/// every other route.
async fn reply(
    target: Target,
    system: &str,
    messages: &[Message],
    schema: Option<&Value>,
    stop: &AtomicBool,
    emit: &mut dyn FnMut(&str),
) -> Result<(String, Value), Error> {
    let stated;
    let system = match schema {
        Some(schema) if target.route.protocol != Protocol::ChromeAi => {
            stated = format!(
                "{system}\n\nRespond with only JSON that matches this JSON schema:\n{schema}"
            );
            stated.as_str()
        }
        _ => system,
    };
    // A trimmed history can open with a reply; providers such as Anthropic refuse that.
    let messages = &messages[messages
        .iter()
        .position(|m| m.role == Role::User)
        .unwrap_or(0)..];
    let turns: Vec<Turn<'_>> = messages
        .iter()
        .map(|m| Turn {
            assistant: m.role == Role::Assistant,
            text: &m.content,
        })
        .collect();
    let mut text = String::new();
    let mut on_event = |event: TextEvent| -> metldr_chat_completions::Result<()> {
        match event {
            TextEvent::Delta(delta) => {
                text.push_str(&delta);
                emit(&delta);
            }
            TextEvent::Final(_) => {}
        }
        Ok(())
    };
    let Target {
        route,
        base,
        model,
        key,
    } = target;
    let url = format!("{base}{}", route.stream);
    let usage = match route.protocol {
        Protocol::SystemOne => return Err("A decision route cannot answer.".into()),
        Protocol::ChromeAi => {
            chrome_ai(system, messages, schema, stop, &mut on_event).await?;
            None
        }
        Protocol::ChatCompletions => {
            let timeout = if route.kind == Kind::Local {
                LOCAL_TIMEOUT_SECS
            } else {
                metldr_chat_completions::DEFAULT_TIMEOUT_SECS
            };
            let client =
                metldr_chat_completions::Client::new(&url, &model, key.as_deref(), None, timeout)
                    .map_err(|e| e.to_string())?;
            Some(
                client
                    .turns(system, &turns, stop, &mut on_event)
                    .await
                    .map_err(|e| e.to_string())?,
            )
        }
        Protocol::Messages => {
            let client = metldr_messages::Client::new(
                &url,
                &model,
                key.as_deref().unwrap_or_default(),
                None,
            )
            .map_err(|e| e.to_string())?;
            Some(
                client
                    .turns(system, &turns, stop, &mut on_event)
                    .await
                    .map_err(|e| e.to_string())?,
            )
        }
        Protocol::Responses => {
            let client = metldr_responses::Client::new(
                &url,
                &model,
                key.as_deref().unwrap_or_default(),
                None,
            )
            .map_err(|e| e.to_string())?;
            Some(
                client
                    .turns(system, &turns, stop, &mut on_event)
                    .await
                    .map_err(|e| e.to_string())?,
            )
        }
        Protocol::GoogleInteractions => {
            let client = metldr_google_interactions::Client::new(
                &url,
                &model,
                key.as_deref().unwrap_or_default(),
                None,
            )
            .map_err(|e| e.to_string())?;
            Some(
                client
                    .turns(system, &turns, stop, &mut on_event)
                    .await
                    .map_err(|e| e.to_string())?,
            )
        }
    };
    drop(on_event);
    let usage = usage.map(|usage| json!({"input": usage.input, "output": usage.output}));
    Ok((text, usage.unwrap_or(Value::Null)))
}

/// Chrome's built-in model, streamed in this worker; stop destroys the session.
async fn chrome_ai(
    system: &str,
    messages: &[Message],
    schema: Option<&Value>,
    stop: &AtomicBool,
    emit: &mut impl FnMut(TextEvent) -> metldr_chat_completions::Result<()>,
) -> Result<(), Error> {
    let model = language_model().ok_or("Chrome's built-in AI is not available in this browser.")?;
    let (last, earlier) = messages.split_last().ok_or("Nothing to answer.")?;
    let mut prompts = vec![json!({"role": "system", "content": system})];
    for message in earlier {
        let role = if message.role == Role::User {
            "user"
        } else {
            "assistant"
        };
        prompts.push(json!({"role": role, "content": message.content}));
    }
    let options = to_js(&json!({
        "initialPrompts": prompts,
        "expectedInputs": [{"type": "text"}],
        "expectedOutputs": [{"type": "text"}]
    }));
    let create: js_sys::Function = get(&model, "create").unchecked_into();
    let session = JsFuture::from(
        create
            .call1(&model, &options)
            .map_err(err)?
            .unchecked_into::<js_sys::Promise>(),
    )
    .await
    .map_err(|_| "Chrome's built-in AI could not start.")?;
    let result = async {
        let stream: js_sys::Function = get(&session, "promptStreaming").unchecked_into();
        let options =
            to_js(&schema.map_or(json!({}), |schema| json!({"responseConstraint": schema})));
        let readable: web_sys::ReadableStream = stream
            .call2(&session, &last.content.as_str().into(), &options)
            .map_err(err)?
            .unchecked_into();
        let reader: web_sys::ReadableStreamDefaultReader = readable.get_reader().unchecked_into();
        loop {
            if stop.load(Ordering::Acquire) {
                let _ = reader.cancel();
                return Err("stopped".to_string());
            }
            let read = JsFuture::from(reader.read())
                .await
                .map_err(|_| "Chrome's built-in AI stopped answering.")?;
            if get(&read, "done").is_truthy() {
                return Ok(());
            }
            if let Some(piece) = get(&read, "value").as_string() {
                emit(TextEvent::Delta(piece)).map_err(|e| e.to_string())?;
            }
        }
    }
    .await;
    if let Ok(destroy) = get(&session, "destroy").dyn_into::<js_sys::Function>() {
        let _ = destroy.call0(&session);
    }
    result
}

// ---------- the port ----------

struct Connection {
    port: Port,
    streams: HashMap<u64, Rc<AtomicBool>>,
    _listeners: Vec<Closure<dyn FnMut(JsValue)>>,
}

thread_local! {
    static CONNECTIONS: RefCell<HashMap<u32, Connection>> = RefCell::new(HashMap::new());
    static NEXT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn send(connection: u32, message: Value) {
    CONNECTIONS.with_borrow(|all| {
        if let Some(c) = all.get(&connection) {
            let _ = c.port.post_message(&to_js(&message));
        }
    });
}

/// Serves one `metldr-gateway` port until it disconnects; its streams stop with it. `early`
/// holds the messages that arrived while this module loaded, in order.
#[wasm_bindgen]
pub fn gateway_accept(port: Port, early: js_sys::Array) {
    let connection = NEXT.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    let on_message = Closure::<dyn FnMut(JsValue)>::new(move |message: JsValue| {
        handle(connection, from_js(&message))
    });
    let on_disconnect = Closure::<dyn FnMut(JsValue)>::new(move |_| {
        if let Some(gone) = CONNECTIONS.with_borrow_mut(|all| all.remove(&connection)) {
            for stop in gone.streams.values() {
                stop.store(true, Ordering::Release);
            }
        }
    });
    port.on_message()
        .add_listener(on_message.as_ref().unchecked_ref());
    port.on_disconnect()
        .add_listener(on_disconnect.as_ref().unchecked_ref());
    let state = Connection {
        port,
        streams: HashMap::new(),
        _listeners: vec![on_message, on_disconnect],
    };
    CONNECTIONS.with_borrow_mut(|all| all.insert(connection, state));
    for message in early.iter() {
        handle(connection, from_js(&message));
    }
}

fn handle(connection: u32, message: Value) {
    let request: Request = match serde_json::from_value(message.clone()) {
        Ok(request) => request,
        Err(_) => {
            let id = message["id"].as_u64().unwrap_or(0);
            return send(
                connection,
                json!({"id": id, "ok": false, "error": "That gateway request is not valid."}),
            );
        }
    };
    let id = request.id();
    if let Request::Stop { id } = request {
        CONNECTIONS.with_borrow(|all| {
            if let Some(stop) = all.get(&connection).and_then(|c| c.streams.get(&id)) {
                stop.store(true, Ordering::Release);
            }
        });
        return;
    }
    spawn_local(async move {
        let answer = match request {
            Request::Detect { .. } => Ok(detect().await),
            Request::Connect {
                route,
                key,
                endpoint,
                model,
                ..
            } => connect(&route, key, endpoint, model)
                .await
                .map(|state| json!({"route": state})),
            Request::Forget { route, .. } => forget(&route).await.map(|()| json!({})),
            Request::Prefer { route, model, .. } => prefer(&route, model).await.map(|()| json!({})),
            Request::Test { route, model, .. } => test(&route, model).await,
            Request::Stream {
                route,
                model,
                system,
                messages,
                ..
            } => return stream(connection, id, route, model, system, messages).await,
            Request::Stop { .. } => return,
        };
        let mut reply = match answer {
            Ok(fields) => fields,
            Err(error) => json!({"ok": false, "error": error}),
        };
        reply["id"] = json!(id);
        if reply.get("ok").is_none() {
            reply["ok"] = json!(true);
        }
        send(connection, reply);
    });
}

async fn test(route: &str, model: Option<String>) -> Result<Value, Error> {
    let target = target(Some(route), model).await?;
    let model = target.model.clone();
    let started = js_sys::Date::now();
    let ask = [Message {
        role: Role::User,
        content: "Reply with the single word OK.".into(),
    }];
    reply(
        target,
        "You answer in one word.",
        &ask,
        None,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .await?;
    Ok(json!({"ms": (js_sys::Date::now() - started).round(), "model": model}))
}

async fn stream(
    connection: u32,
    id: u64,
    route: Option<String>,
    model: Option<String>,
    system: String,
    messages: Vec<Message>,
) {
    let stop = Rc::new(AtomicBool::new(false));
    let registered = CONNECTIONS.with_borrow_mut(|all| {
        all.get_mut(&connection)
            .map(|c| c.streams.insert(id, stop.clone()))
            .is_some()
    });
    if !registered {
        return;
    }
    // Deltas that arrive together (one network read can hold hundreds) leave as one message,
    // flushed in a microtask, so a fast local model does not cost one port message per token.
    let pending = Rc::new(RefCell::new(String::new()));
    let flush = {
        let pending = pending.clone();
        move || {
            let text = std::mem::take(&mut *pending.borrow_mut());
            if !text.is_empty() {
                send(connection, json!({"id": id, "type": "delta", "text": text}));
            }
        }
    };
    let started = js_sys::Date::now();
    let mut used = String::new();
    let result = async {
        super::check_conversation(&system, &messages).map_err(str::to_owned)?;
        let target = target(route.as_deref(), model).await?;
        used.clone_from(&target.model);
        reply(target, &system, &messages, None, &stop, &mut |delta| {
            let first = pending.borrow().is_empty();
            pending.borrow_mut().push_str(delta);
            if first {
                let flush = flush.clone();
                spawn_local(async move { flush() });
            }
        })
        .await
    }
    .await;
    flush();
    CONNECTIONS.with_borrow_mut(|all| all.get_mut(&connection).map(|c| c.streams.remove(&id)));
    if let Ok((_, usage)) = &result {
        // Counted off the answer's path: the done message never waits on the store.
        let (usage, ms) = (usage.clone(), js_sys::Date::now() - started);
        spawn_local(async move { stats::record_chat(&usage, ms, &used).await });
    }
    let end = match result {
        Ok((text, usage)) => json!({"id": id, "type": "done", "text": text, "usage": usage}),
        Err(_) if stop.load(Ordering::Acquire) => {
            json!({"id": id, "type": "error", "message": "stopped"})
        }
        Err(message) => json!({"id": id, "type": "error", "message": message}),
    };
    send(connection, end);
}

// ---------- calls from the service worker's own page tools ----------

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Complete {
    system: String,
    messages: Vec<Message>,
    route: Option<String>,
    model: Option<String>,
    schema: Option<Value>,
}

fn answer(text: String, route: &str, model: &str, started: f64) -> JsValue {
    let ms = (js_sys::Date::now() - started).round();
    let model = if model.is_empty() {
        "gemini-nano"
    } else {
        model
    };
    to_js(&json!({"text": text, "route": route, "model": model, "ms": ms}))
}

fn js_error(message: impl AsRef<str>) -> JsValue {
    js_sys::Error::new(message.as_ref()).into()
}

/// One whole reply on the preferred route (or the one named): `{system, messages, route?,
/// model?, schema?}` to `{text, route, model, ms}`.
#[wasm_bindgen]
pub async fn gateway_complete(request: JsValue) -> Result<JsValue, JsValue> {
    let request: Complete = serde_json::from_value(from_js(&request))
        .map_err(|_| js_error("That model request is not valid."))?;
    super::check_conversation(&request.system, &request.messages).map_err(js_error)?;
    let target = target(request.route.as_deref(), request.model)
        .await
        .map_err(js_error)?;
    let (route, model) = (target.route.id, target.model.clone());
    let started = js_sys::Date::now();
    let stop = AtomicBool::new(false);
    let (text, _) = reply(
        target,
        &request.system,
        &request.messages,
        request.schema.as_ref(),
        &stop,
        &mut |_| {},
    )
    .await
    .map_err(js_error)?;
    Ok(answer(text, route, &model, started))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Summarize {
    content: String,
    /// key-points, tldr, teaser or headline.
    kind: Option<String>,
    /// short, medium or long.
    length: Option<String>,
    context: Option<String>,
}

/// A summary of content of any length on the preferred route: Chrome's Summarizer there when
/// the content fits, a summarizing prompt elsewhere, and sections first for long content.
/// `{content, kind?, length?, context?}` to `{text, route, model, ms}`.
#[wasm_bindgen]
pub async fn gateway_summarize(request: JsValue) -> Result<JsValue, JsValue> {
    let request: Summarize = serde_json::from_value(from_js(&request))
        .map_err(|_| js_error("That summary request is not valid."))?;
    let started = js_sys::Date::now();
    let (text, route, model) = summarized(&request).await.map_err(js_error)?;
    Ok(answer(text, route, &model, started))
}

/// A summary of `request` on the preferred route: its text, route and model.
async fn summarized(request: &Summarize) -> Result<(String, &'static str, String), Error> {
    let kind = request.kind.as_deref().unwrap_or("key-points");
    let length = request.length.as_deref().unwrap_or("medium");
    let target = target(None, None).await?;
    let (route, model) = (target.route.id, target.model.clone());
    let long = request.content.chars().count() > DIRECT_MAX;
    if target.route.protocol == Protocol::ChromeAi
        && !long
        && let Some(text) = chrome_summary(request, kind, length).await
    {
        return Ok((text, route, model));
    }
    let text = if long {
        map_reduce(&target, request, kind, length).await
    } else {
        let context = request.context.as_deref();
        summary(target, &request.content, context, kind, length).await
    }?;
    Ok((text, route, model))
}

/// Longest content summarized in one request; longer content is summarized in sections first.
const DIRECT_MAX: usize = 12_000;
const SECTION_TARGET: usize = 2_500;

async fn ask(target: Target, system: &str, input: String) -> Result<String, Error> {
    let messages = [Message {
        role: Role::User,
        content: input,
    }];
    super::check_conversation(system, &messages).map_err(str::to_owned)?;
    let stop = AtomicBool::new(false);
    Ok(reply(target, system, &messages, None, &stop, &mut |_| {})
        .await?
        .0)
}

async fn summary(
    target: Target,
    content: &str,
    context: Option<&str>,
    kind: &str,
    length: &str,
) -> Result<String, Error> {
    let shape = match kind {
        "tldr" => "Give a short, concise summary in one or two sentences",
        "teaser" => "Write an intriguing teaser that makes the reader want to learn more",
        "headline" => "Write one headline that captures the main point",
        _ => "Extract the most important points as a bullet list, each line starting with \"-\"",
    };
    let size = match length {
        "short" => "Be very brief: one to three bullets or one or two sentences",
        "long" => "Be thorough: five to seven bullets or five or six sentences",
        _ => "Use a moderate length: three to five bullets or three or four sentences",
    };
    let system = format!(
        "You are a factual summarizer. {shape}. {size}.\nRules:\n\
         1. Only use information the content states\n\
         2. Never infer or add information\n\
         3. Be specific, not vague"
    );
    let input = match context {
        Some(context) => format!("Context: {context}\n\nContent to summarize:\n{content}"),
        None => format!("Summarize this content:\n{content}"),
    };
    ask(target, &system, input).await
}

/// Paragraph groups of about `SECTION_TARGET` characters, cut inside a paragraph only when one
/// paragraph is longer than that.
/// Summarizes each section, two at a time, then merges the section summaries; a failed merge
/// falls back to the section summaries themselves.
async fn map_reduce(
    target: &Target,
    request: &Summarize,
    kind: &str,
    length: &str,
) -> Result<String, Error> {
    let page = crate::page::Sections::new(request.content.clone());
    let sections = page.spans(SECTION_TARGET);
    let total = sections.len();
    let about = request
        .context
        .as_deref()
        .map_or(String::new(), |c| format!(" Context: {c}"));
    let section = |index: usize, text: &str| {
        let input = format!(
            "This is section {index} of {total} of a longer document.{about}\n\n\
             Summarize its key points in 150 words or less, facts and main ideas only.\n\n\
             SECTION {index}:\n{text}"
        );
        let target = target.clone();
        async move {
            let summary = ask(
                target,
                "You summarize text sections concisely. Output only the summary, no preamble.",
                input,
            )
            .await;
            format!(
                "[Section {index}/{total}] {}",
                summary.as_deref().map_or("(not summarized)", str::trim)
            )
        }
    };
    let mut summaries = Vec::with_capacity(total);
    for (pair, texts) in sections.chunks(2).enumerate() {
        let first = section(pair * 2 + 1, texts[0]);
        match texts.get(1) {
            Some(second) => {
                let (a, b) = join(first, section(pair * 2 + 2, second)).await;
                summaries.extend([a, b]);
            }
            None => summaries.push(first.await),
        }
    }
    let combined = summaries.join("\n\n");
    let input = format!(
        "These are summaries of the sections of one document.{about}\n\n\
         Write one summary of the whole document.\n\nSECTION SUMMARIES:\n{combined}"
    );
    let merged = summary(target.clone(), &input, None, kind, length).await;
    Ok(merged.unwrap_or(combined))
}

/// Runs two futures together on this thread and returns both results.
async fn join<A: std::future::Future, B: std::future::Future>(
    a: A,
    b: B,
) -> (A::Output, B::Output) {
    let (mut a, mut b) = (std::pin::pin!(a), std::pin::pin!(b));
    let (mut left, mut right) = (None, None);
    std::future::poll_fn(|cx| {
        if left.is_none()
            && let std::task::Poll::Ready(out) = a.as_mut().poll(cx)
        {
            left = Some(out);
        }
        if right.is_none()
            && let std::task::Poll::Ready(out) = b.as_mut().poll(cx)
        {
            right = Some(out);
        }
        if left.is_some() && right.is_some() {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await;
    (left.expect("joined"), right.expect("joined"))
}

/// Chrome's Summarizer, when this browser offers it; `None` lets the prompt path answer.
async fn chrome_summary(request: &Summarize, kind: &str, length: &str) -> Option<String> {
    let summarizer = Some(get(&js_sys::global(), "Summarizer")).filter(JsValue::is_function)?;
    let options = to_js(
        &json!({"type": kind, "length": length, "format": "markdown", "outputLanguage": "en"}),
    );
    let create: js_sys::Function = get(&summarizer, "create").dyn_into().ok()?;
    let session = JsFuture::from(
        create
            .call1(&summarizer, &options)
            .ok()?
            .dyn_into::<js_sys::Promise>()
            .ok()?,
    )
    .await
    .ok()?;
    let summarize: js_sys::Function = get(&session, "summarize").dyn_into().ok()?;
    let context = to_js(&json!({"context": request.context}));
    let done = summarize
        .call2(&session, &request.content.as_str().into(), &context)
        .ok()?;
    let text = JsFuture::from(done.dyn_into::<js_sys::Promise>().ok()?)
        .await
        .ok()
        .and_then(|v| v.as_string());
    if let Ok(destroy) = get(&session, "destroy").dyn_into::<js_sys::Function>() {
        let _ = destroy.call0(&session);
    }
    text
}

/// The language of `text` as a two letter code. Chrome's own detector answers first since it
/// costs no model call.
/// The language of `text`: the compact py3langid model when it is loaded, else Chrome's built-in
/// detector. No model call: detection costs no tokens and waits on no network.
async fn detect_language(text: String) -> Result<String, JsValue> {
    if let Some(found) = language::identify(&text) {
        return Ok(found);
    }
    chrome_language(&text).await.ok_or_else(|| js_error("The language could not be told."))
}

async fn chrome_language(text: &str) -> Option<String> {
    let detector = Some(get(&js_sys::global(), "LanguageDetector")).filter(JsValue::is_function)?;
    let create: js_sys::Function = get(&detector, "create").dyn_into().ok()?;
    let session = JsFuture::from(
        create
            .call0(&detector)
            .ok()?
            .dyn_into::<js_sys::Promise>()
            .ok()?,
    )
    .await
    .ok()?;
    let detect: js_sys::Function = get(&session, "detect").dyn_into().ok()?;
    let found = detect
        .call1(&session, &text.into())
        .ok()?
        .dyn_into::<js_sys::Promise>()
        .ok()?;
    let found = JsFuture::from(found).await.ok();
    if let Ok(destroy) = get(&session, "destroy").dyn_into::<js_sys::Function>() {
        let _ = destroy.call0(&session);
    }
    let top = js_sys::Reflect::get(&found?, &0.into()).ok()?;
    get(&top, "detectedLanguage").as_string()
}

/// The route and model page tools use, or `null` before any is chosen.
#[wasm_bindgen]
pub async fn gateway_preferred() -> JsValue {
    to_js(&settings().await["preferred"])
}
