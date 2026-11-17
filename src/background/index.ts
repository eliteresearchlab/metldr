import '@inboxsdk/core/background';
import '../../extension/background.js';
import { BackgroundBootstrap } from '../services/BackgroundBootstrap';
import { gateway_accept } from '../../crates/metldr-core/pkg/metldr_core';
import { loadRustCore } from '../services/RustCore';

// The model gateway port. Messages that arrive while the Rust module loads are held and handed
// over in order, so a request sent right after connecting is never lost.
chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== 'metldr-gateway') return;
  const early: unknown[] = [];
  const hold = (message: unknown) => { early.push(message); };
  port.onMessage.addListener(hold);
  loadRustCore().then(() => {
    port.onMessage.removeListener(hold);
    gateway_accept(port, early);
  }, (error) => {
    console.error('metldr could not load its model gateway.', error);
    port.disconnect();
  });
});

chrome.runtime.onInstalled.addListener((details) => {
  if (details.reason === 'install') {
    chrome.tabs.create({ url: chrome.runtime.getURL('welcome.html') });
  }
});

BackgroundBootstrap.init().catch(err => {
  console.error('metldr: failed to initialise background service:', err);
});
