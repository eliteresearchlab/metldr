use wasm_bindgen::prelude::*;

use crate::composer::choices::Choice;

#[wasm_bindgen]
pub struct UiDestination {
    choice: Choice,
}

impl From<Choice> for UiDestination {
    fn from(choice: Choice) -> Self {
        Self { choice }
    }
}

#[wasm_bindgen]
impl UiDestination {
    #[wasm_bindgen(getter)]
    pub fn value(&self) -> String {
        self.choice.value.to_string()
    }

    #[wasm_bindgen(getter)]
    pub fn label(&self) -> String {
        self.choice.label.to_string()
    }

    #[wasm_bindgen(getter)]
    pub fn enabled(&self) -> bool {
        self.choice.enabled
    }

    #[wasm_bindgen(getter)]
    pub fn supports_plan(&self) -> bool {
        self.choice.value == "codex" || self.choice.value.starts_with("codex:")
    }
}
