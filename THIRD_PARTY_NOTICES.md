# Third party notices

## t3code

The harness registry and the context handoff in `crates/metldr-harness` are informed by [t3code](https://github.com/pingdotgg/t3code) by T3 Tools Inc. (pingdotgg): its provider adapter registry with declared capabilities and its context handoff service, which carries either the activity since the target provider last saw a thread or a full summary, newest first under a budget, with coverage and omission recorded. Reference revision: [4115648](https://github.com/pingdotgg/t3code/tree/41156481b1c2a9ea8f8b97680b76f17685387d38), files `apps/server/src/orchestration-v2/ProviderAdapterRegistry.ts` and `apps/server/src/orchestration-v2/ContextHandoffService.ts`.

| Area | metldr files | Use | Behaviour change | Checks |
| --- | --- | --- | --- | --- |
| One registry of harnesses with declared capabilities | `crates/metldr-harness/src/lib.rs` | Reference only; written independently in Rust | None | `cargo test --manifest-path crates/metldr-harness/Cargo.toml` |
| Delta or full context handoff, newest first under a cap | `crates/metldr-harness/src/handoff.rs` | Reference only; written independently in Rust | None | the same suite |
| One native session per harness in each chat, a delta since the harness last took part or a full summary for its first run, and a 16k token default budget, from `apps/server/src/orchestration-v2/Orchestrator.ts`, `ProviderTurnStartService.ts` and `packages/provider-core/src/server/handoffBudget.ts` at revision [a61291e](https://github.com/pingdotgg/t3code/tree/a61291e3e2) | `crates/metldr-companion/src/native_start.rs`, `crates/metldr-companion/src/history/edit/handoff.rs`, `crates/metldr-panel/src/composer.rs` | Reference only; written independently in Rust | Picking an agent continues the chat's session for it, and each run opens with what the chat did elsewhere | `cargo test` for both crates and the private Claude Code flow |

```text
MIT License

Copyright (c) 2026 T3 Tools Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Claude Code

`crates/metldr-claude-session` runs the user's installed [Claude Code](https://github.com/anthropics/claude-code) CLI by Anthropic as a separate program. It uses the CLI's documented `--print` stream-json input and output and its `--permission-prompt-tool stdio` control messages, the same interface the Claude Agent SDK uses. No Claude Code or Claude Agent SDK source is copied into metldr, and Claude Code stays under its own license and terms. Reference behaviour: Claude Code 2.1.323.

| Area | metldr files | Use | Behaviour change | Checks |
| --- | --- | --- | --- | --- |
| Stream-json session, resume and stdio permission messages | `crates/metldr-claude-session/src/lib.rs` | Interoperability with the installed CLI; implemented independently in Rust | None | `cargo test --manifest-path crates/metldr-claude-session/Cargo.toml` |

## browse

Browser integration work is informed by [browse](https://github.com/anomalyco/browse). MCP tool discovery follows its use of standard safety annotations, implemented independently in Rust. Reference revision: [ebb6ded](https://github.com/anomalyco/browse/tree/ebb6dedc4cb154c0279e0bb13699f21b507249d1), release 0.0.16.

The upstream license notice is preserved below. Attribution does not mean every Browse capability is implemented in metldr.

### Reference record

Every area below was read at the reference revision above. No Browse source file is copied or adapted into metldr.

| Area | metldr files | Use | Behaviour change | Checks |
| --- | --- | --- | --- | --- |
| MCP tool discovery and safety annotations | `crates/metldr-browser-tools/src/lib.rs`, `crates/metldr-companion/src/browser_mcp.rs` | Reference only; the Rust implementation was written independently | None | `cargo test --manifest-path crates/metldr-browser-tools/Cargo.toml`, `cargo test --manifest-path crates/metldr-companion/Cargo.toml` |
| Tool names, descriptions and input schemas | `crates/metldr-browser-tools/src/lib.rs` | Reference only | None | the same two suites |
| Approval before dispatch | `crates/metldr-companion/src/browser_action.rs`, `crates/metldr-companion/src/browser_owner.rs` | Reference only | None | the same two suites |
| This notice and the README acknowledgment | `THIRD_PARTY_NOTICES.md`, `README.md` | Reference only; the MIT text is reproduced verbatim | None | `diff` of the notice block against the upstream `LICENSE` |

```text
MIT License

Copyright (c) 2025 opencode

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
