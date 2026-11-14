# Claude Code session

This crate runs the installed Claude Code CLI as a MetLDR agent. It starts `claude -p` with stream-json input and output, gives it only the MCP servers MetLDR names (`--strict-mcp-config`), and routes every permission prompt back to MetLDR over stdio (`--permission-prompt-tool stdio`), so the user approves each tool use in MetLDR. New sessions get a MetLDR-chosen UUID and follow-ups resume it.

The crate never installs, signs in or pays for anything. It uses the user's own Claude Code login, or an endpoint given through `Launch::env`. Lines are bounded at 4 MiB, closing ends input and kills the process after 5 seconds, and dropping a session kills it.

The check in `tests/claude.rs` drives the installed binary against a loopback Messages fixture, so it makes no network or paid request. It is skipped when Claude Code is not installed.
