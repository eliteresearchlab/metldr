# Third party notices

## t3code

The harness registry and the context handoff in `crates/metldr-harness` are informed by [t3code](https://github.com/pingdotgg/t3code) by T3 Tools Inc. (pingdotgg): its provider adapter registry with declared capabilities and its context handoff service, which carries either the activity since the target provider last saw a thread or a full summary, newest first under a budget, with coverage and omission recorded. Reference revision: [4115648](https://github.com/pingdotgg/t3code/tree/41156481b1c2a9ea8f8b97680b76f17685387d38), files `apps/server/src/orchestration-v2/ProviderAdapterRegistry.ts` and `apps/server/src/orchestration-v2/ContextHandoffService.ts`.

| Area | metldr files | Use | Behaviour change | Checks |
| --- | --- | --- | --- | --- |
| One registry of harnesses with declared capabilities | `crates/metldr-harness/src/lib.rs` | Reference only; written independently in Rust | None | `cargo test --manifest-path crates/metldr-harness/Cargo.toml` |
| Delta or full context handoff, newest first under a cap | `crates/metldr-harness/src/handoff.rs` | Reference only; written independently in Rust | None until the companion uses it | the same suite |

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
