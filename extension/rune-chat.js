import init, { start_rune_chat, stop_rune_chat } from "../chat/wasm/metldr_panel.js";
import { capturePage } from "./page-capture.js";

try {
  await init();
  start_rune_chat(capturePage);
  window.addEventListener("pagehide", stop_rune_chat, { once: true });
} catch (error) {
  document.body.textContent = String(error);
}
