# Browse

Source: https://github.com/anomalyco/browse, Rust workspace under `rust/`, commit 6128d7b, MIT (LICENSE).

Vendored: `crates/browse-core` and `crates/browse-page` (sources and the page scripts in `js/`).
Each other Browse crate comes in with the change that first uses it.

Changes from upstream:
- `browse-page/src/page.rs`: Browse's isolated world is created without universal access, which
  the metldr extension refuses.
- `browse-page/src/scripts.rs` and `lib.rs`: `injected_source()` and `helpers()` are public, so the
  metldr extension can ship both scripts as files and the companion sends a short marker instead.
- `browse-page/js/helpers.js`: a control with no name, or whose role and name another control of the
  same capture shares, gets a `hint=` in its details (its icon, image file, a stable attribute and its
  row's text), so the model can tell them apart; refs and names are unchanged.
- `Cargo.toml`: tokio builds only the features the two crates use (rt, sync, time, macros, fs)
  instead of `full`.
- Left out: the Chromium integration tests and the `js/generate.sh` script that rebuilds the page
  scripts from Playwright.
