use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Document, Element, HtmlSelectElement};

use crate::state::{Choice, Panel, Server, Setup, Status};

pub fn element(document: &Document, id: &str) -> Result<Element, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("Missing control {id}")))
}

pub fn text(document: &Document, id: &str, value: &str) -> Result<(), JsValue> {
    element(document, id)?.set_text_content(Some(value));
    Ok(())
}

fn flag(document: &Document, id: &str, attribute: &str, enabled: bool) -> Result<(), JsValue> {
    let element = element(document, id)?;
    if enabled {
        element.set_attribute(attribute, "")
    } else {
        element.remove_attribute(attribute)
    }
}

pub fn models(document: &Document, names: &[String], selected: &str) -> Result<(), JsValue> {
    let select = element(document, "model-choice")?;
    select.set_text_content(None);
    for name in names {
        let option = document.create_element("option")?;
        option.set_attribute("value", name)?;
        option.set_text_content(Some(name));
        select.append_child(&option)?;
    }
    select.dyn_into::<HtmlSelectElement>()?.set_value(selected);
    Ok(())
}

pub fn render(document: &Document, setup: &Setup, downloading: bool) -> Result<(), JsValue> {
    let local = matches!(setup.choice, Choice::Chrome | Choice::Local(_));
    let have = matches!(setup.choice, Choice::Local(_) | Choice::Agent);
    let checking = setup.status == Status::Checking;
    for (id, selected) in [
        ("choose-local", setup.choice == Choice::Chrome),
        ("choose-agent", have),
        ("choose-provider", setup.choice == Choice::Provider),
        (
            "choose-ollama",
            setup.choice == Choice::Local(Server::Ollama),
        ),
        (
            "choose-lmstudio",
            setup.choice == Choice::Local(Server::LmStudio),
        ),
        (
            "choose-llamacpp",
            setup.choice == Choice::Local(Server::LlamaCpp),
        ),
        ("choose-installed", setup.choice == Choice::Agent),
    ] {
        element(document, id)?
            .set_attribute("aria-pressed", if selected { "true" } else { "false" })?;
        flag(document, id, "disabled", setup.saving)?;
    }
    for (id, visible) in [
        ("pick-local", setup.recommended == Some(Choice::Chrome)),
        (
            "pick-agent",
            matches!(setup.recommended, Some(Choice::Local(_))),
        ),
        ("pick-provider", setup.recommended == Some(Choice::Provider)),
        ("local-options", have),
        ("chrome-controls", setup.choice == Choice::Chrome),
        (
            "ollama-guide",
            setup.choice == Choice::Local(Server::Ollama),
        ),
        ("key-controls", setup.choice == Choice::Provider),
        (
            "model-controls",
            !setup.models.is_empty() && matches!(setup.choice, Choice::Local(_) | Choice::Provider),
        ),
        ("check-button", local),
        ("panel-setup", setup.choice == Choice::Agent),
        ("continue-button", setup.choice != Choice::Agent),
        ("download-button", setup.status == Status::Downloadable),
        ("cancel-button", downloading),
        ("download-progress", setup.status == Status::Downloading),
        ("preview-panel", setup.preview),
    ] {
        flag(document, id, "hidden", !visible)?;
    }
    for (id, disabled) in [
        (
            "check-button",
            setup.preview || setup.saving || checking || downloading,
        ),
        (
            "download-button",
            setup.preview || setup.saving || downloading || setup.status != Status::Downloadable,
        ),
        ("cancel-button", setup.preview || !downloading),
        ("key-service", setup.preview || setup.saving || checking),
        ("key-input", setup.preview || setup.saving || checking),
        ("key-connect", setup.preview || setup.saving || checking),
        (
            "open-agent-panel",
            setup.preview || setup.saving || !matches!(setup.panel, Panel::Ready(_)),
        ),
        (
            "continue-button",
            setup.preview || setup.loading || setup.saving || setup.theme_saving || !setup.ready(),
        ),
        (
            "explore-button",
            setup.preview || setup.loading || setup.saving || setup.theme_saving,
        ),
        ("theme-choice", setup.saving),
        ("model-choice", setup.saving || setup.models.is_empty()),
    ] {
        flag(document, id, "disabled", disabled)?;
    }
    let heading = if setup.saving {
        "Saving setup".to_owned()
    } else {
        match setup.status {
            Status::Idle => match setup.choice {
                Choice::Chrome => "Chrome's built-in AI".to_owned(),
                Choice::Local(server) => format!("{} on this computer", server.name()),
                Choice::Provider => "Bring a key".to_owned(),
                Choice::Agent => "Set up an installed agent".to_owned(),
            },
            Status::Checking => "Checking".to_owned(),
            Status::Ready => "Connection ready".to_owned(),
            Status::Downloadable => "Model download available".to_owned(),
            Status::Downloading => "Model download in progress".to_owned(),
            Status::Unavailable => "Connection unavailable".to_owned(),
            Status::Error => "Connection check failed".to_owned(),
        }
    };
    element(document, "key-service")?
        .dyn_into::<HtmlSelectElement>()?
        .set_value(setup.service.route());
    text(document, "status-text", &heading)?;
    text(document, "connection-detail", &setup.detail)?;
    text(
        document,
        "panel-setup-status",
        match setup.panel {
            Panel::Preparing => "Preparing the panel.",
            Panel::Ready(_) => "The panel opens beside this tab.",
            Panel::Opening => "Opening the panel.",
            Panel::Unavailable => {
                "Open this page in the installed extension. Reload the extension if the panel stays unavailable."
            }
        },
    )?;
    let status = match setup.status {
        Status::Ready => "ready",
        Status::Error | Status::Unavailable => "error",
        _ => "idle",
    };
    element(document, "status-indicator")?.set_attribute("data-status", status)?;
    let progress = element(document, "download-progress")?;
    match setup.progress {
        Some(value) => progress.set_attribute("value", &(value * 100.0).to_string())?,
        None => progress.remove_attribute("value")?,
    }
    element(document, "theme-choice")?
        .dyn_into::<HtmlSelectElement>()?
        .set_value(&setup.theme);
    Ok(())
}
