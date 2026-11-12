use js_sys::{Function, Object, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Event, Node};

use crate::model::FlowError;

pub(super) struct Activation {
    trusted: Function,
    kind: Function,
    phase: Function,
    current: Function,
    target: Function,
}

impl Activation {
    pub(super) fn new() -> Result<Self, FlowError> {
        let sample = Event::new("click").map_err(|_| rejected())?;
        let value: &JsValue = sample.as_ref();
        let object: &Object = value.unchecked_ref();
        let prototype = Object::get_prototype_of(value);
        Ok(Self {
            trusted: getter(object, "isTrusted")?,
            kind: getter(&prototype, "type")?,
            phase: getter(&prototype, "eventPhase")?,
            current: getter(&prototype, "currentTarget")?,
            target: getter(&prototype, "target")?,
        })
    }

    pub(super) fn require(&self, event: &JsValue) -> Result<(), FlowError> {
        self.require_kind(event, "click")
    }

    fn require_kind(&self, event: &JsValue, kind: &str) -> Result<(), FlowError> {
        // Invoke platform getters directly so a plain object or overridden field cannot supply trust.
        let target = self
            .target
            .call0(event)
            .ok()
            .and_then(|v| v.dyn_into::<Node>().ok());
        let document = web_sys::window().and_then(|w| w.document());
        let in_document = match (target, document) {
            (Some(target), Some(document)) => target
                .owner_document()
                .is_some_and(|owner| owner == document),
            _ => false,
        };
        admit(Dispatch {
            trusted: self.trusted.call0(event).ok().and_then(|v| v.as_bool()) == Some(true),
            kind: self
                .kind
                .call0(event)
                .ok()
                .and_then(|v| v.as_string())
                .as_deref()
                == Some(kind),
            dispatching: self
                .phase
                .call0(event)
                .ok()
                .and_then(|v| v.as_f64())
                .is_some_and(|phase| (1.0..=3.0).contains(&phase)),
            current_target: self.current.call0(event).is_ok_and(|v| !v.is_null()),
            in_document,
        })
    }
}

/// What the browser reported about one event, read through the platform getters.
#[derive(Clone, Copy)]
pub(super) struct Dispatch {
    /// `isTrusted`: the browser created the event, not page script.
    pub(super) trusted: bool,
    /// `type` is the activation kind the caller asked for.
    pub(super) kind: bool,
    /// `eventPhase` is capturing, at target or bubbling: the event is being dispatched now.
    pub(super) dispatching: bool,
    /// `currentTarget` is set. The browser clears it once dispatch ends.
    pub(super) current_target: bool,
    /// The target belongs to this document.
    pub(super) in_document: bool,
}

/// Admits an activation only when every fact holds. Kept apart from the DOM reads so each rule has
/// a native test.
pub(super) fn admit(dispatch: Dispatch) -> Result<(), FlowError> {
    let Dispatch {
        trusted,
        kind,
        dispatching,
        current_target,
        in_document,
    } = dispatch;
    if trusted && kind && dispatching && current_target && in_document {
        Ok(())
    } else {
        Err(rejected())
    }
}

pub(super) struct SubmissionActivation {
    click: Activation,
    key: Function,
    shift: Function,
    composing: Function,
    repeat: Function,
}

impl SubmissionActivation {
    pub(super) fn new() -> Result<Self, FlowError> {
        let event = web_sys::KeyboardEvent::new("keydown").map_err(|_| rejected())?;
        let prototype = Object::get_prototype_of(event.as_ref());
        Ok(Self {
            click: Activation::new()?,
            key: getter(&prototype, "key")?,
            shift: getter(&prototype, "shiftKey")?,
            composing: getter(&prototype, "isComposing")?,
            repeat: getter(&prototype, "repeat")?,
        })
    }

    pub(super) fn require(&self, event: &JsValue) -> Result<(), FlowError> {
        if self
            .click
            .kind
            .call0(event)
            .ok()
            .and_then(|v| v.as_string())
            .as_deref()
            == Some("click")
        {
            return self.click.require(event);
        }
        self.click.require_kind(event, "keydown")?;
        if self
            .key
            .call0(event)
            .ok()
            .and_then(|v| v.as_string())
            .as_deref()
            != Some("Enter")
            || [&self.shift, &self.composing, &self.repeat]
                .iter()
                .any(|getter| getter.call0(event).ok().and_then(|v| v.as_bool()) != Some(false))
        {
            return Err(rejected());
        }
        Ok(())
    }
}

fn getter(object: &Object, name: &str) -> Result<Function, FlowError> {
    let descriptor = Object::get_own_property_descriptor(object, &name.into());
    Reflect::get(&descriptor, &"get".into())
        .map_err(|_| rejected())?
        .dyn_into()
        .map_err(|_| rejected())
}

fn rejected() -> FlowError {
    FlowError::InvalidInput("Use a current click in this panel to answer the request.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: Dispatch = Dispatch {
        trusted: true,
        kind: true,
        dispatching: true,
        current_target: true,
        in_document: true,
    };

    #[test]
    fn pointer_keyboard_and_assistive_activation_is_admitted() {
        // A pointer click, Enter on a focused control and an assistive click all reach the gate as
        // trusted events in dispatch. The gate has no input-method branch, so one shape covers them.
        assert!(admit(LIVE).is_ok());
    }

    #[test]
    fn synthetic_events_are_refused() {
        assert!(
            admit(Dispatch {
                trusted: false,
                ..LIVE
            })
            .is_err()
        );
    }

    #[test]
    fn events_delivered_after_their_dispatch_are_refused() {
        assert!(
            admit(Dispatch {
                dispatching: false,
                current_target: false,
                ..LIVE
            })
            .is_err()
        );
    }
}
