//! Renderer glue for the internal Rune host. The only file that names Rune types.
//!
//! The host owns one Rune app per document. Startup refusals and the GPU failures that the
//! renderer reports asynchronously come back through `settle_rune_host`, so the page can show
//! them outside the canvas.

use std::cell::{Cell, RefCell};

use ::rune::{App, AppHandle, AppSpec, Cx, El, TextStyle, Tx, col, oklch, text};
use js_sys::{Array, Uint8Array};
use wasm_bindgen::prelude::*;

use super::{
    fonts,
    lifecycle::{Lifecycle, Phase, Refusal, Session},
};

/// Renderer state. The placeholder view reads nothing from it yet; the session identifies the
/// mount that the renderer belongs to.
pub(super) struct State {
    session: Session,
}

struct Live {
    session: Session,
    handle: AppHandle<State>,
}

#[derive(Default)]
struct Host {
    lifecycle: Lifecycle,
    live: Option<Live>,
}

thread_local! {
    static HOST: RefCell<Host> = RefCell::new(Host::default());
    /// `performance.now()` at the first frame of the live mount; NaN until one draws.
    static FIRST_FRAME_MS: Cell<f64> = const { Cell::new(f64::NAN) };
}

/// The renderer calls this without arguments. The only mount that can reach it is the one
/// `start_rune_host` began, so its session is the lifecycle's current one.
fn init() -> State {
    State {
        session: HOST.with(|host| host.borrow().lifecycle.current()),
    }
}

fn view(_state: &State, cx: &mut Cx<State>) -> El {
    let (ink, page) = if cx.system_dark() {
        (oklch(0.964, 0.002, 247.839), oklch(0.209, 0.004, 264.477))
    } else {
        (oklch(0.247, 0.006, 258.361), oklch(0.985, 0.001, 286.376))
    };
    col()
        .w_full()
        .h_full()
        .fill(page)
        .color(ink)
        .padding(24.0)
        .child(
            text("metldr")
                .label("metldr")
                .font(TextStyle::new(14.0, 400.0).lh(21.0))
                .color(ink),
        )
}

fn poll(state: &mut State, _now: f64, _tx: &mut Tx) -> bool {
    // The first poll of a frame means the renderer is about to draw. A callback from a stale
    // mount must not record a time for the current one.
    let accepts = HOST.with(|host| host.borrow().lifecycle.accepts_frames(state.session));
    if accepts && FIRST_FRAME_MS.with(Cell::get).is_nan() {
        FIRST_FRAME_MS.with(|cell| cell.set(now_ms()));
    }
    false
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(f64::NAN, |performance| performance.now())
}

fn refusal(refusal: Refusal) -> JsValue {
    JsValue::from_str(match refusal {
        Refusal::Busy(Phase::Closing) => {
            "The Rune host is still closing. Wait for it to settle, then start again."
        }
        Refusal::Busy(_) => "The Rune host is already mounted in this document.",
        Refusal::Stale => "The Rune host request is stale.",
    })
}

/// Mounts the host. `fonts` must be the 13 decoded SFNT files in manifest order. Fails before the
/// renderer starts when the files are wrong, when a mount is already live, or when the renderer
/// refuses the document at once.
#[wasm_bindgen]
pub fn start_rune_host(fonts: Array) -> Result<(), JsValue> {
    let files = fonts
        .iter()
        .map(|value| value.dyn_into::<Uint8Array>().map(|bytes| bytes.to_vec()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| JsValue::from_str("Font files must be Uint8Array values."))?;
    let views: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    fonts::validate(&views).map_err(|error| JsValue::from_str(&error))?;

    let session = HOST
        .with(|host| host.borrow_mut().lifecycle.begin())
        .map_err(refusal)?;
    FIRST_FRAME_MS.with(|cell| cell.set(f64::NAN));
    let spec = AppSpec {
        fonts: fonts::retain(files),
        families: fonts::FAMILIES,
        poll: Some(poll),
        ..AppSpec::new("metldr", (640.0, 720.0), init, view)
    };
    let app = App::new(spec).full_viewport();
    let handle = app.handle();
    let started = app.try_run();

    HOST.with(|host| {
        let mut host = host.borrow_mut();
        match started {
            Ok(()) => {
                host.live = Some(Live { session, handle });
                Ok(())
            }
            Err(error) => {
                let _ = host.lifecycle.failed(session);
                Err(JsValue::from_str(&error))
            }
        }
    })
}

/// Begins closing the live mount. The renderer tears itself down asynchronously; the page polls
/// `settle_rune_host` until it reports `idle`. Does nothing when no mount is live.
#[wasm_bindgen]
pub fn stop_rune_host() {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.lifecycle.close().is_some() {
            if let Some(live) = &host.live {
                live.handle.quit();
            }
        }
    });
}

/// Advances the host and reports its phase: `starting`, `running`, `closing` or `idle`.
/// Throws the renderer's message when startup fails after `start_rune_host` returned.
#[wasm_bindgen]
pub fn settle_rune_host() -> Result<String, JsValue> {
    HOST.with(|host| {
        let mut guard = host.borrow_mut();
        let host = &mut *guard;
        match host.lifecycle.phase() {
            Phase::Idle => Ok("idle".into()),
            Phase::Running => Ok("running".into()),
            Phase::Starting => {
                let Some(live) = host.live.as_ref() else {
                    return Ok("starting".into());
                };
                let (session, error, visible) = (
                    live.session,
                    live.handle.window_error(),
                    live.handle.is_visible(),
                );
                if let Some(error) = error {
                    host.live = None;
                    let _ = host.lifecycle.failed(session);
                    return Err(JsValue::from_str(&error));
                }
                if visible {
                    let _ = host.lifecycle.mounted(session);
                    return Ok("running".into());
                }
                Ok("starting".into())
            }
            Phase::Closing => {
                let Some(live) = host.live.as_ref() else {
                    return Ok("closing".into());
                };
                if live.handle.is_visible() {
                    return Ok("closing".into());
                }
                let session = live.session;
                host.live = None;
                let _ = host.lifecycle.unmounted(session);
                Ok("idle".into())
            }
        }
    })
}

/// `performance.now()` at the first frame of the live mount, or NaN until one has drawn.
#[wasm_bindgen]
pub fn rune_host_first_frame_ms() -> f64 {
    FIRST_FRAME_MS.with(Cell::get)
}
