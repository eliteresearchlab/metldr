use super::function;

/// Focuses the approved field and listens for exactly one trusted keydown of `key` on it.
pub(in super::super) fn arm_key(slot: usize, label: &str, key: &str) -> String {
    let label = serde_json::to_string(label).expect("string serializes");
    let key = serde_json::to_string(key).expect("string serializes");
    function(&format!(
        r#"
if (root.__metldr_key_receipt) root.__metldr_key_receipt.dispose();
const element = root[{slot}];
if (!element || !element.isConnected || eligible(element) !== {label}) return false;
element.focus();
if (document.activeElement !== element) return false;
let count = 0, matches = true, disposed = false;
const listener = event => {{
  if (!event.isTrusted) return;
  count++;
  matches = matches && event.key === {key} && event.composedPath().includes(element);
}};
const dispose = () => {{
  if (disposed) return;
  disposed = true;
  window.removeEventListener('keydown', listener, true);
  clearTimeout(timer);
}};
const timer = setTimeout(dispose, 8000);
window.addEventListener('keydown', listener, true);
root.__metldr_key_receipt = {{ dispose, read: () => {{ const valid = !disposed && count === 1 && matches; dispose(); return valid; }} }};
return true;
"#
    ))
}

pub(in super::super) fn key_receipt() -> String {
    function(
        "const receipt = root.__metldr_key_receipt; if (!receipt) return false; delete root.__metldr_key_receipt; return receipt.read();",
    )
}
