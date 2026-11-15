# Rune source

Source: https://github.com/anomalyco/rune
Revision: 781e2b6d87cd5f32d6f4365956fad4baaf24ee50

This directory keeps the Rune runtime, motion, staging, layout and optional profiling crates with their original relative paths. The workspace lists these crates and retains the upstream edition, license and lint settings. Dependency versions are owned by the consuming crate lockfile.

Local changes:

* The browser calls the existing frame-drawn hook after presentation, allowing restricted answer frames to follow submitted canvas geometry.
* Application fallback fonts are registered for Hebrew, Arabic, Devanagari, Bengali and Thai as well as CJK. Browser symbol fallback recognizes the supplied Noto Sans Symbols 2 family.
* The WebAssembly GPU instance cache uses thread local storage; native targets retain the upstream mutex.
* Browser field semantics share Rust paths and focus state with the existing text service bridge. Secure field values are omitted from the semantic mirror.
* Disabled fields cannot take focus through traversal or restoration, or retain offscreen editing focus.
* Text test fixtures use the identical font already bundled in the runtime.
* Focus and semantic mirror regressions cover these changes.
* A composition whose end event is refused as untrusted is finished by the next trusted input or key that is no longer composing, so the field never stays in a preedit. Mirror nodes rewrite their style only when it changes.
* `El::my_auto` sets auto top and bottom margins, so a centred column in a scroll viewport keeps its top reachable when it overflows.
* On the web every keyed control with a press handler takes the native mirror button path for focus and keyboard or assistive activation, not only controls marked `native_activation`; pointer presses on the canvas are unchanged. A focused mirror button forwards Tab, Escape, the arrows, Home, End, Page Up and Page Down to the runtime, and a held Enter or Space does not repeat the press.
* A touch holds the focus change its press asked for until the finger lifts, and drops it when the touch turns into a pan, as a browser focuses only on a tap. A scroll step after the press is cancelled skips the press search.
* The web mirror gives each pressable control its ARIA role (checkbox, switch, radio, menu item, tab, option, tree item) and state (pressed, checked, selected, disabled, busy), and `El::busy` marks a control whose action is still running. Mirror attributes are rewritten only when they change.
* Web input stops untrusted (`isTrusted` false) input types in the window capture phase, before the page's listeners and winit's. Mirror presses and focus name the browser's trust and the mirror revision they were rendered from, and the runtime refuses any other (`runtime/input/provenance.rs`).

The MIT license and bundled Inter and IBM Plex Mono font licenses remain beside their source assets.

Line endings and trailing blank lines are normalized without changing source tokens or license wording.
