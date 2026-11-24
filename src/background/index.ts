import '@inboxsdk/core/background';
import '../../extension/background.js';
import initCore, { background_message, gateway_accept } from '../../crates/metldr-core/pkg/metldr_core';
import coreWasmUrl from '../../crates/metldr-core/pkg/metldr_core_bg.wasm?url';

// The service worker: the Rust core answers the extension's messages and its model gateway port.
// This script only loads the core and opens the side panel.

let core: ReturnType<typeof initCore> | undefined;

function loadCore(): ReturnType<typeof initCore> {
  return core ??= initCore({ module_or_path: chrome.runtime.getURL(coreWasmUrl.replace(/^\//, '')) }).catch(error => {
    core = undefined;
    throw error;
  });
}

// The model gateway port. Messages that arrive while the core loads are held and handed over in
// order, so a request sent right after connecting is never lost.
chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== 'metldr-gateway') return;
  const early: unknown[] = [];
  const hold = (message: unknown) => { early.push(message); };
  port.onMessage.addListener(hold);
  loadCore().then(() => {
    port.onMessage.removeListener(hold);
    gateway_accept(port, early);
  }, (error) => {
    console.error('metldr could not load its model gateway.', error);
    port.disconnect();
  });
});

// The side panel opens only while the reader's click still counts, so this never waits for the
// core. The PDF tools' Chat asks for it.
function openPanel(tab: number, respond: (answer: unknown) => void): void {
  chrome.sidePanel.setOptions({ tabId: tab, path: 'chat.html', enabled: true });
  chrome.sidePanel.open({ tabId: tab }).then(
    () => respond({ success: true }),
    (error: Error) => respond({ success: false, error: error.message })
  );
}

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (message?.type === 'OPEN_SIDE_PANEL') {
    if (sender.tab?.id === undefined) {
      respond({ success: false, error: 'no tab' });
      return false;
    }
    openPanel(sender.tab.id, respond);
    return true;
  }
  // A message without a type belongs to another listener (the browser tools' banner).
  if (typeof message?.type !== 'string') return false;
  // A type the core does not know answers nothing, as before, so its sender never waits.
  loadCore().then(() => background_message(message, sender)).then(
    answer => respond(answer),
    (error: Error) => respond({ success: false, error: error.message })
  );
  return true;
});

chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true })
  .catch(error => console.error('metldr could not set the side panel behaviour.', error));

chrome.runtime.onInstalled.addListener((details) => {
  if (details.reason === 'install') {
    chrome.tabs.create({ url: chrome.runtime.getURL('welcome.html') });
  }
});
