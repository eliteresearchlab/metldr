# Harness registry

The one list of agent harnesses MetLDR drives: Pi, Codex, OpenCode and Claude Code. Each entry holds the stable wire key, the display label, the executable names to look for and the capabilities MetLDR may offer for it. The companion and the panel read names and capabilities from here instead of repeating them, so adding a harness is one entry here and one adapter in the companion.

`detect` finds each harness executable on `PATH` and in common user install folders, and where Claude Desktop, the Codex app, OpenCode and Cursor keep their MCP configuration, without running or changing anything. Claude Desktop has no Linux build and ChatGPT adds MCP servers in its own settings, so neither has a local file to find there.

`handoff::build` prepares the context a conversation carries when it moves to another harness or model: the newest items first under a character cap, page and tool text labelled untrusted, the covered runs and the count left out, and an instruction never to repeat earlier browser actions. It can send only what happened since the target last took part, or the whole conversation.
