import { capturePage } from "./page-capture.js";

// The fonts ship beside the wasm, not inside it; they load while the wasm compiles.
async function load(url) {
  const response = await fetch(new URL(url, location.href));
  if (!response.ok) throw new Error(`${url} could not load (${response.status})`);
  return response;
}

async function loadFonts() {
  const { fonts } = await (await load("/chat/fonts/manifest.json")).json();
  return Promise.all(fonts.map(async name => new Uint8Array(await (await load(`/chat/fonts/${name}`)).arrayBuffer())));
}

try {
  const fonts = loadFonts();
  const { default: init, start_rune_chat, stop_rune_chat } = await import(/* @vite-ignore */ new URL("/chat/wasm/metldr_panel.js", location.href).href);
  const [files] = await Promise.all([fonts, init()]);
  start_rune_chat(capturePage, files);
  window.addEventListener("pagehide", stop_rune_chat, { once: true });
} catch (error) {
  document.body.textContent = String(error);
}
