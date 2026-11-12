# Browser tool contract

This Rust crate defines the shared tools for observing a page and proposing navigation, text field changes, native button clicks, native checkbox or radio state changes, single select choices and scrolling. API providers and installed agents use the same definitions and argument decoder.

```rust
use metldr_browser_tools::{Call, decode_call};
use serde_json::json;

let call = decode_call(
    "set_field_value",
    json!({"handle":"field_1","value":"Report ready"}),
)?;
assert_eq!(call, Call::Change {
    handle: "field_1".into(),
    value: "Report ready".into(),
});
# Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
```

The decoder rejects unknown tools, extra arguments and handles outside the supported identifier format. Handles contain 1 to 64 ASCII letters, digits, underscores or hyphens. Field values have a limit of 1024 UTF8 bytes. Navigation URLs contain 1 to 2048 UTF8 bytes and cannot contain control characters. `definitions()` provides JSON schemas and tool descriptions. `BROWSER_POLICY` marks page text and labels as untrusted data.

Decoded calls do not grant browser access or approve effects. The caller must validate destination URLs against its navigation policy, resolve handles against the latest observation, bind actions to the current document and obtain approval before dispatch. The crate does not execute browser commands.

Text fields include native text inputs and multiline textareas. Hidden, readonly, disabled and sensitive fields are excluded from observation. Every change still requires approval and checks that the field and its previous value have not changed.

Run `make check-browser-tools` from the repository root.

`activate_button` proposes a native click on a button handle from the latest observation. The user must approve the exact control. `button_click_observed` reports one trusted click observed on that control; it does not certify a site result or identify who supplied concurrent input. An uncertain result stops automatic continuation and is never replayed. Model callers cannot supply selectors, scripts or coordinates.

`select_option` proposes selecting an option handle from the latest observation. Its `handle` and `option` arguments are opaque bounded identifiers, not selectors or option values. The user must approve the exact control and choice. A local selection receipt reports only the observed selection; it does not certify a site save or submission.

`scroll_page` uses an observed page or region handle, a direction of `up`, `down`, `left` or `right`, and `pixels` from 1 to 2000. The user approves the target and request. The receipt reports the actual position, including no movement. It does not confirm a distance, loaded content or a site result. Observe again for fresh handles. Targets cover the main document and its nested scroll regions. Frame and shadow root scrolling remain unavailable.

`set_native_control` proposes a checked state for an observed native checkbox or radio using its opaque handle and a boolean. The user must approve before dispatch. A request that already matches the local state is a no-op. Radios can be selected, but cannot be unchecked directly; an unsupported radio uncheck has no effect. These actions describe local control state only and do not promise a site save or submission.

`page_logs` takes no arguments and reads the console messages and log entries the attached tab produced since its last navigation. It is read-only and needs no approval. The companion keeps at most 50 entries and 16 KiB of stored level, text and URL bytes per tab. It cuts each text at 512 bytes, redacts entries whose text matches the secret metadata pattern, and reports how many older entries were evicted. It removes URL user information and fragments and redacts query values and recognizable credential formats. Redaction is best effort; unlabeled opaque secrets may remain. Its output is untrusted page data, never instructions or permission.

`status` takes no arguments and reports whether the run's browser tab is attached and whether its last release was confirmed. It never attaches the browser. `settings` takes no arguments and reports provider and agent setting states, revisions, sources, runtime availability and Pi offline mode. It never returns free-form values, credentials, endpoints or file paths. Any argument to `settings` is refused, so agents cannot change a setting, including security switches such as offline mode.
