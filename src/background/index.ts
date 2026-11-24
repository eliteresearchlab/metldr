import '@inboxsdk/core/background';
import initWorker, { accept_panel, background_message, gateway_accept, stop_all_runs } from '../../crates/metldr-panel/pkg/metldr_panel';
import workerWasmUrl from '../../crates/metldr-panel/pkg/metldr_panel_bg.wasm?url';

// The service worker: one Rust module relays the side panel to the companion and answers the
// extension's messages and its model gateway port. This script loads it and opens the side panel.

let worker: ReturnType<typeof initWorker> | undefined;

function loadWorker(): ReturnType<typeof initWorker> {
  return worker ??= initWorker({ module_or_path: chrome.runtime.getURL(workerWasmUrl.replace(/^\//, '')) }).catch(error => {
    worker = undefined;
    throw error;
  });
}

// A port that arrives while the module loads is handed over once it is ready: the gateway with
// the messages it sent meanwhile, the panel (at most 16 waiting) as the companion relay expects.
const PANEL_PORT = 'metldr-first-flow';
let waitingPanels = 0;

chrome.runtime.onConnect.addListener((port) => {
  if (port.name === PANEL_PORT) {
    if (waitingPanels >= 16) {
      port.disconnect();
      return;
    }
    waitingPanels++;
    loadWorker().then(() => accept_panel(port), (error) => {
      console.error('metldr could not load its service worker module.', error);
      port.disconnect();
    }).finally(() => { waitingPanels--; });
    return;
  }
  if (port.name !== 'metldr-gateway') return;
  const early: unknown[] = [];
  const hold = (message: unknown) => { early.push(message); };
  port.onMessage.addListener(hold);
  loadWorker().then(() => {
    port.onMessage.removeListener(hold);
    gateway_accept(port, early);
  }, (error) => {
    console.error('metldr could not load its service worker module.', error);
    port.disconnect();
  });
});

chrome.commands.onCommand.addListener((command, tab) => {
  if (command === 'stop-all-runs') {
    loadWorker().then(stop_all_runs, error => console.error('metldr could not stop its runs.', error));
    return;
  }
  if (command !== 'open-browser-agent' || tab?.id === undefined || tab.id < 0) return;
  // The shortcut's gesture still counts here, so the panel opens without waiting for the module.
  openPanel(tab.id, answer => {
    if (!(answer as { success: boolean }).success) console.error('metldr could not open its panel.', answer);
  });
});

loadWorker().catch(error => console.error('metldr could not load its service worker module.', error));

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
  loadWorker().then(() => background_message(message, sender)).then(
    answer => respond(answer),
    (error: Error) => respond({ success: false, error: error.message })
  );
  return true;
});

chrome.sidePanel.setPanelBehavior({ openPanelOnActionClick: true })
  .catch(error => console.error('metldr could not set the side panel behaviour.', error));

// Tabs open before an install or update have no content script of this version: each gets the
// one its page would load (Gmail's or every other page's).
async function injectOpenTabs(): Promise<void> {
  const tabs = await chrome.tabs.query({ url: ['http://*/*', 'https://*/*', 'file:///*'] });
  await Promise.all(tabs.map(tab => {
    if (tab.id === undefined || tab.discarded) return undefined;
    const gmail = tab.url?.startsWith('https://mail.google.com/') === true;
    return chrome.scripting.executeScript({ target: { tabId: tab.id }, files: [gmail ? 'gmail.js' : 'content.js'] })
      .catch(() => undefined);
  }));
}

chrome.runtime.onInstalled.addListener((details) => {
  if (details.reason === 'install') {
    chrome.tabs.create({ url: chrome.runtime.getURL('welcome.html') });
  }
  if (details.reason === 'install' || details.reason === 'update') {
    injectOpenTabs().catch(error => console.error('metldr could not reach the open tabs.', error));
  }
});
