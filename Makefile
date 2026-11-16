.PHONY: rust check check-rust check-ui check-codex check-harness check-claude check-pi check-opencode check-browser-tools check-sse check-responses check-google check-messages check-chat check-credentials check-conversation check-panel check-companion companion dev-install

# Every Cargo root in the repository, in the order the check-* targets use.
RUST_CRATES := metldr-sse metldr-responses metldr-google-interactions metldr-messages \
  metldr-chat-completions metldr-native-credentials metldr-conversation \
  metldr-harness metldr-pi-session metldr-codex-session metldr-opencode-session metldr-claude-session \
  metldr-browser-tools metldr-answer-codec metldr-ui metldr-panel metldr-companion \
  metldr-core metldr-history-pages metldr-welcome

# Fast pre-gate: type-check every Cargo root without linking or running tests.
# Run it before the check-* targets when a change touches a shared crate.
check:
	@set -e; for crate in $(RUST_CRATES); do \
	  echo "check $$crate"; \
	  cargo check --locked --manifest-path crates/$$crate/Cargo.toml; \
	done

rust:
	npm run build:rust

check-rust:
	node scripts/build-rust.mjs --check

# The control behaviour layer: each primitive with its ported cases.
check-ui:
	cargo test --locked --manifest-path crates/metldr-ui/Cargo.toml

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

check-panel:
	cargo test --locked --manifest-path crates/metldr-answer-codec/Cargo.toml
	cargo test --locked --manifest-path crates/metldr-panel/Cargo.toml --features rune-ui

check-companion:
	cargo test --locked --manifest-path crates/metldr-companion/Cargo.toml

companion:
	cargo build --locked --release --manifest-path crates/metldr-companion/Cargo.toml --bin metldr-companion

# Builds the extension and the companion, then registers the companion with every installed
# Chromium browser for the unpacked extension in dist. Load dist in chrome://extensions once.
dev-install: companion
	npm run build
	"$${CARGO_TARGET_DIR:-target}/release/metldr-companion" install dist
