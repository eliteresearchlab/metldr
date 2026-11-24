# Privacy

Updated 19 November 2026.

## Where content goes

Each route sends content only to the destination listed for it.

| Route | Destination |
| --- | --- |
| Chrome built-in AI and the bundled local models | Your computer |
| Local model servers | The server on your computer: Ollama at 127.0.0.1:11434, LM Studio at 127.0.0.1:1234, llama.cpp at 127.0.0.1:8080, or the local address you set. Ollama can also offer cloud models, so check the model you select. |
| Your own API key in the extension | The service the key belongs to: OpenAI (api.openai.com), Anthropic (api.anthropic.com), Google (generativelanguage.googleapis.com), OpenRouter (openrouter.ai), Groq (api.groq.com), DeepSeek (api.deepseek.com), Mistral (api.mistral.ai), Together (api.together.ai), xAI (api.x.ai), or the OpenAI compatible endpoint you set. It receives your question, the page content you ask about and the conversation so far. |
| Provider connections in the companion (Messages, Responses, Google Interactions, Chat Completions) | The endpoint you configure. It receives task instructions, page content, browser observations and relevant prior answers. |
| A decision connection in the companion (TypeSafe) | When you set one up, each step where the browser agent looks for a control sends the step's goal, its last three actions and the names of up to 40 shortlisted controls on the page to the endpoint you set (api.typesafe.ai by default), which orders them. Without one, the ordering runs on your computer. |
| Installed agents (Codex, Claude Code, OpenCode, Pi) | The agent on your computer, which sends what it needs to the account and model you configured for it |
| Reading a page | When the page you ask about links to its own markdown copy on the same site over https, the extension fetches that copy from the site, without cookies, instead of reading the page's layout. No address on your computer or local network is fetched. |
| Reading a PDF | On a tab that shows a PDF, the PDF tools and the chat fetch that PDF from its address, with your cookies for that site as the browser's viewer does, and read it inside the extension. A PDF on your computer is read only from a file you choose. |
| Dictionary lookups | The selected word goes to api.dictionaryapi.dev. Dictionary data files come from media.githubusercontent.com. |
| Search and language models | Once, on first use, the extension downloads its search model (8 MB, WhiteHades/blink-8M-int8) and its language detector (5 MB, WhiteHades/py3langid-compact) from huggingface.co, each at a fixed revision checked by its SHA-256. Nothing about you or your pages is sent; searches and detection run on your device. |
| Gmail features | The InboxSDK library runs on mail.google.com. It sends its own error reports and usage events to inboxsdk.com. |

Installed agents' tools and metldr's browser tools can provide additional access, subject to the agent's permissions and metldr's browser approvals. Your provider's and agent's policies apply to their processing. Downloads you start contact the services that supply the requested files.

## What stays saved

The extension saves settings, cached summaries and page context in your browser. The companion saves conversations, results and recovery records on your computer. Installed agents can keep their own sessions and files.

The extension also saves usage records in your browser, including page addresses, domains, session activity, model use and feature events. Automatic deletion after a set period is not currently enabled. Remove the extension's browser data to delete those records.

API keys you add in the extension are encrypted with a key that cannot leave this browser profile, and only the encrypted form is saved, locally and never synced. Keys you add in the companion stay in its credential store, outside page scripts. Installed agents keep their own authentication.

Clearing the extension's cache or browser data does not remove companion records or agent files. Remove those separately using their storage locations and tools. The companion keeps its data in `~/.local/share/metldr`, or in `$XDG_DATA_HOME/metldr` when that is set.

## Browser access

Page permissions let metldr add its controls and read content for its features. The browser agent asks for approval before supported page changes. Installed agents also have their own permission rules.

Review your selected connection and permissions before sharing private content. For questions, use the support contact on the [Chrome Web Store listing](https://chromewebstore.google.com/detail/kbfdmfgmmibkdnbfdaeganjckfgbfjlh).
