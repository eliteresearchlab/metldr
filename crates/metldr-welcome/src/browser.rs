use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

pub fn runtime_id() -> Result<String, JsValue> {
    let chrome = get(&js_sys::global(), "chrome")?;
    get(&get(&chrome, "runtime")?, "id")?
        .as_string()
        .ok_or_else(|| JsValue::from_str("Extension ID unavailable"))
}

pub fn start_clipboard_write(text: &str) -> Result<Promise, JsValue> {
    let navigator = get(&js_sys::global(), "navigator")?;
    let clipboard = get(&navigator, "clipboard")?;
    call(&clipboard, "writeText", &[JsValue::from_str(text)])?.dyn_into::<Promise>()
}

pub async fn prepare_panel() -> Result<i32, JsValue> {
    let origin = crate::setup_guide::ExtensionOrigin::from_id(&runtime_id()?)
        .ok_or_else(|| JsValue::from_str("Extension ID unavailable"))?;
    let chrome = get(&js_sys::global(), "chrome")?;
    let tab = await_call(&get(&chrome, "tabs")?, "getCurrent", &[]).await?;
    let id = get(&tab, "id")?
        .as_f64()
        .filter(|id| id.is_finite() && id.fract() == 0.0 && (0.0..=i32::MAX as f64).contains(id))
        .ok_or_else(|| JsValue::from_str("Welcome tab unavailable"))? as i32;
    let url = get(&tab, "url")?.as_string().unwrap_or_default();
    if url.split('#').next() != Some(format!("{}/welcome.html", origin.as_str()).as_str()) {
        return Err(JsValue::from_str("Welcome tab unavailable"));
    }
    let options = object(&[
        ("tabId", id.into()),
        ("path", "chat.html".into()),
        ("enabled", true.into()),
    ])?;
    await_call(&get(&chrome, "sidePanel")?, "setOptions", &[options]).await?;
    Ok(id)
}

pub fn start_panel_open(tab: i32) -> Result<Promise, JsValue> {
    let chrome = get(&js_sys::global(), "chrome")?;
    call(
        &get(&chrome, "sidePanel")?,
        "open",
        &[object(&[("tabId", tab.into())])?],
    )?
    .dyn_into::<Promise>()
}

pub fn get(object: &JsValue, key: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &JsValue::from_str(key))
}

pub fn call(object: &JsValue, method: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let function = get(object, method)?.dyn_into::<Function>()?;
    let args = Array::from_iter(args.iter());
    function.apply(object, &args)
}

pub async fn await_call(
    object: &JsValue,
    method: &str,
    args: &[JsValue],
) -> Result<JsValue, JsValue> {
    JsFuture::from(Promise::resolve(&call(object, method, args)?)).await
}

/// The panel theme saved in `chrome.storage.local`, when it is one of the four.
pub async fn load_theme() -> Result<Option<String>, JsValue> {
    let storage = get(&get(&js_sys::global(), "chrome")?, "storage")?;
    let saved = await_call(&get(&storage, "local")?, "get", &["theme".into()]).await?;
    Ok(get(&saved, "theme")?
        .as_string()
        .filter(|theme| valid_theme(theme)))
}

pub fn valid_theme(value: &str) -> bool {
    matches!(value, "catppuccin" | "gruvbox" | "cyberpunk" | "light")
}

pub fn saved_destination() -> Result<Option<String>, JsValue> {
    let storage = get(&js_sys::global(), "localStorage")?;
    let value = call(&storage, "getItem", &["metldr.composer.destination".into()])?;
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_string()
        .filter(|value| value.len() <= 1024)
        .map(Some)
        .ok_or_else(|| JsValue::from_str("Saved destination is invalid"))
}

pub async fn save_theme(theme: &str) -> Result<(), JsValue> {
    if !valid_theme(theme) {
        return Err(JsValue::from_str("Invalid theme"));
    }
    let storage = get(&get(&js_sys::global(), "chrome")?, "storage")?;
    let settings = object(&[("theme", theme.into())])?;
    await_call(&get(&storage, "local")?, "set", &[settings]).await?;
    Ok(())
}

pub fn object(fields: &[(&str, JsValue)]) -> Result<JsValue, JsValue> {
    let result = Object::new();
    for (key, value) in fields {
        Reflect::set(&result, &JsValue::from_str(key), value)?;
    }
    Ok(result.into())
}

/// `promise`, or `undefined` once `ms` pass without an answer.
pub fn within(promise: &JsValue, ms: f64) -> Result<Promise, JsValue> {
    let set_timeout = get(&js_sys::global(), "setTimeout")?.dyn_into::<Function>()?;
    let timeout = Promise::new(&mut |resolve, _| {
        let _ = set_timeout.call2(&js_sys::global(), &resolve, &JsValue::from_f64(ms));
    });
    Ok(Promise::race(&Array::of2(
        &Promise::resolve(promise),
        &timeout,
    )))
}

pub fn summarizer() -> Option<JsValue> {
    get(&js_sys::global(), "Summarizer")
        .ok()
        .filter(|value| !value.is_null() && !value.is_undefined())
}
