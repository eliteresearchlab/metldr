//! What every in-page frame shares. Each frame over a page (the word popup, the PDF tools) runs
//! one Rune app of the chat's own build in an extension frame, so it keeps the extension's policy
//! on every site. The frame takes its jobs from the extension, never from the page, and tells the
//! page only the size it needs and when to close.

use super::State;
use metldr_ui::theme::{Palette, Typography};
use ::rune::{App, AppSpec, Cx, El, KeyEvent, Tx, WindowOptions};
use std::cell::{Cell, RefCell};
use wasm_bindgen::prelude::*;

/// Room around a frame's content for its shadow, in points on each side.
pub(super) const MARGIN: f32 = 16.0;

thread_local! {
    static THEME: RefCell<String> = RefCell::new("dark".into());
    static HOST: RefCell<Option<::rune::AppHandle<State>>> = const { RefCell::new(None) };
    static CLOSE: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

/// One kind of frame: its app and the keyed element whose size is the frame's.
pub(super) struct Frame {
    pub title: &'static str,
    /// The frame's starting size in points, before its first frame reports one.
    pub size: (f32, f32),
    /// The element the frame fits: its presented size plus [`MARGIN`] on each side.
    pub fits: &'static str,
    pub init: fn() -> State,
    pub view: fn(&State, &mut Cx<State>) -> El,
    pub on_key: fn(&mut State, &KeyEvent, &mut Tx) -> bool,
}

/// The theme the reader picked, for a frame's first state.
pub(super) fn palette() -> (Palette, Typography) {
    let palette = THEME.with_borrow(|theme| Palette::for_theme(theme));
    (palette, Typography::new(1.0, &palette))
}

/// Starts `frame` in `theme` with the interface `fonts`; `sized(width, height)` hears the frame
/// size its content needs, and `close()`, when given, that the reader dismissed it.
pub(super) fn start(
    frame: Frame,
    fonts: js_sys::Array,
    theme: String,
    sized: js_sys::Function,
    close: Option<js_sys::Function>,
) -> Result<(), JsValue> {
    let fonts = metldr_ui::fonts::read(&fonts, &metldr_ui::fonts::NAMES);
    let fonts = fonts.map_err(|error| JsValue::from_str(&error))?;
    THEME.with_borrow_mut(|stored| *stored = theme);
    CLOSE.with_borrow_mut(|stored| *stored = close);
    let spec = AppSpec {
        fonts: metldr_ui::fonts::retain(fonts),
        families: metldr_ui::fonts::FAMILIES,
        on_key: Some(frame.on_key),
        ..AppSpec::new(frame.title, frame.size, frame.init, frame.view)
    };
    let fits = frame.fits;
    let last = Cell::new((0.0f32, 0.0f32));
    let app = App::new(spec)
        .canvas_id("metldr-inpage")
        .full_viewport()
        .window_options(WindowOptions { transparent: true, ..Default::default() })
        .on_frame_drawn(move |runtime| {
            let Some(([_, _, w, h], _)) = runtime.presented(fits) else { return };
            let size = ((w + 2.0 * MARGIN).ceil(), (h + 2.0 * MARGIN).ceil());
            if size != last.replace(size) {
                let _ = sized.call2(&JsValue::NULL, &size.0.into(), &size.1.into());
            }
        });
    let handle = app.handle();
    app.try_run().map_err(|error| JsValue::from_str(&error))?;
    HOST.with_borrow_mut(|host| *host = Some(handle));
    Ok(())
}

/// Tells the page the reader dismissed the frame.
pub(super) fn close() {
    CLOSE.with_borrow(|close| close.as_ref().map(|close| close.call0(&JsValue::NULL)));
}

/// Changes the running frame's state and redraws it.
pub(super) fn update(change: impl FnOnce(&mut State) + Send + 'static) {
    HOST.with_borrow(|host| {
        if let Some(host) = host {
            host.update(move |state: &mut State, _| change(state));
        }
    });
}

/// Redraws the frame in `theme`, the one the reader just picked.
#[wasm_bindgen]
pub fn inpage_theme(theme: String) {
    let palette = Palette::for_theme(&theme);
    THEME.with_borrow_mut(|stored| *stored = theme);
    let typography = Typography::new(1.0, &palette);
    update(move |state| {
        if let Some(popup) = state.word_popup.as_deref_mut() {
            popup.retheme(palette, typography);
        }
        if let Some(tools) = state.pdf_tools.as_deref_mut() {
            tools.retheme(palette, typography);
        }
    });
}
