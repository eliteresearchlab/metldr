import { capturePage } from "./page-capture.js";

try {
  const { default: init, start_rune_chat, stop_rune_chat } = await import(/* @vite-ignore */ new URL("/chat/wasm/metldr_panel.js", location.href).href);
  await init();
  start_rune_chat(capturePage);
  window.addEventListener("pagehide", stop_rune_chat, { once: true });
} catch (error) {
  document.body.textContent = String(error);
}
