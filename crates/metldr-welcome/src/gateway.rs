//! The welcome page's side of the extension's model gateway (the `metldr-gateway` port): it
//! detects routes, checks and stores a key, and saves the route metldr uses. Keys go to the
//! service worker once and are never kept here.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use js_sys::{Function, Promise};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;

use crate::browser::{call, get, object, within};

/// Longest wait for one reply; a key check lists the service's models within ten seconds.
const REPLY_MS: f64 = 15_000.0;

pub struct Gateway {
    port: JsValue,
    next: Cell<u64>,
    pending: Rc<RefCell<HashMap<u64, Function>>>,
    _listeners: [Closure<dyn FnMut(JsValue)>; 2],
}

impl Gateway {
    pub fn connect() -> Result<Rc<Self>, JsValue> {
        let runtime = get(&get(&js_sys::global(), "chrome")?, "runtime")?;
        let port = call(
            &runtime,
            "connect",
            &[object(&[("name", "metldr-gateway".into())])?],
        )?;
        let pending: Rc<RefCell<HashMap<u64, Function>>> = Rc::default();
        let replies = pending.clone();
        let on_message = Closure::<dyn FnMut(JsValue)>::new(move |message: JsValue| {
            let id = get(&message, "id").ok().and_then(|id| id.as_f64());
            if let Some(resolve) = id.and_then(|id| replies.borrow_mut().remove(&(id as u64))) {
                let _ = resolve.call1(&JsValue::NULL, &message);
            }
        });
        let closed = pending.clone();
        let on_disconnect = Closure::<dyn FnMut(JsValue)>::new(move |_| {
            let gone = object(&[
                ("ok", false.into()),
                ("error", "The gateway closed.".into()),
            ])
            .unwrap_or(JsValue::NULL);
            for (_, resolve) in closed.borrow_mut().drain() {
                let _ = resolve.call1(&JsValue::NULL, &gone);
            }
        });
        for (event, listener) in [("onMessage", &on_message), ("onDisconnect", &on_disconnect)] {
            call(
                &get(&port, event)?,
                "addListener",
                &[listener.as_ref().clone()],
            )?;
        }
        Ok(Rc::new(Self {
            port,
            next: Cell::new(1),
            pending,
            _listeners: [on_message, on_disconnect],
        }))
    }

    /// Sends `op` with `fields` and waits for its reply; an error reply becomes its message.
    async fn request(&self, op: &str, fields: &[(&str, JsValue)]) -> Result<JsValue, String> {
        let id = self.next.get();
        self.next.set(id + 1);
        let mut message = vec![("op", JsValue::from_str(op)), ("id", (id as f64).into())];
        message.extend(fields.iter().cloned());
        let message = object(&message).map_err(|_| "The request could not be built.")?;
        let pending = self.pending.clone();
        let reply = Promise::new(&mut |resolve, _| {
            pending.borrow_mut().insert(id, resolve);
        });
        if call(&self.port, "postMessage", &[message]).is_err() {
            self.pending.borrow_mut().remove(&id);
            return Err("The gateway is not reachable. Reload this page.".into());
        }
        let reply = within(&reply, REPLY_MS)
            .map(JsFuture::from)
            .map_err(|_| "The gateway did not answer.")?
            .await
            .unwrap_or(JsValue::UNDEFINED);
        self.pending.borrow_mut().remove(&id);
        if reply.is_undefined() {
            return Err("The gateway did not answer. Check again.".into());
        }
        if get(&reply, "ok").ok().and_then(|ok| ok.as_bool()) == Some(true) {
            Ok(reply)
        } else {
            Err(get(&reply, "error")
                .ok()
                .and_then(|error| error.as_string())
                .unwrap_or_else(|| "The gateway refused the request.".into()))
        }
    }

    /// Every route's state, and the route metldr uses now (`{routes, preferred}`).
    pub async fn detect(&self) -> Result<JsValue, String> {
        self.request("detect", &[]).await
    }

    /// Checks `key` with the service of `route` and stores it encrypted; the route's state.
    pub async fn connect_key(&self, route: &str, key: String) -> Result<JsValue, String> {
        let reply = self
            .request("connect", &[("route", route.into()), ("key", key.into())])
            .await?;
        get(&reply, "route").map_err(|_| "The gateway sent no route.".into())
    }

    /// Makes `route` with `model` the one metldr uses.
    pub async fn prefer(&self, route: &str, model: &str) -> Result<(), String> {
        self.request(
            "prefer",
            &[("route", route.into()), ("model", model.into())],
        )
        .await
        .map(|_| ())
    }
}

/// The names in a route state's `models`, bounded like the gateway's own list.
pub fn models(state: &JsValue) -> Vec<String> {
    get(state, "models")
        .ok()
        .and_then(|models| models.dyn_into::<js_sys::Array>().ok())
        .map(|models| {
            models
                .iter()
                .take(200)
                .filter_map(|name| name.as_string())
                .filter(|name| !name.is_empty() && name.len() <= 128)
                .collect()
        })
        .unwrap_or_default()
}
