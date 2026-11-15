//! Field: a control with its visible label, an optional description and an error message, tied
//! together for assistive technology. The control takes the label as its accessible name and
//! the description as its description; while the field is invalid the control reads as invalid
//! and the error shows under it as an alert, so it is read when it appears. Not ported: the
//! browser's own constraint validation and required flag (Rune fields have no form submission).

use rune::El;

/// What a field shows around its control.
#[derive(Clone, Copy, Default)]
pub struct Props<'a> {
    /// Read after the name, and shown under the control.
    pub description: Option<&'a str>,
    /// Shown, and the control invalid, while it is set.
    pub error: Option<&'a str>,
}

/// `control` named `name` under its visible `label`, with `props`' description and error, each
/// built by `text` so the field takes the theme's type.
pub fn field(label: El, control: El, name: impl Into<String>, props: Props<'_>, text: impl Fn(&str) -> El) -> El {
    let mut control = control.label(name).invalid(props.error.is_some());
    if let Some(description) = props.description {
        control = control.description(description);
    }
    // The control carries the name, so the visible label is not read a second time.
    let mut field = rune::col().gap(3.0).child(label.a11y_hidden()).child(control);
    if let Some(description) = props.description {
        field = field.child(text(description).a11y_hidden());
    }
    if let Some(error) = props.error {
        field = field.child(text(error).role("alert"));
    }
    field
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Cx, Env, Runtime, TextEdit, TextStyle, text, text_input};
    use serde_json::Value;

    fn view(state: &(TextEdit, bool), _: &mut Cx<(TextEdit, bool)>) -> El {
        let props = Props { description: Some("Where requests go"), error: state.1.then_some("Enter a URL") };
        field(text("Endpoint"), text_input(&state.0, TextStyle::new(14.0, 400.0)).key("endpoint").w(200.0), "OpenAI endpoint", props, |t| text(t.to_owned()))
    }

    #[test]
    fn the_control_carries_the_name_description_and_error() {
        for invalid in [false, true] {
            let mut rt = Runtime::new((TextEdit::new(""), invalid), view, (300.0, 200.0), Env::new(false, 1.0));
            rt.frame(0.0);
            let items: Vec<Value> = serde_json::from_str(&rt.a11y_json()).unwrap();
            let input = items.iter().find(|i| i["k"] == "input").expect("the control");
            assert_eq!(input["name"], "OpenAI endpoint");
            assert_eq!(input["description"], "Where requests go");
            assert_eq!(input["invalid"], invalid);
            assert!(!items.iter().any(|i| i["t"] == "Endpoint"), "the visible label is not read twice");
            assert_eq!(items.iter().any(|i| i["k"] == "alert" && i["t"] == "Enter a URL"), invalid);
        }
    }
}
