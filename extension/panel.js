function capturePage() {
  // Chrome serializes this function without its module scope; cache per isolated document.
  const whitespaceSource = "\\s+";
  const whitespaceFlags = "gu";
  const whitespaceKey = Symbol.for(`metldr.capture.regex.${whitespaceFlags}.${whitespaceSource}`);
  const whitespacePattern = globalThis[whitespaceKey] ??= new RegExp(whitespaceSource, whitespaceFlags);
  const omitted = (element) => {
    if (element.matches("script,style,noscript,template,h1,h2,h3,form,input,textarea,select,button,nav,[hidden],[inert],[aria-hidden='true'],[role='textbox']") || element.isContentEditable) return true;
    const style = getComputedStyle(element);
    return style.display === "none" || style.visibility !== "visible" || Number(style.opacity) === 0 || style.contentVisibility === "hidden" || style.clipPath !== "none" || style.clip !== "auto";
  };
  const included = (candidate) => {
    let depth = 0;
    for (let element = candidate; element; element = element.parentElement) {
      if (++depth > 64 || omitted(element)) return false;
    }
    return true;
  };
  const articles = document.getElementsByTagName("article");
  let root = document.body;
  for (let index = 0; index < Math.min(articles.length, 32); index++) {
    if (included(articles[index])) { root = articles[index]; break; }
  }
  if (!root || !included(root)) throw new Error("Page content is unavailable.");
  let nodes = 0;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (++nodes > 4096) throw new Error("Page content exceeds the capture limit.");
      if (node.nodeType === Node.ELEMENT_NODE) return omitted(node) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_SKIP;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  const parts = [];
  let size = 0;
  let previousBlock;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.data.replace(whitespacePattern, " ");
    if (!text) continue;
    const range = document.createRange();
    range.selectNodeContents(node);
    if (![...range.getClientRects()].some((rect) => rect.width > 0 && rect.height > 0)) continue;
    let block = node.parentElement;
    while (block && block !== root && ["inline", "inline-block", "contents"].includes(getComputedStyle(block).display)) block = block.parentElement;
    const separator = previousBlock && previousBlock !== block ? " " : "";
    size += text.length + separator.length;
    if (size > 8192) throw new Error("Page content exceeds the capture limit.");
    parts.push(separator, text);
    previousBlock = block;
  }
  return {
    url: location.href,
    title: document.title,
    text: parts.join("").trim().replace(whitespacePattern, " "),
  };
}

// Internal Rune host route. Only `?host=rune` reaches it; the shipping panel never does. The
// wasm and the decoded fonts sit in `rune/` beside the page (built by `make rune-host`). The fonts
// load while the wasm compiles, and the host waits for both before it mounts.
async function fetchRuneFile(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url.pathname} could not load (${response.status})`);
  return response;
}

async function loadRuneFonts(base) {
  const manifest = await (await fetchRuneFile(new URL("fonts/manifest.json", base))).json();
  return Promise.all(manifest.fonts.map(async (font) => {
    const response = await fetchRuneFile(new URL(`fonts/${font.name}`, base));
    return new Uint8Array(await response.arrayBuffer());
  }));
}

async function startRuneHost() {
  const base = new URL("rune/", document.baseURI);
  const fonts = loadRuneFonts(base);
  const glue = await import(/* @vite-ignore */ new URL("metldr_panel.js", base).href);
  const initialized = glue.default({ module_or_path: new URL("metldr_panel_bg.wasm", base) });
  const [files] = await Promise.all([fonts, initialized]);
  // Lifecycle handle for the internal route only (the harness drives mount, stop and restart).
  const runeHost = {
    start: () => glue.start_rune_host(files),
    settle: () => glue.settle_rune_host(),
    stop: () => glue.stop_rune_host(),
    firstFrameMs: () => glue.rune_host_first_frame_ms(),
  };
  globalThis.metldrRuneHost = runeHost;
  runeHost.start();
  await waitUntilRunning(runeHost.settle);
}

async function waitUntilRunning(settle) {
  const deadline = performance.now() + 15000;
  for (;;) {
    const phase = settle();
    if (phase === "running") return;
    if (phase === "idle") throw new Error("the host closed before it opened");
    if (performance.now() > deadline) throw new Error("the renderer did not open in time");
    await new Promise((resolve) => setTimeout(resolve, 16));
  }
}

document.getElementById("panel-reload")?.addEventListener("click", (event) => {
  if (event.isTrusted) location.reload();
});
if (new URLSearchParams(location.search).get("host") === "rune") {
  startRuneHost().catch((error) => {
    const status = document.getElementById("panel-boot-status");
    if (status) status.textContent = `metldr Rune host could not start: ${error?.message ?? error}`;
    console.error("metldr Rune host startup failed", error);
  });
} else {
  try {
    const { default: init, start_panel } = await import("../crates/metldr-panel/pkg/metldr_panel.js");
    await init();
    start_panel(capturePage);
  } catch (error) {
    const status = document.getElementById("panel-boot-status");
    if (status) status.textContent = "metldr could not load. Reload the panel to try again.";
    console.error("metldr panel startup failed", error);
  }
}
