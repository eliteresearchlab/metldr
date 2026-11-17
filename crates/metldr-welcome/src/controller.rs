use std::{cell::RefCell, rc::Rc};

use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::spawn_local;
use web_sys::{AbortController, Document, Element, Event, HtmlInputElement, HtmlSelectElement};

use crate::{
    backdrop, browser, dom, download,
    gateway::{self, Gateway},
    guide_view, setup_view,
    state::{Choice, Panel, Server, Service, Setup, Status},
};

pub type Shared = Rc<RefCell<App>>;

pub struct App {
    pub document: Document,
    pub setup: Setup,
    pub download: Option<AbortController>,
    pub alive: bool,
    gateway: Option<Rc<Gateway>>,
    theme_changed: bool,
    theme_revision: u64,
}

impl App {
    pub fn paint(&self) {
        if dom::render(&self.document, &self.setup, self.download.is_some()).is_err() {
            self.notice(
                "Setup could not update its controls. Reload to try again. Help remains available.",
            );
        }
    }

    pub fn notice(&self, message: &str) {
        let _ = dom::text(&self.document, "boot-status", message);
    }

    pub fn cancel(&mut self) {
        self.setup.generation += 1;
        if let Some(controller) = self.download.take() {
            controller.abort();
        }
    }
}

pub fn mount() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("Window unavailable"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("Document unavailable"))?;
    let preview = window.location().protocol()? != "chrome-extension:";
    guide_view::mount(&document)?;
    setup_view::mount(&document, preview)?;
    // The hero keeps its CSS gradient when WebGL2 is missing or the shader fails.
    let _ = backdrop::mount(&document);
    let app = Rc::new(RefCell::new(App {
        document: document.clone(),
        setup: Setup {
            preview,
            loading: !preview,
            panel: if preview {
                Panel::Unavailable
            } else {
                Panel::Preparing
            },
            ..Setup::default()
        },
        download: None,
        alive: true,
        gateway: if preview {
            None
        } else {
            Gateway::connect().ok()
        },
        theme_changed: false,
        theme_revision: 0,
    }));
    let clicks = app.clone();
    let click = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let button = event
            .target()
            .and_then(|v| v.dyn_into::<Element>().ok())
            .and_then(|v| v.closest("button").ok().flatten());
        if let Some(button) = button.filter(|v| !v.has_attribute("disabled")) {
            if button.id() == "open-agent-panel" {
                if event.is_trusted() {
                    open_panel(clicks.clone());
                }
            } else {
                handle_click(clicks.clone(), &button.id());
            }
        }
    });
    document.add_event_listener_with_callback("click", click.as_ref().unchecked_ref())?;
    click.forget();
    let changes = app.clone();
    let change = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        if let Some(select) = event
            .target()
            .and_then(|v| v.dyn_into::<HtmlSelectElement>().ok())
        {
            handle_change(changes.clone(), &select.id(), select.value());
        }
    });
    document.add_event_listener_with_callback("change", change.as_ref().unchecked_ref())?;
    change.forget();
    let leaving = app.clone();
    let pagehide = Closure::<dyn FnMut(Event)>::new(move |_| {
        let mut app = leaving.borrow_mut();
        app.alive = false;
        app.cancel();
        if matches!(app.setup.status, Status::Checking | Status::Downloading) {
            app.setup.status = Status::Idle;
            app.setup.progress = None;
            app.setup.detail =
                "The setup request stopped when you left this page. Check again to continue."
                    .into();
        }
    });
    window.add_event_listener_with_callback("pagehide", pagehide.as_ref().unchecked_ref())?;
    pagehide.forget();
    let returning = app.clone();
    let pageshow = Closure::<dyn FnMut(Event)>::new(move |_| {
        if !returning.borrow().alive {
            if let Some(window) = web_sys::window() {
                let _ = window.location().reload();
            }
        }
    });
    window.add_event_listener_with_callback("pageshow", pageshow.as_ref().unchecked_ref())?;
    pageshow.forget();
    app.borrow().paint();
    if preview {
        app.borrow().notice(
            "This webpage previews metldr. Open the installed Chrome extension to set it up.",
        );
    } else {
        spawn_local(prepare_panel(app.clone()));
        spawn_local(load(app));
    }
    Ok(())
}

async fn prepare_panel(app: Shared) {
    let result = browser::prepare_panel().await;
    let mut app = app.borrow_mut();
    if app.alive {
        app.setup.panel = result.map_or(Panel::Unavailable, Panel::Ready);
        app.paint();
    }
}

fn open_panel(app: Shared) {
    let mut current = app.borrow_mut();
    let Panel::Ready(tab) = current.setup.panel else {
        return;
    };
    if current.setup.preview
        || !current.alive
        || current.setup.saving
        || current.setup.choice != Choice::Agent
    {
        return;
    }
    current.cancel();
    // Start Chrome's call before yielding so it retains the trusted click.
    let result = browser::start_panel_open(tab);
    current.setup.panel = Panel::Opening;
    current.paint();
    drop(current);
    spawn_local(async move {
        let result = match result {
            Ok(promise) => wasm_bindgen_futures::JsFuture::from(promise)
                .await
                .map(|_| ()),
            Err(error) => Err(error),
        };
        let mut app = app.borrow_mut();
        if !app.alive {
            return;
        }
        app.setup.panel = Panel::Ready(tab);
        let notice = if result.is_ok() {
            "Panel opened. Choose Settings to continue."
        } else {
            "The panel could not open. Try again or press Alt+Shift+M in Chrome."
        };
        app.notice(notice);
        app.paint();
    });
}

async fn load(shared: Shared) {
    let destination = browser::saved_destination();
    // Chrome's check starts before the gateway's, so the two run at the same time.
    let chrome = browser::summarizer()
        .and_then(|api| browser::call(&api, "availability", &[]).ok())
        .and_then(|value| browser::within(&value, 3_000.0).ok())
        .map(wasm_bindgen_futures::JsFuture::from);
    let gateway = shared.borrow().gateway.clone();
    let detected = match &gateway {
        Some(gateway) => gateway.detect().await,
        None => Err("The gateway is not reachable.".into()),
    };
    let theme = browser::load_theme().await;
    let availability = match chrome {
        Some(future) => future.await.ok().and_then(|value| value.as_string()),
        None => Some("missing-api".into()),
    };
    let mut app = shared.borrow_mut();
    if !app.alive {
        return;
    }
    let preferred = detected
        .as_ref()
        .ok()
        .and_then(|found| browser::get(found, "preferred").ok())
        .filter(|preferred| preferred.is_object());
    let preferred_route = preferred
        .as_ref()
        .and_then(|preferred| browser::get(preferred, "route").ok())
        .and_then(|route| route.as_string());
    let preferred_model = preferred
        .as_ref()
        .and_then(|preferred| browser::get(preferred, "model").ok())
        .and_then(|model| model.as_string());
    app.setup.servers = detected.as_ref().map(servers).unwrap_or_default();
    let pick = Setup::recommend(availability.as_deref(), &app.setup.servers);
    app.setup.recommended = Some(pick);
    if app.setup.generation == 0 {
        match &destination {
            Ok(None) if preferred_route.is_none() => app.setup.choose(pick),
            Ok(destination) => app
                .setup
                .restore_choice(destination.as_deref(), preferred_route.as_deref()),
            Err(_) => {
                app.setup.detail =
                    "Your saved destination could not be read. Review it in the panel before connecting."
                        .into();
            }
        }
        if let Some(model) = preferred_model.filter(|model| app.setup.models.contains(model)) {
            app.setup.model = model;
        }
        match app.setup.choice {
            Choice::Chrome => app
                .setup
                .availability(availability.as_deref().unwrap_or("unknown")),
            Choice::Provider if destination.as_ref().is_ok_and(Option::is_none) => {
                if pick == Choice::Provider {
                    app.setup.detail = "Chrome's built-in AI and local model servers were not found here. Paste a key from a service you use.".into();
                }
            }
            _ => {}
        }
    }
    if !app.theme_changed {
        if let Ok(Some(theme)) = theme {
            app.setup.theme = theme;
        }
    }
    app.notice(if detected.is_ok() {
        "Choose a connection to get started."
    } else {
        "metldr's model gateway did not answer. Reload this page to try again."
    });
    app.setup.loading = false;
    let _ = dom::models(&app.document, &app.setup.models, &app.setup.model);
    app.paint();
}

/// The local servers in a gateway `detect` reply that answered, with their models.
fn servers(detected: &JsValue) -> Vec<(Server, Vec<String>)> {
    let routes = browser::get(detected, "routes")
        .ok()
        .and_then(|routes| routes.dyn_into::<js_sys::Array>().ok());
    let Some(routes) = routes else {
        return Vec::new();
    };
    routes
        .iter()
        .filter(|state| {
            browser::get(state, "state")
                .ok()
                .and_then(|v| v.as_string())
                .as_deref()
                == Some("ready")
        })
        .filter_map(|state| {
            let route = browser::get(&state, "route").ok()?.as_string()?;
            Some((Server::from_route(&route)?, gateway::models(&state)))
        })
        .collect()
}

fn handle_click(app: Shared, id: &str) {
    let found = {
        let app = app.borrow();
        match app.setup.recommended {
            Some(Choice::Local(server)) => Some(server),
            _ => app.setup.servers.first().map(|(server, _)| *server),
        }
    };
    let choice = match id {
        "choose-local" => Some(Choice::Chrome),
        "choose-agent" => Some(found.map_or(Choice::Agent, Choice::Local)),
        "choose-ollama" => Some(Choice::Local(Server::Ollama)),
        "choose-lmstudio" => Some(Choice::Local(Server::LmStudio)),
        "choose-llamacpp" => Some(Choice::Local(Server::LlamaCpp)),
        "choose-installed" => Some(Choice::Agent),
        "choose-provider" => Some(Choice::Provider),
        _ => None,
    };
    if let Some(choice) = choice {
        let mut app = app.borrow_mut();
        if app.setup.saving {
            return;
        }
        app.cancel();
        app.setup.choose(choice);
        let _ = dom::models(&app.document, &app.setup.models, &app.setup.model);
        app.paint();
        return;
    }
    if app.borrow().setup.preview {
        return;
    }
    match id {
        "check-button" => check(app),
        "key-connect" => connect_key(app),
        "download-button" => {
            if download::start(app.clone()).is_err() {
                let mut app = app.borrow_mut();
                app.cancel();
                app.setup.status = Status::Error;
                app.setup.detail =
                    "The model download could not start. Check again to retry.".into();
                app.paint();
            }
        }
        "cancel-button" => {
            let mut app = app.borrow_mut();
            app.cancel();
            app.setup.status = Status::Idle;
            app.setup.progress = None;
            app.setup.detail = "The setup request was cancelled. Chrome may continue a model download shared with other pages.".into();
            app.paint();
        }
        "continue-button" => finish(app),
        "explore-button" => {
            if let Some(window) = web_sys::window() {
                let _ = window.close();
            }
        }
        _ => {}
    }
}

fn handle_change(app: Shared, id: &str, value: String) {
    let mut current = app.borrow_mut();
    if current.setup.saving {
        return;
    }
    match id {
        "model-choice" if current.setup.models.contains(&value) => current.setup.model = value,
        "key-service" => match Service::from_route(&value) {
            Some(service) => {
                current.cancel();
                current.setup.service = service;
                current.setup.choose(Choice::Provider);
            }
            None => return,
        },
        "theme-choice" if browser::valid_theme(&value) => {
            current.theme_changed = true;
            current.theme_revision += 1;
            current.setup.theme = value;
            if !current.setup.preview && !current.setup.theme_saving {
                current.setup.theme_saving = true;
                spawn_local(save_theme(app.clone()));
            }
        }
        _ => return,
    }
    current.paint();
}

async fn save_theme(app: Shared) {
    loop {
        let (theme, revision) = {
            let app = app.borrow();
            if !app.alive {
                return;
            }
            (app.setup.theme.clone(), app.theme_revision)
        };
        let result = browser::save_theme(&theme).await;
        let mut app = app.borrow_mut();
        if !app.alive {
            return;
        }
        if result.is_err() {
            app.notice(
                "The theme changed for this page but could not be saved. Try choosing it again.",
            );
        }
        if revision == app.theme_revision {
            app.setup.theme_saving = false;
            app.paint();
            return;
        }
    }
}

fn check(app: Shared) {
    let (generation, choice, gateway) = {
        let mut app = app.borrow_mut();
        if app.setup.saving || app.download.is_some() || app.setup.status == Status::Checking {
            return;
        }
        let choice = app.setup.choice;
        if !matches!(choice, Choice::Chrome | Choice::Local(_)) {
            return;
        }
        let generation = app.setup.begin(Status::Checking);
        app.setup.detail = "Checking availability. No prompt or page content is sent.".into();
        app.paint();
        (generation, choice, app.gateway.clone())
    };
    spawn_local(async move {
        // Chrome reports its availability; a local server is found by the gateway's detect.
        let result = match (choice, gateway) {
            (Choice::Chrome, _) => match browser::summarizer() {
                Some(api) => browser::await_call(&api, "availability", &[])
                    .await
                    .map(|value| (value.as_string(), Vec::new()))
                    .map_err(|_| {
                        "Chrome AI could not be checked. Try again or choose another connection."
                            .to_owned()
                    }),
                None => Ok((Some("missing-api".into()), Vec::new())),
            },
            (_, Some(gateway)) => gateway.detect().await.map(|found| (None, servers(&found))),
            (_, None) => {
                Err("metldr's model gateway did not answer. Reload this page to try again.".into())
            }
        };
        let mut app = app.borrow_mut();
        if !app.alive || !app.setup.current(generation) {
            return;
        }
        match (choice, result) {
            (Choice::Chrome, Ok((availability, _))) => app
                .setup
                .availability(availability.as_deref().unwrap_or("unknown")),
            (Choice::Local(server), Ok((_, found))) => {
                app.setup.servers = found;
                app.setup.choose(choice);
                if app.setup.found(server).is_none() {
                    app.setup.status = Status::Unavailable;
                    app.setup.detail = format!(
                        "Nothing answered at {}. Start {}, then check again.",
                        server.address(),
                        server.name()
                    );
                }
                if dom::models(&app.document, &app.setup.models, &app.setup.model).is_err() {
                    app.setup.status = Status::Error;
                    app.setup.detail =
                        "Installed models could not be shown. Reload to try again.".into();
                }
            }
            (_, Err(error)) => {
                app.setup.status = Status::Error;
                app.setup.detail = error;
            }
            _ => {}
        }
        app.paint();
    });
}

/// Checks the pasted key with its service through the gateway, which stores it encrypted.
fn connect_key(app: Shared) {
    let (generation, route, key, gateway) = {
        let mut app = app.borrow_mut();
        if app.setup.saving || app.setup.status == Status::Checking {
            return;
        }
        let Ok(input) = dom::element(&app.document, "key-input")
            .and_then(|input| input.dyn_into::<HtmlInputElement>().map_err(Into::into))
        else {
            return;
        };
        let key = input.value().trim().to_owned();
        if key.is_empty() {
            app.setup.status = Status::Error;
            app.setup.detail = "Paste your API key.".into();
            app.paint();
            return;
        }
        let Some(gateway) = app.gateway.clone() else {
            app.setup.status = Status::Error;
            app.setup.detail =
                "metldr's model gateway did not answer. Reload this page to try again.".into();
            app.paint();
            return;
        };
        let generation = app.setup.begin(Status::Checking);
        app.setup.detail =
            "Checking the key with the service. No prompt or page content is sent.".into();
        app.paint();
        (generation, app.setup.service.route(), key, gateway)
    };
    spawn_local(async move {
        let result = gateway.connect_key(route, key).await;
        let mut app = app.borrow_mut();
        if !app.alive || !app.setup.current(generation) {
            return;
        }
        match result {
            Ok(state) => {
                if let Ok(input) = dom::element(&app.document, "key-input")
                    .and_then(|input| input.dyn_into::<HtmlInputElement>().map_err(Into::into))
                {
                    input.set_value("");
                }
                app.setup.models = gateway::models(&state);
                app.setup.model = browser::get(&state, "model")
                    .ok()
                    .and_then(|model| model.as_string())
                    .filter(|model| app.setup.models.contains(model))
                    .or_else(|| app.setup.models.first().cloned())
                    .unwrap_or_default();
                app.setup.status = if app.setup.models.is_empty() {
                    Status::Unavailable
                } else {
                    Status::Ready
                };
                app.setup.detail = if app.setup.models.is_empty() {
                    "The key works, but the service listed no models for it."
                } else {
                    "Key accepted and kept encrypted in this browser. Choose a model, then save."
                }
                .into();
                let _ = dom::models(&app.document, &app.setup.models, &app.setup.model);
            }
            Err(error) => {
                app.setup.status = Status::Error;
                app.setup.detail = error;
            }
        }
        app.paint();
    });
}

/// Makes the checked choice the route metldr uses, then closes the tab.
fn finish(app: Shared) {
    let (route, model, gateway) = {
        let mut app = app.borrow_mut();
        if app.setup.loading || app.setup.saving || app.setup.theme_saving || !app.setup.ready() {
            return;
        }
        let route = match app.setup.choice {
            Choice::Chrome => "chrome-ai",
            Choice::Local(server) => server.route(),
            Choice::Provider => app.setup.service.route(),
            Choice::Agent => return,
        };
        let model = if app.setup.choice == Choice::Chrome {
            String::new()
        } else {
            app.setup.model.clone()
        };
        let Some(gateway) = app.gateway.clone() else {
            return;
        };
        app.cancel();
        app.setup.saving = true;
        app.paint();
        (route, model, gateway)
    };
    spawn_local(async move {
        let result = gateway.prefer(route, &model).await;
        let mut app = app.borrow_mut();
        if !app.alive {
            return;
        }
        app.setup.saving = false;
        match result {
            Ok(()) => {
                app.notice("Setup saved. You can close this tab and open metldr from the toolbar.");
                if let Some(window) = web_sys::window() {
                    let _ = window.close();
                }
            }
            Err(error) => app.notice(&format!(
                "Setup could not be saved: {error} This tab stays open. Try saving again."
            )),
        }
        app.paint();
    });
}
