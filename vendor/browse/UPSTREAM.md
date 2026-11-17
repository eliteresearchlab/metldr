# Browse

Source: https://github.com/anomalyco/browse, Rust workspace under `rust/`, commit 6128d7b, MIT (LICENSE).

Vendored: `crates/browse-core` and `crates/browse-page` (sources and the page scripts in `js/`).
Each other Browse crate comes in with the change that first uses it.

Changes from upstream:
- `browse-page/src/page.rs`: Browse's isolated world is created without universal access, which
  the metldr extension refuses.
- `browse-page/src/scripts.rs` and `lib.rs`: `injected_source()` and `helpers()` are public, so the
  metldr extension can ship both scripts as files and the companion sends a short marker instead.
- `Cargo.toml`: tokio builds only the features the two crates use (rt, sync, time, macros, fs)
  instead of `full`.
- Left out: the Chromium integration tests and the `js/generate.sh` script that rebuilds the page
  scripts from Playwright.
