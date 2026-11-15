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

// The chat draws with WebGPU, or with WebGL2 where WebGPU has no adapter. Without either, say
// what to change instead of showing the renderer's error. `navigator.gpu` alone is not enough:
// with graphics acceleration off it is there and finds no adapter.
async function canDraw() {
  try {
    if (await navigator.gpu?.requestAdapter()) return true;
  } catch {
    // An adapter request that throws is no adapter.
  }
  try {
    return Boolean(document.createElement("canvas").getContext("webgl2"));
  } catch {
    return false;
  }
}

function explain(detail) {
  // A content grid: the text sits in a 34rem column with a 1rem gutter on narrow panels.
  const note = document.createElement("main");
  note.setAttribute("role", "alert");
  note.style.cssText = "display: grid; grid-template-columns: minmax(1rem, 1fr) min(100% - 2rem, 34rem) minmax(1rem, 1fr); padding-block: 3rem; font: 14px/1.5 system-ui, sans-serif;";
  const title = Object.assign(document.createElement("h1"), { textContent: "metldr cannot draw its chat here" });
  title.style.cssText = "grid-column: 2; margin-block: 0 0.5rem; font-size: 1.25rem; line-height: 1.25; text-wrap: balance;";
  const help = Object.assign(document.createElement("p"), { textContent: "It needs WebGPU or WebGL2. Turn on \u201cUse graphics acceleration when available\u201d in chrome://settings/system, relaunch Chrome, then open metldr again." });
  help.style.cssText = "grid-column: 2; margin: 0; text-wrap: pretty;";
  note.append(title, help);
  document.body.replaceChildren(note);
  if (detail) console.error("metldr chat could not start", detail);
}

if (!(await canDraw())) {
  explain();
} else try {
  const fonts = loadFonts();
  const { default: init, start_rune_chat, stop_rune_chat } = await import(/* @vite-ignore */ new URL("/chat/rune/metldr_panel.js", location.href).href);
  const [files] = await Promise.all([fonts, init()]);
  start_rune_chat(capturePage, files);
  window.addEventListener("pagehide", stop_rune_chat, { once: true });
} catch (error) {
  explain(error);
}
