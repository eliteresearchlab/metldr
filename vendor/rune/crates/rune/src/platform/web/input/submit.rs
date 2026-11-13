use super::*;
use crate::runtime::web_runtime::NativeSubmitBinding;

struct Lifecycle {
    generation: Cell<u64>,
    composing: Cell<bool>,
    needs_sync: Cell<bool>,
}

impl Lifecycle {
    fn invalidate(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.needs_sync.set(true);
    }
}

struct FieldGetters {
    active: Function,
    value: Function,
    disabled: Function,
    read_only: Function,
    inert: Function,
    key_code: Function,
}

impl FieldGetters {
    fn new(node: &web_sys::HtmlElement) -> Option<Self> {
        let document = web_sys::window()?.document()?;
        let keyboard = web_sys::KeyboardEvent::new("keydown").ok()?;
        Some(Self {
            active: inherited_getter(document.as_ref(), "activeElement")?,
            value: inherited_getter(node.as_ref(), "value")?,
            disabled: inherited_getter(node.as_ref(), "disabled")?,
            read_only: inherited_getter(node.as_ref(), "readOnly")?,
            inert: inherited_getter(node.as_ref(), "inert")?,
            key_code: inherited_getter(keyboard.as_ref(), "keyCode")?,
        })
    }

    fn ready(&self, node: &web_sys::HtmlElement, value: &str) -> bool {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return false };
        let target: &JsValue = node.as_ref();
        self.active.call0(document.as_ref()).is_ok_and(|active| active == *target)
            && self.value.call0(target).ok().and_then(|v| v.as_string()).as_deref() == Some(value)
            && [&self.disabled, &self.read_only, &self.inert].iter()
                .all(|getter| getter.call0(target).ok().and_then(|v| v.as_bool()) == Some(false))
    }
}

pub(super) struct NativeSubmitField {
    node: web_sys::HtmlElement,
    lifecycle: Rc<Lifecycle>,
    getters: FieldGetters,
    binding: Option<(u64, NativeSubmitBinding)>,
    keydown: Closure<dyn FnMut(web_sys::Event)>,
    lifecycle_event: Closure<dyn FnMut(web_sys::Event)>,
}

impl NativeSubmitField {
    pub(super) fn new<S: 'static>(
        node: web_sys::HtmlElement,
        getters: NativeGetters,
        running: std::rc::Weak<RefCell<Option<Running<S>>>>,
        window: Arc<Window>,
    ) -> Option<Self> {
        if node.tag_name() != "TEXTAREA" { return None; }
        let field_getters = FieldGetters::new(&node)?;
        let lifecycle = Rc::new(Lifecycle {
            generation: Cell::new(0), composing: Cell::new(false), needs_sync: Cell::new(true),
        });
        let key_window = window.clone();
        let keydown = Closure::<dyn FnMut(web_sys::Event)>::new(move |event| {
            let Some(shared) = running.upgrade() else { return };
            let Ok(mut shared) = shared.try_borrow_mut() else { return };
            if let Some(running) = shared.as_mut() {
                running.native_field_keydown(&event, &key_window);
            }
        });
        let field = node.clone();
        let life = Rc::clone(&lifecycle);
        let lifecycle_event = Closure::<dyn FnMut(web_sys::Event)>::new(move |event| {
            let Some(kind) = getters.value(&getters.kind, &event).and_then(|v| v.as_string()) else { return };
            if !getters.valid(&event, &kind, field.as_ref(), field.as_ref()) { return; }
            match kind.as_str() {
                "compositionstart" => life.composing.set(true),
                "compositionend" | "blur" => life.composing.set(false),
                "focus" => {},
                _ => return,
            }
            // This veto must survive a synchronous event while Running is already borrowed.
            life.invalidate();
            window.request_redraw();
        });
        let registration = Self { node, lifecycle, getters: field_getters, binding: None, keydown, lifecycle_event };
        registration.node.add_event_listener_with_callback_and_bool("keydown", registration.keydown.as_ref().unchecked_ref(), true).ok()?;
        for name in ["compositionstart", "compositionend", "blur", "focus"] {
            registration.node.add_event_listener_with_callback_and_bool(name, registration.lifecycle_event.as_ref().unchecked_ref(), true).ok()?;
        }
        Some(registration)
    }

    fn invalidate(&mut self) {
        self.binding = None;
        self.lifecycle.invalidate();
    }

    pub(super) fn synced(&self) {
        if !self.lifecycle.composing.get() { self.lifecycle.needs_sync.set(false); }
    }
}

impl Drop for NativeSubmitField {
    fn drop(&mut self) {
        self.invalidate();
        let _ = self.node.remove_event_listener_with_callback_and_bool("keydown", self.keydown.as_ref().unchecked_ref(), true);
        for name in ["compositionstart", "compositionend", "blur", "focus"] {
            let _ = self.node.remove_event_listener_with_callback_and_bool(name, self.lifecycle_event.as_ref().unchecked_ref(), true);
        }
    }
}

impl<S: 'static> Running<S> {
    pub(super) fn retire_stale_submit(&mut self) {
        let previous = self.input.submit.as_ref().and_then(|field| field.binding.as_ref()).map(|(_, binding)| binding.clone());
        let Some(previous) = previous else { return };
        if !self.rt.native_submit_binding().is_some_and(|current| previous.matches(&current)) {
            if let Some(field) = &mut self.input.submit { field.invalidate(); }
        }
    }

    pub(super) fn reconcile_submit(&mut self) {
        let Some(field) = &mut self.input.submit else { return };
        let current = self.rt.native_submit_binding();
        field.binding = current.filter(|binding| {
            !field.lifecycle.composing.get() && !field.lifecycle.needs_sync.get()
                && self.input.getters.as_ref().is_some_and(|g| g.in_document(field.node.as_ref()))
                && binding.matches_mirror(&self.input.mirror) && field.getters.ready(&field.node, &binding.window)
        }).map(|binding| (field.lifecycle.generation.get(), binding));
    }

    fn native_field_keydown(&mut self, event: &web_sys::Event, window: &Arc<Window>) {
        let (Some(field), Some(getters)) = (&self.input.submit, &self.input.getters) else { return };
        if !getters.in_document(field.node.as_ref())
            || !getters.valid(event, "keydown", field.node.as_ref(), field.node.as_ref())
            || getters.value(&getters.key, event).and_then(|v| v.as_string()).as_deref() != Some("Enter")
            || getters.value(&getters.shift, event).and_then(|v| v.as_bool()) != Some(false) { return; }
        match field.getters.key_code.call0(event.as_ref()).ok().and_then(|v| v.as_f64()) {
            Some(229.0) => { field.lifecycle.invalidate(); window.request_redraw(); return; }
            Some(_) => {},
            None => return,
        }
        let Some((generation, binding)) = &field.binding else { return };
        if *generation != field.lifecycle.generation.get() || field.lifecycle.composing.get()
            || field.lifecycle.needs_sync.get() || !binding.matches_mirror(&self.input.mirror)
            || !field.getters.ready(&field.node, &binding.window) { return; }
        let binding = binding.clone();
        if self.rt.native_submit(&binding, event) {
            event.prevent_default();
            event.stop_immediate_propagation();
            self.input.candidate = None;
            if let Some(field) = &mut self.input.submit { field.invalidate(); }
            window.request_redraw();
        }
    }
}
