use super::*;
use wasm_bindgen::{JsCast, JsValue, prelude::*};
use winit::event::{ElementState, MouseButton, TouchPhase};
use winit::keyboard::ModifiersState;
use std::cell::Cell;
use js_sys::{Function, Object, Reflect};

#[wasm_bindgen(module = "/src/platform/web/bridge.js")]
extern "C" {
    fn mount(
        canvas: &web_sys::HtmlCanvasElement,
        callback: &js_sys::Function,
        raw: &js_sys::Function,
        embedded: bool,
        fit_height: bool,
    ) -> JsValue;
    fn native_button(handle: &JsValue, path: &str) -> JsValue;
    fn set_native_button(handle: &JsValue, node: &web_sys::HtmlElement, enabled: bool);
    fn focus_native_button(handle: &JsValue, node: &web_sys::HtmlElement, requested: bool);
    fn sync(
        handle: &JsValue,
        key: &str,
        text: &str,
        base: usize,
        anchor: u32,
        focus: u32,
        secure: bool,
    );
    #[wasm_bindgen(js_name = syncCopy)]
    fn sync_copy(handle: &JsValue, text: &str);
    #[wasm_bindgen(js_name = syncA11y)]
    fn sync_a11y(handle: &JsValue, json: &str, revision: f64);
    fn activate(handle: &JsValue, request_focus: bool);
    fn destroy(handle: &JsValue);
    #[wasm_bindgen(js_name = pastedImage)]
    fn pasted_image(handle: &JsValue, max: u32) -> js_sys::Promise;
}

#[derive(Clone)]
struct NativeGetters {
    trusted: Function, kind: Function, phase: Function, current: Function, target: Function,
    pointer_id: Function, primary: Function, button: Function, x: Function, y: Function,
    key: Function, shift: Function, ctrl: Function, alt: Function, meta: Function,
    owner_document: Function, connected: Function,
}

fn getter(object: &Object, name: &str) -> Option<Function> {
    let descriptor = Object::get_own_property_descriptor(object, &name.into());
    Reflect::get(&descriptor, &"get".into()).ok()?.dyn_into().ok()
}

fn inherited_getter(value: &JsValue, name: &str) -> Option<Function> {
    let mut prototype = Object::get_prototype_of(value);
    loop {
        if let Some(found) = getter(&prototype, name) { return Some(found); }
        let next = Object::get_prototype_of(prototype.as_ref());
        if next.is_null() { return None; }
        prototype = next;
    }
}

impl NativeGetters {
    fn new() -> Option<Self> {
        let event = web_sys::Event::new("click").ok()?;
        let value: &JsValue = event.as_ref();
        let event_proto = Object::get_prototype_of(value);
        let pointer = web_sys::PointerEvent::new("pointerdown").ok()?;
        let pointer_proto = Object::get_prototype_of(pointer.as_ref());
        let mouse_proto = Object::get_prototype_of(pointer_proto.as_ref());
        let keyboard = web_sys::KeyboardEvent::new("keydown").ok()?;
        let keyboard_proto = Object::get_prototype_of(keyboard.as_ref());
        let node = web_sys::window()?.document()?.create_element("button").ok()?;
        Some(Self {
            trusted: getter(value.unchecked_ref(), "isTrusted")?,
            kind: getter(&event_proto, "type")?, phase: getter(&event_proto, "eventPhase")?,
            current: getter(&event_proto, "currentTarget")?, target: getter(&event_proto, "target")?,
            pointer_id: getter(&pointer_proto, "pointerId")?, primary: getter(&pointer_proto, "isPrimary")?,
            button: getter(&mouse_proto, "button")?, x: getter(&mouse_proto, "clientX")?,
            y: getter(&mouse_proto, "clientY")?, key: getter(&keyboard_proto, "key")?,
            shift: getter(&keyboard_proto, "shiftKey")?,
            ctrl: getter(&keyboard_proto, "ctrlKey")?, alt: getter(&keyboard_proto, "altKey")?,
            meta: getter(&keyboard_proto, "metaKey")?,
            owner_document: inherited_getter(node.as_ref(), "ownerDocument")?,
            connected: inherited_getter(node.as_ref(), "isConnected")?,
        })
    }

    fn value(&self, getter: &Function, event: &web_sys::Event) -> Option<JsValue> {
        getter.call0(event.as_ref()).ok()
    }

    fn valid(&self, event: &web_sys::Event, kind: &str, current: &JsValue, target: &JsValue) -> bool {
        self.value(&self.trusted, event).and_then(|v| v.as_bool()) == Some(true)
            && self.value(&self.kind, event).and_then(|v| v.as_string()).as_deref() == Some(kind)
            && self.value(&self.phase, event).and_then(|v| v.as_f64()).is_some_and(|p| (1.0..=3.0).contains(&p))
            && self.value(&self.current, event).is_some_and(|v| v == *current)
            && self.value(&self.target, event).is_some_and(|v| v == *target)
    }

    fn in_document(&self, node: &JsValue) -> bool {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return false };
        let document_value: &JsValue = document.as_ref();
        self.connected.call0(node).ok().and_then(|v| v.as_bool()) == Some(true)
            && self.owner_document.call0(node).is_ok_and(|v| v == *document_value)
    }

    fn connected(&self, node: &web_sys::HtmlElement) -> bool {
        self.in_document(node.as_ref()) && node.tag_name() == "BUTTON"
    }
}

struct NativeBinding { path: String, key: String, node: web_sys::HtmlElement, live: Cell<bool> }
struct NativeRegistration {
    binding: Rc<NativeBinding>,
    click: Closure<dyn FnMut(web_sys::Event)>,
    focus: Closure<dyn FnMut(web_sys::Event)>,
    tab: Closure<dyn FnMut(web_sys::Event)>,
}

impl NativeRegistration {
    fn retire(self, handle: &JsValue) {
        self.binding.live.set(false);
        let node = &self.binding.node;
        let _ = node.remove_event_listener_with_callback("click", self.click.as_ref().unchecked_ref());
        let _ = node.remove_event_listener_with_callback("focus", self.focus.as_ref().unchecked_ref());
        let _ = node.remove_event_listener_with_callback("keydown", self.tab.as_ref().unchecked_ref());
        set_native_button(handle, node, false);
    }
}

pub(super) struct Bridge<S: 'static> {
    handle: JsValue,
    _callback: Closure<dyn FnMut(String)>,
    _raw: Closure<dyn FnMut(web_sys::Event)>,
    native: Vec<NativeRegistration>,
    candidate: Option<(i32, String, String)>,
    getters: Option<NativeGetters>,
    running: std::rc::Weak<RefCell<Option<Running<S>>>>,
    dispose_viewport: js_sys::Function,
    mods: ModifiersState,
    mirror: crate::runtime::web_runtime::Mirror,
    a11y: crate::runtime::web_a11y::A11yMirror,
    // Counts each mirror pushed to the page. A press names the revision it was rendered from.
    a11y_revision: u64,
    copy_cache: String,
    fit_height: bool,
}

impl<S: 'static> Bridge<S> {
    pub(super) fn new(
        canvas: &web_sys::HtmlCanvasElement,
        running: std::rc::Weak<RefCell<Option<Running<S>>>>,
        window: Arc<Window>,
        embedded: bool,
        fit_height: bool,
    ) -> Self {
        let raw_running = running.clone();
        let raw_window = window.clone();
        let raw_canvas = canvas.clone();
        let raw = Closure::new(move |event: web_sys::Event| {
            let Some(running) = raw_running.upgrade() else { return };
            let Ok(mut running) = running.try_borrow_mut() else { return };
            if let Some(r) = &mut *running {
                r.native_raw(&event, &raw_canvas, &raw_window);
            }
        });
        let callback_running = running.clone();
        let callback_window = window.clone();
        let callback = Closure::new(move |event: String| {
            let Some(running) = callback_running.upgrade() else {
                return;
            };
            let event = zeroize::Zeroizing::new(event);
            if let Ok(mut event) = serde_json::from_str(&event) {
                // A DOM event fired synchronously by page JavaScript Rune called mid-frame (a
                // same-tab router tearing the route down) is dropped rather than re-entering.
                let Ok(mut running) = running.try_borrow_mut() else {
                    return;
                };
                if let Some(r) = &mut *running {
                    r.input_event(&mut event, &callback_window);
                }
                callback_window.request_redraw();
            }
        });
        let dispose_viewport =
            platform::viewport(canvas, callback.as_ref().unchecked_ref(), embedded);
        let handle = mount(
            canvas,
            callback.as_ref().unchecked_ref(),
            raw.as_ref().unchecked_ref(),
            embedded,
            fit_height,
        );
        Self {
            handle,
            _callback: callback,
            _raw: raw,
            native: Vec::new(),
            candidate: None,
            getters: NativeGetters::new(),
            running,
            dispose_viewport,
            mods: ModifiersState::empty(),
            mirror: Default::default(),
            a11y: Default::default(),
            a11y_revision: 0,
            copy_cache: String::new(),
            fit_height,
        }
    }
}
impl<S: 'static> Drop for Bridge<S> {
    fn drop(&mut self) {
        for registration in self.native.drain(..) { registration.retire(&self.handle); }
        let _ = self.dispose_viewport.call0(&JsValue::NULL);
        destroy(&self.handle);
    }
}

impl<S: 'static> Running<S> {
    fn native_registered(&self, binding: &Rc<NativeBinding>) -> bool {
        binding.live.get() && self.input.getters.as_ref().is_some_and(|g| g.connected(&binding.node))
            && self.input.native.iter().any(|r| Rc::ptr_eq(&r.binding, binding))
    }

    pub(super) fn reconcile_native(&mut self, window: &Arc<Window>) {
        let desired = self.rt.native_controls();
        let old = std::mem::take(&mut self.input.native);
        for registration in old {
            let binding = &registration.binding;
            let locator = native_button(&self.input.handle, &binding.path);
            let bound_node: &JsValue = binding.node.as_ref();
            if desired.iter().any(|(p, k)| p == &binding.path && k == &binding.key)
                && self.input.getters.as_ref().is_some_and(|g| g.connected(&binding.node)) && locator == *bound_node {
                self.input.native.push(registration);
            } else {
                registration.retire(&self.input.handle);
            }
        }
        let Some(getters) = self.input.getters.clone() else { return };
        for (path, key) in desired {
            if self.input.native.iter().any(|r| r.binding.path == path && r.binding.key == key) { continue; }
            let Ok(node) = native_button(&self.input.handle, &path).dyn_into::<web_sys::HtmlElement>() else { continue };
            if !getters.connected(&node) { continue; }
            let binding = Rc::new(NativeBinding { path, key, node: node.clone(), live: Cell::new(true) });
            let weak = self.input.running.clone();
            let b = binding.clone(); let g = getters.clone(); let w = window.clone();
            let click = Closure::<dyn FnMut(web_sys::Event)>::new(move |event| {
                if !b.live.get() || !g.connected(&b.node) || !g.valid(&event, "click", b.node.as_ref(), b.node.as_ref()) { return; }
                let Some(shared) = weak.upgrade() else { return }; let Ok(mut shared) = shared.try_borrow_mut() else { return };
                let Some(r) = shared.as_mut() else { return };
                if r.native_registered(&b) && r.rt.native_press(&b.path, &b.key) { w.request_redraw(); }
            });
            let weak = self.input.running.clone();
            let b = binding.clone(); let g = getters.clone(); let w = window.clone();
            let focus = Closure::<dyn FnMut(web_sys::Event)>::new(move |event| {
                if !b.live.get() || !g.connected(&b.node) || !g.valid(&event, "focus", b.node.as_ref(), b.node.as_ref()) { return; }
                let Some(shared) = weak.upgrade() else { return }; let Ok(mut shared) = shared.try_borrow_mut() else { return };
                let Some(r) = shared.as_mut() else { return };
                if r.native_registered(&b) && r.rt.native_focus(&b.path, &b.key) { w.request_redraw(); }
            });
            let weak = self.input.running.clone();
            let b = binding.clone(); let g = getters.clone(); let w = window.clone();
            let tab = Closure::<dyn FnMut(web_sys::Event)>::new(move |event| {
                if !b.live.get() || !g.connected(&b.node) || !g.valid(&event, "keydown", b.node.as_ref(), b.node.as_ref())
                    || g.value(&g.key, &event).and_then(|v| v.as_string()).as_deref() != Some("Tab")
                    || [&g.ctrl, &g.alt, &g.meta].iter().any(|get| g.value(get, &event).and_then(|v| v.as_bool()) != Some(false)) { return; }
                let Some(shared) = weak.upgrade() else { return }; let Ok(mut shared) = shared.try_borrow_mut() else { return };
                let Some(r) = shared.as_mut() else { return };
                if !r.native_registered(&b) || !r.rt.native_controls().iter().any(|(p,k)| p == &b.path && k == &b.key) { return; }
                event.prevent_default(); event.stop_propagation();
                let shift = g.value(&g.shift, &event).and_then(|v| v.as_bool()) == Some(true);
                r.rt.key(crate::KeyEvent { key: crate::Key::Tab, mods: crate::Mods { shift, ..Default::default() } });
                r.rt.frame(0.0); r.apply_frame_requests(&w); r.sync_input(true); w.request_redraw();
            });
            set_native_button(&self.input.handle, &node, true);
            let listeners = [("click", click.as_ref()), ("focus", focus.as_ref()), ("keydown", tab.as_ref())];
            let mut installed = 0;
            for (name, callback) in &listeners {
                if node.add_event_listener_with_callback(name, callback.unchecked_ref()).is_err() { break; }
                installed += 1;
            }
            if installed != listeners.len() {
                binding.live.set(false);
                for (name, callback) in &listeners[..installed] { let _ = node.remove_event_listener_with_callback(name, callback.unchecked_ref()); }
                set_native_button(&self.input.handle, &node, false);
                continue;
            }
            self.input.native.push(NativeRegistration { binding, click, focus, tab });
        }
        self.focus_native(false);
    }

    fn focus_native(&mut self, requested: bool) {
        if requested {
            self.input.candidate = None;
        } else if self.input.candidate.is_some() {
            return;
        }
        if let Some(path) = self.rt.native_focused_path()
            && let Some(reg) = self.input.native.iter().find(|r| r.binding.path == path && self.native_registered(&r.binding)) {
            if self.rt.native_controls().iter().any(|(p, k)| p == path && k == &reg.binding.key) {
                focus_native_button(&self.input.handle, &reg.binding.node, requested);
            }
        }
    }

    fn native_raw(&mut self, event: &web_sys::Event, canvas: &web_sys::HtmlCanvasElement, window: &Arc<Window>) {
        let Some(g) = self.input.getters.as_ref() else { return };
        let Some(kind) = g.value(&g.kind, event).and_then(|v| v.as_string()) else { return };
        if kind == "pointerdown" { self.input.candidate = None; }
        if kind == "blur" {
            if let Some(dom_window) = web_sys::window()
                && (g.valid(event, "blur", dom_window.as_ref(), dom_window.as_ref())
                    || g.valid(event, "blur", dom_window.as_ref(), canvas.as_ref())) {
                self.input.candidate = None;
            }
            return;
        }
        if kind == "pointercancel" { self.input.candidate = None; return; }
        if kind != "pointerdown" && kind != "click" { return; }
        let candidate = if kind == "click" { self.input.candidate.take() } else { None };
        if candidate.is_some() { window.request_redraw(); }
        let Some(dom_window) = web_sys::window() else { return };
        if !g.in_document(canvas.as_ref())
            || !g.valid(event, &kind, dom_window.as_ref(), canvas.as_ref()) { return; }
        let Some(id) = g.value(&g.pointer_id, event).and_then(|v| v.as_f64()).filter(|v| v.fract() == 0.0 && *v >= 0.0 && *v <= i32::MAX as f64).map(|v| v as i32) else { return };
        if (kind == "pointerdown" && g.value(&g.primary, event).and_then(|v| v.as_bool()) != Some(true))
            || g.value(&g.button, event).and_then(|v| v.as_f64()) != Some(0.0) { return; }
        let (Some(x), Some(y)) = (g.value(&g.x, event).and_then(|v| v.as_f64()), g.value(&g.y, event).and_then(|v| v.as_f64())) else { return };
        if !self.rt.native_geometry_valid { window.request_redraw(); return; }
        let rect = canvas.get_bounding_client_rect();
        let target = self.rt.native_primary_target((x - rect.left()) as f32, (y - rect.top()) as f32);
        if kind == "pointerdown" {
            if let Some((path, key)) = target
                && self.input.native.iter().any(|r| r.binding.path == path && r.binding.key == key && self.native_registered(&r.binding)) {
                self.input.candidate = Some((id, path, key));
            }
        } else if let Some((down_id, path, key)) = candidate
            && down_id == id && target.as_ref() == Some(&(path.clone(), key.clone()))
            && self.input.native.iter().any(|r| r.binding.path == path && r.binding.key == key && self.native_registered(&r.binding))
            && self.rt.native_press(&path, &key) { window.request_redraw(); }
    }

    pub(super) fn sync_input(&mut self, focus: bool) {
        if focus {
            self.input.candidate = None;
        }
        let semantics_changed = if let Some(json) = self.rt.web_a11y(&mut self.input.a11y) {
            self.input.a11y_revision += 1;
            sync_a11y(&self.input.handle, json, self.input.a11y_revision as f64);
            true
        } else {
            false
        };
        let context_changed = self.rt.web_context(&mut self.input.mirror);
        if context_changed {
            let m = &self.input.mirror;
            sync(
                &self.input.handle,
                &m.key,
                &m.text,
                m.base,
                m.anchor,
                m.focus,
                m.secure,
            );
        }
        let next_copy = self.rt.copy_text().unwrap_or_default();
        if self.input.copy_cache != next_copy {
            self.input.copy_cache = next_copy;
            sync_copy(&self.input.handle, &self.input.copy_cache);
        }
        if semantics_changed || context_changed || focus {
            activate(&self.input.handle, focus);
            self.focus_native(focus);
        }
    }

    pub(super) fn input_event(&mut self, event: &mut serde_json::Value, window: &Arc<Window>) {
        if !crate::runtime::provenance::admits(
            event["kind"].as_str().unwrap_or_default(),
            event["trusted"].as_bool() == Some(true),
            event["revision"].as_u64(),
            self.input.a11y_revision,
        ) {
            return;
        }
        let mut activate = false;
        match event["kind"].as_str() {
            Some("viewport") => {
                let insets = std::array::from_fn(|i| {
                    event["insets"][i].as_f64().unwrap_or(0.0).max(0.0) as f32
                });
                if INSETS.replace(insets) != insets {
                    self.rt.native_geometry_valid = false;
                    self.rt.invalidate();
                }
            }
            Some("dpr") => {
                let size = window.inner_size();
                let scale = event["scale"]
                    .as_f64()
                    .unwrap_or_else(|| window.scale_factor())
                    .max(0.5) as f32;
                self.resize_surface(window, size, scale);
            }
            Some("motion") => self
                .rt
                .set_system_reduced_motion(event["reduced"].as_bool().unwrap_or(false)),
            Some("focus") => self
                .rt
                .window_focus(event["focused"].as_bool().unwrap_or(false)),
            Some("visibility") => {
                self.shared.visible.store(
                    event["visible"].as_bool().unwrap_or(false),
                    Ordering::Release,
                );
                return;
            }
            Some("edit" | "selection" | "preedit") => self.rt.web_edit(event),
            Some("sync") => self.input.mirror.key.clear(), // Composition's final sync was deferred by the DOM.
            Some("paste") => {
                let mut content =
                    crate::ClipboardContent::new(event["text"].as_str().unwrap_or_default());
                if let Some(html) = event["html"].as_str().filter(|html| !html.is_empty()) {
                    content = content.with_html(html);
                }
                let handle = &self.input.handle;
                let image = (event["image"].as_bool() == Some(true))
                    .then_some(|| pasted_image(handle, crate::clipboard::MAX_IMAGE_BYTES as u32));
                self.clipboard.paste_event(&mut self.rt, content, image);
            }
            Some("secure") => {
                if let serde_json::Value::String(text) = event["text"].take() {
                    self.rt
                        .web_secure(event["key"].as_str().unwrap_or_default(), text);
                }
            }
            Some("key") => activate = self.rt.web_key(event),
            Some("a11y_focus") => activate = self.rt.web_a11y_action(event),
            Some("a11y_press" | "a11y_link") => {
                let _ = self.rt.web_a11y_action(event);
            }
            _ => {}
        }
        // Commit within the DOM event: clipboard, link and soft-keyboard APIs need activation.
        self.rt.frame(0.0);
        self.apply_frame_requests(window);
        self.clipboard.flush(&mut self.rt);
        self.sync_input(activate);
    }

    pub(super) fn window_input(&mut self, event: &WindowEvent, window: &Arc<Window>) -> bool {
        let scale = window.scale_factor() as f32;
        let mut activate = false;
        let mut immediate = false;
        match event {
            WindowEvent::ModifiersChanged(mods) => {
                self.input.mods = mods.state();
                return false;
            }
            WindowEvent::CursorMoved { position, .. } if !self.rt.touching() => {
                self.rt
                    .pointer_move(position.x as f32 / scale, position.y as f32 / scale);
                self.cursor.apply(window, self.rt.cursor(), true);
            }
            WindowEvent::CursorLeft { .. } if !self.rt.touching() => {
                self.rt.pointer_leave();
                self.cursor.apply(window, crate::el::Cursor::Default, false);
            }
            WindowEvent::MouseInput { button, state, .. } if !self.rt.touching() => {
                // Keep presented hit geometry available until the native click arrives.
                immediate = *button != MouseButton::Left || self.input.candidate.is_none();
                let Some((x, y)) = self.rt.pointer() else {
                    return false;
                };
                match (button, state) {
                    (MouseButton::Left, ElementState::Pressed) => {
                        self.rt.pointer_down(x, y);
                        activate = true;
                    }
                    (MouseButton::Left, ElementState::Released) => self.rt.pointer_up(x, y),
                    (MouseButton::Right, ElementState::Pressed) => self.rt.secondary_down(x, y),
                    (MouseButton::Right, ElementState::Released) => self.rt.secondary_up(x, y),
                    (&button, ElementState::Pressed) => {
                        self.rt
                            .button_down(x, y, crate::window::pointer_button(button))
                    }
                    (&button, ElementState::Released) => {
                        self.rt
                            .button_up(x, y, crate::window::pointer_button(button))
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some((x, y)) = self.rt.pointer() else {
                    return false;
                };
                let (dx, dy) =
                    crate::runtime::wheel_delta(*delta, scale, self.input.mods.shift_key());
                if self.input.mods.control_key() {
                    return self.rt.pinch_wheel(x, y, dy);
                }
                // On a content-sized embedded canvas, vertical wheel belongs to the page unless
                // a horizontal strip is being scrolled.
                if self.input.fit_height && dx.abs() <= dy.abs() {
                    return false;
                }
                let precise = matches!(delta, winit::event::MouseScrollDelta::PixelDelta(_));
                self.rt.wheel_event(
                    crate::WheelEvent::new(dx, dy)
                        .at(x, y)
                        .mods(modifiers(self.input.mods))
                        .precise(precise),
                );
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                immediate = true;
                let pressed = event.state == ElementState::Pressed;
                let mut logical = (pressed || self.rt.wants_key_up())
                    .then(|| crate::window::map_key(&event.logical_key, self.input.mods))
                    .flatten();
                if let Some(key) = &mut logical {
                    key.mods = modifiers(self.input.mods);
                    if key.mods.cmd
                        && let crate::Key::Text(text) = &key.key
                        && text != " "
                        && text.chars().count() == 1
                    {
                        key.key =
                            crate::Key::Command(text.chars().next().unwrap().to_ascii_lowercase());
                    }
                }
                activate = logical.as_ref().is_some_and(|k| k.key == crate::Key::Tab);
                if let winit::keyboard::PhysicalKey::Code(key) = event.physical_key {
                    self.rt.physical_with_fallback(
                        crate::PhysicalInput::Key {
                            key,
                            mods: modifiers(self.input.mods),
                            pressed: event.state == ElementState::Pressed,
                            repeat: event.repeat,
                        },
                        logical,
                    );
                } else if let Some(key) = logical {
                    if pressed {
                        self.rt.key(key);
                    } else {
                        self.rt.key_up(key);
                    }
                }
            }
            WindowEvent::Touch(touch) if !self.input.fit_height => {
                crate::window::touch(&mut self.rt, touch, scale);
                // A tap is a user activation: links, the clipboard and focus act before the handler returns.
                if matches!(touch.phase, TouchPhase::Ended) {
                    activate = true;
                    immediate = self.input.candidate.is_none();
                }
            }
            _ => return false,
        }
        // Moves and wheels are coalesced by rAF. Only activation-sensitive input must
        // commit before the DOM handler returns (clipboard, links and keyboard focus).
        if immediate {
            self.rt.frame(0.0);
            self.apply_frame_requests(window);
            self.clipboard.flush(&mut self.rt);
            self.sync_input(activate);
            self.cursor
                .apply(window, self.rt.cursor(), self.rt.pointer().is_some());
        }
        true
    }
}

fn modifiers(mods: ModifiersState) -> crate::Mods {
    crate::Mods {
        shift: mods.shift_key(),
        alt: mods.alt_key(),
        ctrl: mods.control_key(),
        logo: mods.super_key(),
        cmd: mods.super_key() || mods.control_key(),
    }
}
