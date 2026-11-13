# Privacy

Updated 13 November 2026.

## Where content goes

Each route sends content only to the destination listed for it.

| Route | Destination |
| --- | --- |
| Chrome built-in AI and the bundled local models | Your computer |
| Ollama | The Ollama server on your computer, at 127.0.0.1:11434. Ollama can also offer cloud models, so check the model you select. |
| Provider connections (Messages, Responses, Google Interactions, Chat Completions) | The endpoint you configure. It receives task instructions, page content, browser observations and relevant prior answers. |
| Installed agents (Codex, Pi, OpenCode) | The agent on your computer, which sends what it needs to the account and model you configured for it |
| Dictionary lookups | The selected word goes to api.dictionaryapi.dev. Dictionary data files come from media.githubusercontent.com. |
| Gmail features | The InboxSDK library runs on mail.google.com. It sends its own error reports and usage events to inboxsdk.com. |

Installed agents' tools and metldr's browser tools can provide additional access, subject to the agent's permissions and metldr's browser approvals. Your provider's and agent's policies apply to their processing. Downloads you start contact the services that supply the requested files.

## What stays saved

The extension saves settings, cached summaries and page context in your browser. The companion saves conversations, results and recovery records on your computer. Installed agents can keep their own sessions and files.

The extension also saves usage records in your browser, including page addresses, domains, session activity, model use and feature events. Automatic deletion after a set period is not currently enabled. Remove the extension's browser data to delete those records.

Provider keys are managed by the companion's credential store, outside page scripts. Installed agents keep their own authentication.

Clearing the extension's cache or browser data does not remove companion records or agent files. Remove those separately using their storage locations and tools. See [companion setup](crates/metldr-companion/README.md) for its data location.

## Browser access

Page permissions let metldr add its controls and read content for its features. The browser agent asks for approval before supported page changes. Installed agents also have their own permission rules.

Review your selected connection and permissions before sharing private content. For questions, use the support contact on the [Chrome Web Store listing](https://chromewebstore.google.com/detail/kbfdmfgmmibkdnbfdaeganjckfgbfjlh).
