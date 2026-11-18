import { capturePage } from "./page-capture.js";

// The fonts ship beside the wasm, not inside it. The interface fonts load while the graphics check
// runs; the fallbacks for other scripts and emoji (about 33 MB) wait until the chat has drawn.
async function load(url) {
  const response = await fetch(new URL(url, location.href));
  if (!response.ok) throw new Error(`${url} could not load (${response.status})`);
  return response;
}

async function loadFile(name) {
  const response = await load(`/chat/fonts/${name}`);
  return new Uint8Array(await response.arrayBuffer());
}

const loadFiles = names => Promise.all(names.map(loadFile));
const nextFrame = () => new Promise(resolve => requestAnimationFrame(resolve));
const idle = () => new Promise(resolve => requestIdleCallback(resolve, { timeout: 1000 }));

// A font group from the manifest, fetched now and parsed once the page is idle, so the copy into
// the renderer never delays a frame the user is waiting for. Without it the chat still works, so
// a failure is only logged.
async function addGroup(add, group) {
  try {
    const files = await loadFiles((await manifest)[group]);
    await idle();
    add(group, files);
  } catch (error) {
    console.error(`metldr chat could not load its ${group} fonts`, error);
  }
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

// PDFs are read here, in the chat page, by the PDF module (PDFium, about 8 MB), which loads the
// first time one is attached; the file never crosses to the service worker.
let pdfReader;
async function attach(file, chat) {
  if (!file) return;
  try {
    pdfReader ??= import(/* @vite-ignore */ new URL("/chat/pdf/metldr_pdf.js", location.href).href)
      .then(async module => (await module.default({ module_or_path: new URL("/assets/metldr_pdf_bg.wasm", location.href) }), module));
    const { pdf_read } = await pdfReader;
    chat.rune_chat_attach(file.name, await pdf_read(new Uint8Array(await file.arrayBuffer()), 2000));
  } catch (error) {
    pdfReader = undefined;
    chat.rune_chat_attach(file.name, "", `${file.name} could not be read as a PDF.`);
    console.error("metldr could not read a PDF", error);
  }
}

// The file picker the chat opens from its sources menu, and PDFs dropped on the chat.
function documents(chat) {
  const input = Object.assign(document.createElement("input"), { type: "file", accept: "application/pdf,.pdf", hidden: true });
  input.addEventListener("change", () => {
    attach(input.files[0], chat);
    input.value = "";
  });
  document.body.append(input);
  window.addEventListener("dragover", event => event.preventDefault());
  window.addEventListener("drop", event => {
    event.preventDefault();
    attach([...event.dataTransfer.files].find(file => file.type === "application/pdf" || file.name.endsWith(".pdf")), chat);
  });
  return () => input.click();
}

// The renderer's glue and the interface fonts load while the graphics check runs, instead of
// after it: the check waits on the GPU process, and the chat needs both anyway. A failed load
// reports itself only once the check has passed.
const glue = import(/* @vite-ignore */ new URL("/chat/rune/metldr_panel.js", location.href).href);
const manifest = load("/chat/fonts/manifest.json").then(response => response.json());
const fonts = manifest.then(({ fonts }) => loadFiles(fonts));
glue.catch(() => {});
fonts.catch(() => {});
if (!(await canDraw())) {
  explain();
} else try {
  const chat = await glue;
  const { default: init, start_rune_chat, stop_rune_chat, add_rune_chat_fonts } = chat;
  const [files] = await Promise.all([fonts, init()]);
  // The chat asks for the CJK collection (about 19.5 MB) only once it lays out CJK text.
  start_rune_chat(capturePage, documents(chat), files, group => addGroup(add_rune_chat_fonts, group));
  window.addEventListener("pagehide", stop_rune_chat, { once: true });
  // Two frames in, the chat has drawn; the other fallbacks follow.
  nextFrame().then(nextFrame).then(() => addGroup(add_rune_chat_fonts, "fallbacks"));
} catch (error) {
  explain(error);
}
