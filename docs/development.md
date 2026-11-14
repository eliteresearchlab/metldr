# Development

## Install and run

Install Node.js, the mise managed Portless command, and the pinned Rust toolchain. The repository uses Rust 1.99.1 and wasm-bindgen-cli 0.2.128. Add the WebAssembly target and install the matching binding generator, then install JavaScript packages:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
npm ci
npm run install:chat
```

The chat interface has its own pinned dependencies. Run `npm run install:chat` again when its lock file changes. `npm run check:chat` uses its Go based TypeScript compiler. The current page tools keep their existing compiler during migration.

Run `npm run dev` to build the Rust modules and start the named metldr preview through Portless. Open the URL printed by Portless. Add `/welcome.html`, `/chat.html`, `/panel.html`, or `/index.html` to view the welcome page, chat interface, earlier browser agent, or current page tools. A web preview does not have Chrome extension access.

Run `npm run build` to create the extension in `dist`. In Chrome, open `chrome://extensions`, enable Developer mode, choose **Load unpacked**, and select `dist`. After another build, reload the extension from the same page.

The toolbar opens the current page tools. Press Alt+Shift+M to open the browser agent. Press Ctrl+Shift+Y to open the page tools. Change either shortcut at `chrome://extensions/shortcuts` if Chrome reports a conflict.

## Native companion

Provider and installed agent tasks need the native companion. Load `dist` unpacked once, then run `make dev-install` after each change. It builds the extension and the companion, copies the companion to `~/.local/share/metldr`, and registers it with Chrome, Chromium, Brave and Edge for the unpacked extension. If `chrome://extensions` shows a different ID, run `metldr-companion install <id>`. `metldr-companion uninstall` removes the registration and keeps saved history. Linux is the verified native platform.

## Local AI setup

Open Help on the welcome page for Chrome AI and Ollama setup. Its Ollama guide includes the extension origin and platform commands. Check reads availability without sending a prompt.

## Build configuration

Cargo uses one target directory for the whole repository, set in `.cargo/config.toml`. Every crate under `crates/` and `prototypes/` is a separate Cargo root, so without that file the shared dependency stack is compiled again for each crate.

`scripts/build-rust.mjs` builds the four wasm crates through a pool, two at a time by default. Set `METLDR_BUILD_JOBS` to change the limit. Each build gets its own directory under `target/wasm`, so cargo does not serialise them on one target lock.

`sccache` is optional. The repository does not configure it, because a contributor without it would fail the build. Enable it in your shell when you want it:

```sh
export RUSTC_WRAPPER=sccache
export CARGO_INCREMENTAL=0
```

`sccache` shares its compile cache across builds. It does not cache incremental compilation units, so set `CARGO_INCREMENTAL=0` when using it.

`make check` type-checks the Cargo roots under `crates/` and the Rune frontend prototype with `cargo check`, without linking or running tests. Run it before the `check-*` targets when a change touches a shared crate.

## Checks

Run `npm run check:chat` for the chat interface and `npm run build:chat` after generating its Rust bindings.

Run `make check-rust`, `make check-panel` or `make check-companion` for the affected code. Run `make rust` after Rust changes during a preview session. The build also refreshes the chat's Rust bindings. See the [panel guide](../crates/metldr-panel/README.md) for browser setup and module details.

The background scheduler has three Rust driven checks against its actual TypeScript service. They need Node.js with native TypeScript support and the `timeout` command. Run from the repository root:

```sh
mkdir -p .tmp/check-concurrency
rustc --edition 2024 --test scripts/check-concurrency.rs -o .tmp/check-concurrency/check
.tmp/check-concurrency/check --test-threads=1
rm .tmp/check-concurrency/check
```
