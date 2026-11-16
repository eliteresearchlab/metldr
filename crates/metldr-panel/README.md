# Rust panel module

The crate builds two WebAssembly modules from one source:

- The service worker module (default features) owns the native connection to `io.metldr.first_flow` using wire version 22. `extension/background.js` passes its named connected ports to `accept_panel` and calls `stop_all_runs`. It validates each sender, accepts only `chat.html` within the same extension, and checks every native frame before it forwards the frame to a panel.
- The chat module (`rune-ui` feature) draws the chat at `chat.html` with Rune. `start_rune_chat` mounts it with the page capture function and the fonts, and `stop_rune_chat` unmounts it. Alt+Shift+M and the welcome page open it in the side panel.

Run `make check-panel` to test the crate with the chat module included. Run `make rust` to build both modules and record the matching chat receipt. Both commands use the pinned Rust toolchain. The build also needs the `wasm32-unknown-unknown` target and `wasm-bindgen-cli` version 0.2.128, matching the crate dependency. Set `CARGO_TARGET_DIR` to choose the Cargo build directory.

`ui_bridge/run.rs` owns the shared run driver and delivers typed `Event` values to a Rust callback. `Core::stream` uses that callback for saved run updates. Admission, Stop, session checks and uncertain receipts stay in the Rust owner. Rust consumers call `Core::run` and `Core::capture` in the input handler to reserve work before polling its future. Dropping a run before dispatch releases its reservation. Dropping it after dispatch leaves its result uncertain. Capture sends the Chrome permission request before the input handler yields.

The chat keeps one ordered message list per conversation and replaces activity with each snapshot, so corrections cannot leave stale activity. New chat creates an empty conversation. Saved chats lists saved conversations with search over loaded pages. Selecting one restores its saved page and results and keeps the composer draft. Active work blocks switching. A lost confirmation keeps the exact change request for an explicit retry and blocks new work until its outcome is known.

The model menu lists configured providers, browser task entries and installed agents. The sources menu adds the current page. The Codex composer offers Default and Plan. The chat answers nonsecret native questions, required string MCP forms and separate MCP tool approvals. Stop withdraws pending input. Other form types and secret questions are not supported.

Native work and saved run ownership stay with the paired companion. Provider credentials belong to the companion, not to page scripts. The Chrome background validates browser context and grants before browser commands.

Run `npm run build` at the repository root to build the extension into `dist`, then load that directory at `chrome://extensions`. Press Alt+Shift+M to open the chat, or Ctrl+Shift+Y to open the page tools. Change shortcuts at `chrome://extensions/shortcuts`. Native work requires the [native companion](../metldr-companion/README.md), registered for the loaded extension ID. Building does not install or register it.

Gmail, PDF, dictionary, summaries and local inference stay in the page tools until their Rust ports ship. Native Windows and macOS verification is not complete.
