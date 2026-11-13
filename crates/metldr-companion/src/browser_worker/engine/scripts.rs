mod buttons;
pub(super) mod controls;
pub(super) mod scroll;
pub(super) mod selects;
pub(super) use buttons::{arm_button, button_receipt, check_button};

/// The secret metadata pattern. The page capture script and the Rust log redaction both read it from here.
macro_rules! secret_metadata_pattern {
    () => {
        r"password|passcode|one[ _-]?time[ _-]?code|(^|[^a-z0-9])otp($|[^a-z0-9])|token|secret|api[ _-]?key|payment|bank|card|cc-|cc[ _-]?(number|name|exp|csc|cvc|cvv)|(^|[^a-z0-9])(cvc|cvv|ssn)($|[^a-z0-9])|social[ _-]?security"
    };
}
pub(super) const SECRET_METADATA_PATTERN: &str = secret_metadata_pattern!();

const HELPERS: &str = concat!(
    r#"
const encoder = new TextEncoder();
const whitespacePattern = /\s+/g;
const labelReferencesPattern = /\s+/;
const secretMetadataPattern = /"#,
    secret_metadata_pattern!(),
    r#"/;
const clean = value => String(value || '').replace(whitespacePattern, ' ').trim();
const bounded = (value, limit) => {
  let text = '', size = 0, truncated = false;
  for (const character of String(value || '')) {
    const bytes = encoder.encode(character).length;
    if (size + bytes > limit) { truncated = true; break; }
    text += character; size += bytes;
  }
  return [text, truncated];
};
const label = element => {
  const direct = element.getAttribute('aria-label');
  if (direct) return clean(direct);
  const labelled = element.getAttribute('aria-labelledby');
  const references = labelled ? labelled.split(labelReferencesPattern) : [];
  const labels = element.labels ? Array.from(element.labels) : [];
  if (references.length > 32 || labels.length > 32) return null;
  const referenced = references.map(id => labelText(element.getRootNode().getElementById(id)));
  const associated = labels.map(labelText);
  if (referenced.includes(null) || associated.includes(null)) return null;
  return clean(referenced.join(' ') || associated.join(' ') || element.getAttribute('title') || element.placeholder || '');
};
// Memos live for one invocation of the wrapped function, so one capture() shares them across
// every node it visits. A caller that changes the DOM must finish its visibility checks first.
const styleMemo = new WeakMap(), chainMemo = new WeakMap(), visibilityMemo = new WeakMap();
const styleOf = node => {
  let style = styleMemo.get(node);
  if (!style) { style = getComputedStyle(node); styleMemo.set(node, style); }
  return style;
};
const hiddenByStyle = style => style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse' || Number(style.opacity) === 0 || style.contentVisibility === 'hidden' || style.clipPath !== 'none' || style.clip !== 'auto';
// Summarizes an element and its element ancestors: how many there are, and whether none is hidden by style.
const chainOf = element => {
  const uncached = [];
  let node = element, summary = { depth: 0, clear: true };
  while (node && node.nodeType === 1) {
    const cached = chainMemo.get(node);
    if (cached) { summary = cached; break; }
    uncached.push(node);
    node = node.parentElement || node.getRootNode().host;
  }
  for (let index = uncached.length - 1; index >= 0; index--) {
    const current = uncached[index];
    summary = { depth: summary.depth + 1, clear: summary.clear && !current.matches('[hidden],[inert],[aria-hidden="true"]') && !hiddenByStyle(styleOf(current)) };
    chainMemo.set(current, summary);
  }
  return summary;
};
const visible = element => {
  let known = visibilityMemo.get(element);
  if (known === undefined) {
    known = false;
    if (element.isConnected && element.ownerDocument === document && !element.closest('[hidden],[inert],[aria-hidden="true"]')) {
      const chain = chainOf(element);
      known = chain.depth <= 64 && chain.clear && element.getClientRects().length > 0;
    }
    visibilityMemo.set(element, known);
  }
  return known;
};
const labelText = root => {
  if (!root || !visible(root)) return '';
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
  let text = '', nodes = 0;
  while (walker.nextNode()) {
    if (++nodes > 512) return null;
    const node = walker.currentNode;
    if (node.nodeType !== Node.TEXT_NODE) continue;
    const parent = node.parentElement;
    if (!parent || parent.closest('script,style,noscript,input,textarea,select,[contenteditable]:not([contenteditable="false"])') || !visible(parent)) continue;
    text += node.nodeValue || '';
    if (text.length > 512) return null;
  }
  return text;
};
const safeLabel = element => {
  const accessible = label(element);
  if (accessible === null) return null;
  const metadataValues = [element.autocomplete, element.name, element.id, accessible, element.placeholder];
  if (metadataValues.some(value => String(value || '').length > 512)) return null;
  const metadata = metadataValues.join(' ').toLowerCase();
  if (secretMetadataPattern.test(metadata)) return null;
  return accessible;
};
const secretKind = element => {
  if (!(element instanceof HTMLInputElement) || !['text','password','tel','number'].includes(element.type) || element.matches(':disabled') || element.readOnly || !visible(element)) return null;
  const accessible = label(element);
  if (accessible === null) return null;
  const values = [element.autocomplete, element.name, element.id, accessible, element.placeholder];
  if (values.some(value => String(value || '').length > 512)) return null;
  const metadata = values.join(' ').toLowerCase();
  if (/card|payment|bank|cc-|cc[ _-]?(number|name|exp|csc|cvc|cvv)|(^|[^a-z0-9])(cvc|cvv|ssn)($|[^a-z0-9])|social[ _-]?security|api[ _-]?key|token|secret/.test(metadata)) return null;
  if (/one[ _-]?time[ _-]?code|(^|[^a-z0-9])otp($|[^a-z0-9])/.test(metadata)) return 'one_time_code';
  if (element.type === 'password') return 'password';
  return null;
};
const refElement = saved => {
  const element = saved?.element || saved;
  if (!element || !element.isConnected || element.ownerDocument !== document) throw new Error('removed snapshot ref');
  const identity = root?.__metldr_refs?.get(element);
  if (!identity || identity.parent !== element.parentNode || identity.type !== element.type || identity.id !== element.id || identity.inputName !== element.name || identity.autocomplete !== element.autocomplete || identity.role !== element.getAttribute('role')) throw new Error('stale snapshot ref');
  if (identity.name !== label(element) || identity.text !== eligibleButton(element)) throw new Error('renamed snapshot ref');
  return element;
};
const textField = element => element instanceof HTMLTextAreaElement || element instanceof HTMLInputElement && element.type === 'text';
const eligible = element => {
  if (!textField(element) || element.matches(':disabled') || element.readOnly || !visible(element)) return null;
  return safeLabel(element);
};
const eligibleButton = element => {
  const native = element instanceof HTMLButtonElement || element instanceof HTMLInputElement && ['button','submit','reset'].includes(element.type);
  if (!native || element.matches(':disabled') || element.getAttribute('aria-disabled') === 'true' || !visible(element)) return null;
  const accessible = label(element);
  if (accessible === null) return null;
  const text = accessible || (element instanceof HTMLButtonElement ? labelText(element) : element.value);
  if (text === null) return null;
  const result = clean(text);
  return result && result.length <= 512 ? result : null;
};
"#
);

fn function(body: &str) -> String {
    format!("function(root) {{ {HELPERS} {body} }}")
}

pub(super) fn capture(scope: Option<&str>) -> String {
    let body = format!(
        "{}\n{}\n{}",
        selects::HELPERS,
        controls::HELPERS,
        r#"
const scope = CAPTURE_SCOPE;
const scoped = CAPTURE_SCOPED;
const snapshot = [];
const identities = new WeakMap();
Object.defineProperty(snapshot, '__metldr_refs', { value: identities });
const remember = element => { identities.set(element, { parent: element.parentNode, name: label(element), type: element.type, id: element.id, inputName: element.name, autocomplete: element.autocomplete, role: element.getAttribute('role'), text: eligibleButton(element) }); return element; };
const buttons = [];
const selects = [];
const controls = [];
const scrollTargets = [];
let optionCount = 0;
Object.defineProperty(snapshot, '__metldr_buttons', { value: buttons });
Object.defineProperty(snapshot, '__metldr_selects', { value: selects });
Object.defineProperty(snapshot, '__metldr_controls', { value: controls });
Object.defineProperty(snapshot, '__metldr_scrollers', { value: scrollTargets });
const summary = { url: bounded(location.href, 2048)[0], title: bounded(document.title, 128)[0], text: '', text_truncated: false, fields_truncated: false, fields: [], buttons_truncated: false, buttons: [], selects_truncated: false, selects: [], controls_truncated:false, controls:[], scroll_targets_truncated: false, frame_scroll_targets_observed: false, scroll_targets: [] };
const readonlyFields = [];
const scrollingElement = document.scrollingElement;
if (!scrollingElement || scrollingElement.ownerDocument !== document) throw new Error('document scrolling element unavailable');
const saveScroller = (element, kind, slot) => {
  const style = styleOf(element);
  const accessible = label(element);
  const targetLabel = bounded(accessible || (kind === 'page' ? 'Page' : `Scrollable region ${slot}`), 128)[0];
  const saved = { element, parent: element.parentElement, document, url: location.href, kind, label: targetLabel, overflowX: style.overflowX, overflowY: style.overflowY, width: element.scrollWidth, height: element.scrollHeight, clientWidth: element.clientWidth, clientHeight: element.clientHeight, x: element.scrollLeft, y: element.scrollTop };
  const index = scrollTargets.push((remember(element), saved)) - 1;
  summary.scroll_targets.push({ slot: index, kind, label: targetLabel, x: saved.x, y: saved.y });
};
if (!scoped || scope === document.body || scope === document.documentElement) saveScroller(scrollingElement, 'page', 0);
let bytes = 0, nodes = 0;
const walkers = [document.createTreeWalker(scope, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT)];
let first = scope;
const nextNode = () => { while (walkers.length) { const walker = walkers[walkers.length - 1]; if (walker.nextNode()) return walker.currentNode; walkers.pop(); } return null; };
let current;
while ((current = first || nextNode())) {
  first = null;
  nodes++;
  if (nodes > 4096) { summary.text_truncated = true; summary.fields_truncated = true; summary.buttons_truncated = true; summary.selects_truncated = true; summary.controls_truncated = true; summary.scroll_targets_truncated = true; break; }
  const node = current;
  if (node.nodeType === Node.ELEMENT_NODE) {
    if (node.shadowRoot) walkers.push(document.createTreeWalker(node.shadowRoot, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT));
    if (node !== scrollingElement && node !== document.documentElement && node !== document.body && node.ownerDocument === document && node.isConnected && visible(node)) {
      const style = styleOf(node);
      const canScroll = value => ['auto','scroll','overlay'].includes(value);
      if ((canScroll(style.overflowX) && node.scrollWidth > node.clientWidth) || (canScroll(style.overflowY) && node.scrollHeight > node.clientHeight)) {
        if (scrollTargets.length < 16) saveScroller(node, 'region', scrollTargets.length);
        else summary.scroll_targets_truncated = true;
      }
    }
    const select = captureSelect(node);
    const control = captureControl(node, controls.length);
    if (control) {
      if (control.blocked || controls.length === 32) summary.controls_truncated = true;
      else {
        remember(node);
        controls.push(control);
        summary.controls.push({slot:control.slot,label:control.label,state:control.state});
      }
    }
    if (select) {
      if (select.oversized || selects.length === 32 || optionCount + select.options.length > 64) summary.selects_truncated = true;
      else {
        const slot = selects.push((remember(node), select)) - 1;
        optionCount += select.options.length;
        summary.selects.push({ slot, label: select.label, selected: node.selectedIndex < 0 ? null : node.selectedIndex, options: select.options.map(option => ({ slot: option.index, label: option.label, enabled: option.enabled })) });
      }
    }
    const buttonLabel = eligibleButton(node);
    if (buttonLabel !== null) {
      if (buttons.length === 32) summary.buttons_truncated = true;
      else {
        const slot = buttons.push(remember(node)) - 1;
        summary.buttons.push({ slot, label: bounded(buttonLabel, 128)[0], check_label: buttonLabel });
      }
    }
    const secret = secretKind(node);
    if (secret) {
      if (summary.fields.length === 32) { summary.fields_truncated = true; continue; }
      const accessible = label(node);
      const slot = snapshot.push(remember(node)) - 1;
      summary.fields.push({ slot, label: bounded(accessible || 'Secret field', 128)[0], check_label: accessible, secret });
    } else if (textField(node)) {
      if (node.readOnly && visible(node)) {
        const accessible = safeLabel(node);
        if (accessible !== null && readonlyFields.length < 32) readonlyFields.push({label:bounded(accessible || 'Read-only field', 128)[0], value:bounded(node.value, 1024)[0]});
      }
      const accessible = eligible(node);
      if (accessible !== null) {
        if (summary.fields.length === 32) { summary.fields_truncated = true; continue; }
        const slot = snapshot.push(remember(node)) - 1;
        const fieldLabel = bounded(accessible || `Text field ${slot + 1}`, 128)[0];
        summary.fields.push({ slot, label: fieldLabel, check_label: accessible });
      }
    }
    continue;
  }
  if (summary.text_truncated) continue;
  const parent = node.parentElement || node.getRootNode().host;
  if (!parent || parent.closest('script,style,noscript,input,textarea,select,[contenteditable]:not([contenteditable="false"])')) continue;
  if (!visible(parent)) continue;
  const [part, truncated] = bounded(node.nodeValue || '', 4096 - bytes);
  if (part) {
    const separator = summary.text ? ' ' : '';
    const [piece, separatorTruncated] = bounded(separator + part, 4096 - bytes);
    summary.text += piece; bytes += encoder.encode(piece).length;
    if (separatorTruncated || truncated) summary.text_truncated = true;
  }
  if (truncated && !part) summary.text_truncated = true;
}
const secretValues = new Set();
const inputs = [], roots = [document];
let inspected = 0;
for (let index = 0; index < roots.length; index++) {
  for (const element of roots[index].querySelectorAll('*')) {
    if (++inspected > 4096) throw new Error('secret redaction capacity exceeded');
    if (element.matches('input,textarea,select')) inputs.push(element);
    if (element.shadowRoot) roots.push(element.shadowRoot);
  }
}
let secretBytes = 0;
for (const input of inputs) {
  const metadata = [input.autocomplete, input.name, input.id, input.getAttribute('aria-label'), input.placeholder, label(input)].join(' ').toLowerCase();
  if (input.type !== 'password' && !secretMetadataPattern.test(metadata)) continue;
  const value = input.value;
  if (!value) continue;
  secretBytes += value.length;
  if (secretBytes > 1048576) throw new Error('secret redaction capacity exceeded');
  secretValues.add(value);
}
const secrets = Array.from(secretValues).sort((a,b) => b.length - a.length);
const redact = value => {
  if (typeof value === 'string') {
    for (const secret of secrets) value = value.split(secret).join('[redacted]');
    return value;
  }
  if (Array.isArray(value)) return value.map(redact);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key,item]) => [key,redact(item)]));
  return value;
};
if (scoped) summary.scoped = true;
if (readonlyFields.length) summary.readonly_fields = readonlyFields;
Object.defineProperty(snapshot, '__metldr_summary', { value: redact(summary) });
return snapshot;
"#,
    );
    function(
        &body
            .replace(
                "CAPTURE_SCOPED",
                if scope.is_some() { "true" } else { "false" },
            )
            .replace(
                "CAPTURE_SCOPE",
                scope.unwrap_or("document.body || document.documentElement"),
            ),
    )
}

pub(super) fn read_summary() -> String {
    function("return root.__metldr_summary;")
}

pub(super) fn read_value(slot: usize, check_label: &str) -> String {
    let slot = serde_json::to_string(&slot).expect("usize serializes");
    let label = serde_json::to_string(check_label).expect("string serializes");
    function(&format!(
        "const element = root[{slot}]; const current = eligible(element); if (current === null || current !== {label} || encoder.encode(element.value).length > 1024) return null; return element.value;"
    ))
}

pub(super) fn write_value(slot: usize, check_label: &str, previous: &str, value: &str) -> String {
    let slot = serde_json::to_string(&slot).expect("usize serializes");
    let label = serde_json::to_string(check_label).expect("string serializes");
    let previous = serde_json::to_string(previous).expect("string serializes");
    let value = serde_json::to_string(value).expect("string serializes");
    function(&format!(
        "const element = root[{slot}]; const current = eligible(element); if (current === null || current !== {label} || element.value !== {previous}) return false; element.value = {value}; element.dispatchEvent(new Event('input', {{ bubbles: true }})); return true;"
    ))
}

/// Types the secret the owner passes as the second call argument. The secret never appears in
/// this source and the result is only whether the field now holds it.
pub(super) fn write_secret(slot: usize, check_label: &str, kind: &str) -> String {
    let slot = serde_json::to_string(&slot).expect("usize serializes");
    let label = serde_json::to_string(check_label).expect("string serializes");
    let kind = serde_json::to_string(kind).expect("string serializes");
    format!(
        "function(root, secret) {{ {HELPERS} const element = root[{slot}]; if (!element || !element.isConnected || element.ownerDocument !== document || secretKind(element) !== {kind} || label(element) !== {label} || typeof secret !== 'string') return false; Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(element, secret); element.dispatchEvent(new Event('input', {{ bubbles: true }})); element.dispatchEvent(new Event('change', {{ bubbles: true }})); return element.value === secret; }}"
    )
}

pub(super) fn verify_value(slot: usize, check_label: &str, value: &str) -> String {
    let slot = serde_json::to_string(&slot).expect("usize serializes");
    let label = serde_json::to_string(check_label).expect("string serializes");
    let value = serde_json::to_string(value).expect("string serializes");
    function(&format!(
        "const element = root[{slot}]; const current = eligible(element); return current !== null && current === {label} && element.value === {value} ? element.value : null;"
    ))
}
