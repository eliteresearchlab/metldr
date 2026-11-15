//! What `El` holds for focus and keys: `FocusSpec`, kept behind `Cold` because most elements set none of it.
use crate::el::{InputKeyFn, StepFn, StepLimitFn};
use crate::color::Color;
use crate::el::Outline;

/// Which arrow keys move through a focus group ([`crate::El::focus_axis`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusAxis {
    /// Left and Right; Up and Down go on to the app.
    Horizontal,
    /// Up and Down; Left and Right go on to the app.
    Vertical,
    /// All four arrows.
    Both,
}

#[derive(Clone)]
pub(crate) struct FocusSpec {
    pub physical: Option<crate::input::physical_input::Handler>,
    pub enabled: bool,
    pub order: i32,
    pub group: bool,
    pub selected: bool,
    pub reveal_selected: bool,
    pub preserve_on_press: crate::runtime::selectable::OnPress,
    pub ring: Option<Outline>,
    /// `no_focus_ring`: no default ring on this control, and none from this element.
    pub ring_off: bool,
    /// `default_button`: Return presses it.
    pub default: bool,
    pub highlight: Option<Color>,
    pub follows: bool,
    /// `focus_axis`: the group's arrows, in place of its role's.
    pub axis: Option<FocusAxis>,
    /// `focus_wrap`: whether the group wraps at its ends, in place of its role's.
    pub wrap: Option<bool>,
    pub return_to: Option<String>,
    /// `on_step`: a slider's arrow and page keys.
    pub step: Option<StepFn>,
    /// `on_step_limit`: a slider's Home and End.
    pub step_limit: Option<StepLimitFn>,
    /// `on_key_up`.
    pub key_up: Option<InputKeyFn>,
    /// `El::on_key`: keys while this element or one inside it has focus.
    pub key: Option<InputKeyFn>,
}

impl crate::cold::Empty for FocusSpec {
    const EMPTY: &'static Self = &FocusSpec {
        physical: None,
        enabled: false,
        order: 0,
        group: false,
        selected: false,
        reveal_selected: false,
        preserve_on_press: crate::runtime::selectable::OnPress { focus: false, selection: false },
        ring: None,
        ring_off: false,
        default: false,
        highlight: None,
        follows: false,
        axis: None,
        wrap: None,
        return_to: None,
        step: None,
        step_limit: None,
        key_up: None,
        key: None,
    };
}
