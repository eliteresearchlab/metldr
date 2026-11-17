//! Browse's page scripts ship with the extension as browse/<name>.js. In place of each one the
//! companion sends a marker naming the script and its SHA-256; the extension's copy goes back in
//! here once its hash matches, so both sides always run the same script.
use super::*;
use std::rc::Rc;

const PREFIX: &str = "/*metldr-bundled-script:";
const NAMES: [&str; 2] = ["injected-script", "helpers"];

thread_local! {
    /// Each script read so far, with its hex SHA-256, kept for the service worker's life.
    static SCRIPTS: RefCell<BTreeMap<&'static str, Rc<(String, String)>>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// `params` with a bundled script marker in the evaluated source replaced by the script.
pub(super) async fn expand(method: &str, mut params: Value) -> Result<Value, String> {
    let field = match method {
        "Runtime.callFunctionOn" => "functionDeclaration",
        "Runtime.evaluate" => "expression",
        _ => return Ok(params),
    };
    let Some(Value::String(source)) = params.get_mut(field) else {
        return Ok(params);
    };
    let Some(start) = source.find(PREFIX) else {
        return Ok(params);
    };
    let invalid = || "Invalid bundled script marker.".to_owned();
    let rest = &source[start + PREFIX.len()..];
    let (name, rest) = rest.split_once(':').ok_or_else(invalid)?;
    let name = NAMES.into_iter().find(|known| *known == name).ok_or_else(invalid)?;
    let hash = rest
        .get(..64)
        .filter(|hash| hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        .ok_or_else(invalid)?;
    let end = start + PREFIX.len() + name.len() + 1 + 64;
    if source.get(end..end + 2) != Some("*/") || source[end + 2..].contains(PREFIX) {
        return Err(invalid());
    }
    let script = load(name).await?;
    if script.0 != hash {
        return Err("The extension's page scripts do not match the companion's. Update both.".into());
    }
    let mut expanded = String::with_capacity(source.len() + script.1.len());
    expanded.push_str(&source[..start]);
    expanded.push_str(&script.1);
    expanded.push_str(&source[end + 2..]);
    *source = expanded;
    Ok(params)
}

async fn load(name: &'static str) -> Result<Rc<(String, String)>, String> {
    if let Some(script) = SCRIPTS.with(|cell| cell.borrow().get(name).cloned()) {
        return Ok(script);
    }
    let runtime = chrome_api("runtime").map_err(message)?;
    let path = format!("browse/{name}.js");
    let url = call(&runtime, "getURL", &[JsValue::from_str(&path)]).map_err(message)?;
    let global = js_sys::global();
    let response = await_call(&global, "fetch", &[url]).await.map_err(message)?;
    if get(&response, "ok").ok().and_then(|ok| ok.as_bool()) != Some(true) {
        return Err("The extension's page scripts are missing.".into());
    }
    let text = await_call(&response, "text", &[])
        .await
        .map_err(message)?
        .as_string()
        .ok_or("The extension's page script is not text.")?;
    let subtle = get(&get(&global, "crypto").map_err(message)?, "subtle").map_err(message)?;
    let bytes = js_sys::Uint8Array::from(text.as_bytes());
    let digest = await_call(&subtle, "digest", &[JsValue::from_str("SHA-256"), bytes.into()])
        .await
        .map_err(message)?;
    let hash: String = js_sys::Uint8Array::new(&digest)
        .to_vec()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let script = Rc::new((hash, text));
    SCRIPTS.with(|cell| cell.borrow_mut().insert(name, Rc::clone(&script)));
    Ok(script)
}
