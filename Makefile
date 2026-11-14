.PHONY: rust check check-rust check-rune check-codex check-harness check-claude check-pi check-opencode check-browser-tools check-sse check-responses check-google check-messages check-chat check-credentials check-conversation check-panel panel-wasm rune-host check-rune-app check-companion companion

# Every Cargo root in the repository, in the order the check-* targets use.
RUST_CRATES := metldr-sse metldr-responses metldr-google-interactions metldr-messages \
  metldr-chat-completions metldr-native-credentials metldr-conversation \
  metldr-harness metldr-pi-session metldr-codex-session metldr-opencode-session metldr-claude-session \
  metldr-browser-tools metldr-answer-codec metldr-panel metldr-companion \
  metldr-core metldr-history-pages metldr-welcome metldr-rune-host

# Fast pre-gate: type-check every Cargo root without linking or running tests.
# Run it before the check-* targets when a change touches a shared crate.
check:
	@set -e; for crate in $(RUST_CRATES); do \
	  echo "check $$crate"; \
	  cargo check --locked --manifest-path crates/$$crate/Cargo.toml; \
	done
	cargo check --locked --manifest-path prototypes/rune-frontend/Cargo.toml --lib

rust:
	npm run build:rust

check-rust:
	node scripts/build-rust.mjs --check

check-rune:
	cargo test --locked --manifest-path prototypes/rune-frontend/Cargo.toml --lib

check-codex:
	cargo test --locked --manifest-path crates/metldr-codex-session/Cargo.toml

check-harness:
	cargo test --locked --manifest-path crates/metldr-harness/Cargo.toml

check-claude:
	cargo test --locked --manifest-path crates/metldr-claude-session/Cargo.toml

check-pi:
	cargo test --locked --manifest-path crates/metldr-pi-session/Cargo.toml

check-opencode:
	cargo test --locked --manifest-path crates/metldr-opencode-session/Cargo.toml

check-browser-tools:
	cargo test --locked --manifest-path crates/metldr-browser-tools/Cargo.toml

check-sse:
	cargo test --locked --manifest-path crates/metldr-sse/Cargo.toml

check-responses:
	cargo test --locked --manifest-path crates/metldr-responses/Cargo.toml

check-google:
	cargo test --locked --manifest-path crates/metldr-google-interactions/Cargo.toml

check-messages:
	cargo test --locked --manifest-path crates/metldr-messages/Cargo.toml

check-chat:
	cargo test --locked --manifest-path crates/metldr-chat-completions/Cargo.toml

check-credentials:
	cargo test --locked --manifest-path crates/metldr-native-credentials/Cargo.toml

check-conversation:
	cargo test --locked --manifest-path crates/metldr-conversation/Cargo.toml

WASM_BINDGEN ?= wasm-bindgen
# The directory scripts/build-rust.mjs builds the panel wasm into.
PANEL_TARGET_DIR ?= target/wasm/metldr-panel

check-panel:
	cargo test --locked --manifest-path crates/metldr-answer-codec/Cargo.toml
	cargo test --locked --manifest-path crates/metldr-panel/Cargo.toml

panel-wasm:
	cargo build --locked --release --manifest-path crates/metldr-panel/Cargo.toml --target wasm32-unknown-unknown --target-dir "$(PANEL_TARGET_DIR)"
	mkdir -p crates/metldr-panel/pkg
	$(WASM_BINDGEN) --target web --out-dir crates/metldr-panel/pkg "$(PANEL_TARGET_DIR)/wasm32-unknown-unknown/release/metldr_panel.wasm"

RUNE_HOST_TARGET_DIR ?= target/wasm/metldr-rune-host
RUNE_HOST_OUT ?= crates/metldr-rune-host/pkg

# Internal Rune host route: native lifecycle and font tests (no renderer), then the decoded fonts.
check-rune-app:
	cargo test --locked --manifest-path crates/metldr-rune-host/Cargo.toml
	node scripts/check-rune-fonts.mjs

# Rune host as its own wasm module, copied with its fonts into public/rune for the internal route
# only. Shipping builds never run this target, so their packages gain nothing.
rune-host:
	cargo build --locked --release --features renderer --manifest-path crates/metldr-rune-host/Cargo.toml --target wasm32-unknown-unknown --target-dir "$(RUNE_HOST_TARGET_DIR)"
	mkdir -p "$(RUNE_HOST_OUT)" public/rune
	$(WASM_BINDGEN) --target web --out-name metldr_panel --out-dir "$(RUNE_HOST_OUT)" "$(RUNE_HOST_TARGET_DIR)/wasm32-unknown-unknown/release/metldr_rune_host.wasm"
	cp "$(RUNE_HOST_OUT)/metldr_panel.js" "$(RUNE_HOST_OUT)/metldr_panel_bg.wasm" public/rune/
	rm -rf public/rune/snippets && cp -R "$(RUNE_HOST_OUT)/snippets" public/rune/
	mkdir -p public/rune/fonts && cp crates/metldr-rune-host/assets/fonts/* public/rune/fonts/
	node scripts/check-rune-fonts.mjs

check-companion:
	cargo test --locked --manifest-path crates/metldr-companion/Cargo.toml

companion:
	cargo build --locked --release --manifest-path crates/metldr-companion/Cargo.toml --bin metldr-companion
