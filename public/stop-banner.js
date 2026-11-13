(() => {
  if (globalThis.__metldrStopBanner) return;
  const host = document.createElement("div");
  const root = host.attachShadow({ mode: "closed" });
  const style = document.createElement("style");
  style.textContent = `
    :host { all: initial; position: fixed; inset-block-start: 12px; inset-inline-end: 12px; z-index: 2147483647; }
    div { display: flex; align-items: center; gap: 12px; padding: 8px 12px; border: 1px solid oklch(0.45 0.04 260); border-radius: 8px; background: oklch(0.2 0.02 260); color: oklch(1 0 none); font: 14px system-ui, sans-serif; }
    button { min-block-size: 44px; padding-inline: 16px; border: 0; border-radius: 5px; background: oklch(0.62 0.2 25); color: oklch(1 0 none); font: inherit; cursor: pointer; }
    button:focus-visible { outline: 2px solid oklch(1 0 none); outline-offset: 2px; }
    button:active { transform: scale(0.97); }
  `;
  const bar = document.createElement("div");
  const label = document.createElement("span");
  label.textContent = "MetLDR controls this tab";
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = "Stop";
  button.addEventListener("click", (event) => {
    if (event.isTrusted) chrome.runtime.sendMessage({ kind: "metldr-banner-stop" });
  });
  bar.append(label, button);
  root.append(style, bar);
  document.documentElement.append(host);
  globalThis.__metldrStopBanner = host;
  chrome.runtime.onMessage.addListener(function remove(message) {
    if (message?.kind !== "metldr-banner-remove") return;
    host.remove();
    delete globalThis.__metldrStopBanner;
    chrome.runtime.onMessage.removeListener(remove);
  });
})();
