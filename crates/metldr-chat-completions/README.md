# metldr Chat Completions client

This Rust client streams plain text and retains private assistant history for browser function calls. The application owns credentials, browser permissions, approvals and actual tool execution.

```no_run
use metldr_chat_completions::{Client, Prompt};
use std::sync::atomic::AtomicBool;
# fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
let stop = AtomicBool::new(false);
let client = Client::new(
    "https://api.example.test/v1/chat/completions",
    "configured-model",
    Some("example-token"),
    None,
    metldr_chat_completions::DEFAULT_TIMEOUT_SECS,
)?;
let mut conversation = client.start(Prompt {
    instruction: "Read the current page.",
    title: "Example",
    url: "https://example.test/",
    text: "Captured page text",
})?;
# Ok(())
# }
```

For a text response, use `Client::text` instead of starting a browser conversation.
The caller supplies system instructions and input separately.

```no_run
use metldr_chat_completions::{Client, Result, TextEvent};
use std::sync::atomic::AtomicBool;

# async fn example() -> Result<()> {
let client = Client::new("https://api.example.test/v1/chat/completions", "configured-model",
    Some(&std::env::var("MODEL_API_KEY")?), None, metldr_chat_completions::DEFAULT_TIMEOUT_SECS)?;
client.text("Answer using the supplied source.", "The report lists 17 units.",
    &AtomicBool::new(false), |event| {
        if let TextEvent::Delta(text) = event { print!("{text}"); }
        Ok(())
    }).await?;
# Ok(())
# }
```

The text request includes no tool definitions or browser policy. It rejects tool
calls, empty answers and incomplete streams. Partial text arrives as `Delta`
events, followed by one `Final` snapshot after successful validation. Partial
text can arrive before an error. The caller owns source selection, history and
the final run status.
The same transport bounds and cancellation behavior apply to both paths.

The key is optional. Pass `None` for an endpoint that runs without one, and the
client sends no authorization header. The response timeout is in seconds and
must be within `TIMEOUT_SECS` (5 to 600). `DEFAULT_TIMEOUT_SECS` is 45.

For browser tasks, call `next` with Stop, the remaining browser call budget and a `TextEvent` callback. It returns one validated `Invocation` at a time. Execute it through the application's browser authority and submit its actual result with `submit_tool_result`. Every sibling in a model response must validate and fit the remaining budget before the first call is returned. All results must arrive before another HTTP request. `usage` reports optional token counts for the latest completed response.

An incomplete stream, invalid call, changed reused identity, callback error or cancellation closes the conversation. Partial text can already have reached the callback, so the application must show the failed run. It must not count partial text as a completed result or repeat browser effects automatically. Active provider history stays in memory and is not a restart mechanism.

The transport requires HTTPS and disables proxies, redirects and retries. A supplied certificate bundle replaces default roots. Stop closes the local request. Remote inference and billing cancellation depend on the provider.

This client implements a bounded text and function tool subset of Chat Completions. Endpoint parameters, model access and reasoning formats vary by provider. Controlled checks do not establish live account access or support for every compatible vendor, model or capability.
