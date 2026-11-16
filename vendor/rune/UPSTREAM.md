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
* `El::unavailable` marks a control that reads as disabled yet keeps focus and its place in the tab order (ARIA `aria-disabled`), in the web mirror and the native tree; the native tree also reports busy.
* Focus groups take `El::focus_axis` and `El::focus_wrap` in place of their role's arrows and wrapping, and Tab returns to the item in a group that last had focus.
* `El::has_popup` names the kind of popup a control opens (ARIA `aria-haspopup`), on the web mirror.
* `El::press_travel` sinks a control while it is pressed and keeps its outer shadows in place, so a key comes down onto its lip.
* The web host tracks effect marks when a compositor is attached, as the native window does, so compositor effect slots draw in the browser.
* `El::caret_block` draws a text field's caret as a block the width of the character after it (a space wide at a line end), as CSS `caret-shape: block`; that character takes the given ink, and a selection or composition hides the block (`tests/caret_block.rs`).
* `El::on_press_down` runs as a primary pointer press lands on an enabled element, before the release activates it, so a key can sound its down-stroke (`tests/press_down.rs`).
* On the web the hidden text field reports the keys it types and every key it releases as physical input, so a focused field's `El::on_physical_input` hears typing as it does on a native host; the password field reports none.
* `El::focus_ring_default` sets the default keyboard focus ring for every control inside an element that sets none of its own, so a theme draws its ring once at its root (`tests/focus_ring_default.rs`). Painting checks an element's own ring before walking up to the focus, so elements without one skip the walk.
* `El::scroll_stop` makes a scroll view a Tab stop while its content overflows and nothing inside it can take focus; focused, the arrow keys scroll it a 40 point line and the page keys a page, and the web mirror shows it as a named region that takes native focus. The web host reconciles its native controls only when the mirror changes, not on every frame.
* Web mirror elements carry stable ids hashed from their runtime paths, and `El::controls` and `El::active_descendant` reach the mirror as `aria-controls` and `aria-activedescendant` by those ids.
* A mirror sync sends an item that did not change as its place in the last sync (`{"same":n}`); the bridge keeps that element and only checks its order, and asks for a full sync (`a11y_resync`) if it ever lacks one.
* The web mirror shows a `role("listbox")` element as a listbox container around its options, and an element with a control role (option, tab, radio and the like) stays that control without a press handler, so a disabled option still reads as one.
* The workspace's dev and test profiles keep line tables only, not full debug info: a test build of the Rune crate links faster and its target takes 13 GiB instead of 37 GiB.
* `El::layer` makes a non-modal layer (popover, tooltip, list popup): the topmost one, above any modal, hears Escape, a press outside it and its anchor, focus moving out and the window losing focus, each with a `Dismiss` reason, and the app decides (`runtime/dismiss_layer_tests.rs`). A press counts as inside by the point, so text and padding inside a layer are inside.
* A press on a layer's anchor reaches the layer as `Dismiss::AnchorPress` beside the anchor's own handler, so a tooltip closes when its trigger is pressed.
* `El::on_focus_change` tells an element it gained or lost focus and whether the keyboard moved it, so a tooltip opens on keyboard focus and not on a click.
* Finding a node by its path, which each press, hover, dismissal and focus change does, uses the path index and falls back to a scan only for an exiting or unmounted node.
* The web mirror shows `role("tablist")` and a named `role("tabpanel")` as containers. `El::focus_manual` on a focus group lets the arrows move over its radio and tab items without pressing them, for tabs with manual activation.
* The web mirror shows `role("toolbar")` as a named container, and `role("progressbar")` and `role("meter")` as one element each with `aria-valuenow`, `aria-valuemin`, `aria-valuemax` and `aria-valuetext` (a progress bar with no value is indeterminate). The mirror's JSON is written straight into one buffer instead of through a `serde_json::Value` tree.
* The items an arrow key moves between come from the group's own subtree, with the membership test before the reachability one, instead of every node on the page.
* Web input stops untrusted (`isTrusted` false) input types in the window capture phase, before the page's listeners and winit's. Mirror presses and focus name the browser's trust and the mirror revision they were rendered from, and the runtime refuses any other (`runtime/input/provenance.rs`).

The MIT license and bundled Inter and IBM Plex Mono font licenses remain beside their source assets.

Line endings and trailing blank lines are normalized without changing source tokens or license wording.
