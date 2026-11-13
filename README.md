<p align="center">
  <img src="public/assets/logo/metldr-mark.svg" width="96" height="96" alt="metldr">
</p>
<h1 align="center">metldr</h1>
<p align="center">
  <a href="https://chromewebstore.google.com/detail/kbfdmfgmmibkdnbfdaeganjckfgbfjlh"><img src="https://img.shields.io/badge/Chrome-extension-ff8338?style=flat-square" alt="Chrome extension"></a>
  <a href="docs/features.md"><img src="https://img.shields.io/badge/AI-your%20key%20or%20local-b4befe?style=flat-square" alt="Your API key or local AI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/App-free-75659a?style=flat-square" alt="Free app"></a>
</p>

metldr is a free AI assistant for Chrome that reads, researches and does real browser work for you, with your approval, using the AI you choose: your API keys, the coding agents you already use, or fully local models.

> [!WARNING]
> **Work in progress: new version.** metldr is being rebuilt in Rust with a new Rune-based interface, a native companion, and expanded browser control. Features, interfaces and data formats may change, and some parts are not finished yet. The current extension remains the reference until each replacement reaches parity.

* Summarize pages, PDFs and Gmail, look up words and search pages.
* Run approved tasks, restore chats, and choose APIs, installed agents or local AI.

## Planned features

Some features are available today, and the full list is planned.

### Browser work and conversations

* Work in your allowed signed in tabs: read pages, inspect controls and layouts, and capture screenshots.
* Navigate pages, history and tabs, track new tabs and popups, and group tabs by task or conversation.
* Click, double click, hover, drag, drop, scroll and select, with context menus, keyboard shortcuts and the clipboard.
* Fill forms across pages, upload files, edit rich text, and run approved page scripts or WebMCP page tools.
* Review plans, set scoped approval rules and automatic approvals for each session, and see when browser control is active.
* Stop, pause and resume tasks, and verify results, redirects and dead links.
* Keep and restore chats, attachments, live activity and interrupted results, and continue work across conversations.
* Send selected content and website prompts to the sidebar, and get contextual suggestions while browsing.
* Organize projects, goals and working directories.

### Tasks you can complete

* Summarize pages, emails, PDFs and video transcripts, explain code, contracts and research, and create study notes and flashcards.
* Triage inboxes, flag urgent messages and draft follow ups for unanswered threads.
* Research companies, suppliers and products, and compare prices, quotes and sources with citations.
* Find, qualify and track leads, and draft outreach and connection requests.
* Complete applications and surveys from documents, and extract invoices and reconcile payments.
* Screen applicants, update recruitment systems, and plan agendas, appointments and invitations.
* Build recurring reports across dashboards and query BigQuery and other connected data warehouses.
* Track orders, bookings and price changes.

### Workflows, Skills and memory

* Teach workflows through instructions or demonstrations, and save them as versioned templates with inputs, outputs and approval rules.
* Run workflows and Skills manually, through slash commands, on schedules with timezones, exceptions and overlap rules, or when a page title changes.
* Pause, resume or remove scheduled work, recover missed and interrupted runs, and keep tasks running with the panel closed.
* Start, track and manage tasks, exceptions and run histories from a dashboard.
* Create, share and browse built in, site and plugin Skills, and import them from URLs or ZIP files.
* Manage context files and editable memory: inspect, edit, disable or delete it, and scope corrections to projects or workflows.
* Optionally sync memory, projects, workflows and preferences, and manage team workspaces, member access and shared or individual budgets.
* Preview, edit, undo, list and remove site scripts, and run them automatically on matching pages.
* Reuse learned site tools and cached prompts, add custom tools and dashboards to pages, and apply reading preferences and page shortcuts.
* Receive completion alerts and approval requests.

### Files and interactive answers

* Pin local files and folders to chats, and browse, read and write within approved working directories.
* Edit Excel, CSV, browser spreadsheets, Google Sheets cell blocks, Word and browser documents while preserving formulas and unrelated content.
* Search chats, pages, files and documents, and read large pages and files in chunks.
* Use PDF text, OCR and dictionary lookup, and preview, zoom and navigate PDFs, images and generated files.
* Attach files, clipboard images and selected text, and copy images between allowed websites.
* Get streamed answers with tables, charts, diagrams, calculators, maps, interactive explanations and local controls, and save them as reusable artifacts.
* Keep screenshot history during long tasks, record approved task videos for replay, and export results with screenshots or print them to PDF.
* Download all task outputs together, or folders as ZIP files.

### Email, voice and remote control

* Draft, reply, forward and send Gmail, and verify sent mail and approval outcomes.
* Read, search and manage WhatsApp chats: send approved messages, files and reactions, manage members, roles and invite links, and control read status and history.
* Dictate prompts, and review, edit or cancel voice recordings and transcripts.
* Send tasks, voice notes, photos and files through Telegram or WhatsApp, and receive results, screenshots and status updates.
* Approve or cancel remote actions, pair, switch and revoke devices or profiles, and let authorized MCP clients control tasks.

### Model choice and reliability

* Choose popular APIs, custom endpoints, local and browser models, or approved local and network model servers, using API keys, supported account sign ins, or no key where supported.
* Run Codex, OpenCode, Pi and Claude Code with their accounts, tools, permissions and sessions, and connect remote MCP servers and tools.
* Switch models with visible capability limits, set timeouts and connection settings, and choose separate speech and task providers.
* Inspect token usage and estimated costs, manage context with /compact and /usage, and select browser profiles through a CLI.
* Recover connections without silent provider switching, and review uncertain writes before any repeat attempt.
* Keep site, tab and file permissions explicit, isolate generated answers from browser authority, and keep local inference local unless you approve.
* See model data destinations and access limits, with bounded storage, memory and background work.
* Export, delete and recover saved data, preserved through upgrades and interrupted work.
* Use multilingual and accessible controls with themes, text size and sound preferences.
* Use Chrome, Edge, Brave and Chromebooks with Linux, Windows and macOS companions, and every feature without an app subscription.

See the [feature guide](docs/features.md) for current availability, planned details and compatibility limits.

[Install from the Chrome Web Store](https://chromewebstore.google.com/detail/kbfdmfgmmibkdnbfdaeganjckfgbfjlh).
The app is free. Your model provider may charge.

The browser agent is in development. Its current build needs Linux and [companion setup](crates/metldr-companion/README.md).

Browser integration work draws on [browse](https://github.com/anomalyco/browse). See [third party notices](THIRD_PARTY_NOTICES.md).

[Build and setup](docs/development.md) | [Privacy](PRIVACY.md)
