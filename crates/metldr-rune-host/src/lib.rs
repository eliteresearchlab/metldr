//! Internal Rune host route: mount lifecycle, font intake and the renderer glue.
//!
//! Built as its own wasm module, so the shipping panel never contains or downloads it. The page
//! loads it only with `?host=rune`. The lifecycle and font checks have no renderer dependency,
//! so `cargo test` covers them without compiling Rune.
#![cfg_attr(not(feature = "renderer"), allow(dead_code))]

mod fonts;
mod lifecycle;

#[cfg(feature = "renderer")]
mod app;
