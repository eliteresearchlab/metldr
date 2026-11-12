import { Idiomorph } from "idiomorph/dist/idiomorph.esm.js";
import { AnswerFailureKind, GuestCommandKind, GuestProgram, GuestSession, initSync } from "../../crates/metldr-answer-codec/pkg/answer_codec.js";
import answerCodecWasm from "../../crates/metldr-answer-codec/pkg/answer_codec_bg.wasm?binary";
import { AnswerProjection } from "./answer-projection";
import "../../crates/metldr-panel/assets/answer-renderer.js";
import "../../crates/metldr-panel/assets/answer-frame.css";

const module = new WebAssembly.Module(answerCodecWasm);
initSync({ module });
const policy = document.createElement("meta");
policy.httpEquiv = "Content-Security-Policy";
policy.content = "default-src 'none'; script-src 'unsafe-eval' blob:; worker-src blob:; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'";
document.head.append(policy);

function start(initialization: string, report: (encoded: string) => void, followup: boolean, onRuntimeFailure: (reason: AnswerFailureKind) => void) {
  const session = GuestSession.new(initialization);
  let projection: AnswerProjection | undefined;
  let running = false;
  let closed = false;
  let finalDocument = false;
  let rendered = false;
  let followupPending = false;
  let lastRequest = 1;
  let resizeFrame = 0;
  const abort = new AbortController();
  const failed = () => {
    if (!closed) { try { report(session.failed(lastRequest)); } catch { } }
  };
  const renderer = metldrCreateAnswerRenderer({
    insert: html => Idiomorph.morph(document.body, html, { morphStyle: "innerHTML" }),
    onRendered: complete => { finalDocument = complete; rendered = true; },
  });
  const propose = (text: string) => {
    if (closed || !followup || followupPending || !finalDocument) return;
    const encoded = session.followup(text);
    if (encoded !== undefined) { followupPending = true; report(encoded); }
  };
  const resize = () => {
    if (resizeFrame || !rendered || closed) return;
    resizeFrame = requestAnimationFrame(() => {
      resizeFrame = 0;
      if (!closed) report(session.height(Math.ceil(document.body.scrollHeight)));
    });
  };
  const observer = new ResizeObserver(resize);
  observer.observe(document.body);
  window.addEventListener("resize", resize, { signal: abort.signal });
  window.addEventListener("load", resize, { signal: abort.signal });
  document.addEventListener("submit", event => event.preventDefault(), { signal: abort.signal });
  if (followup) window.addEventListener("wheel", event => {
    if (closed || !event.isTrusted || event.defaultPrevented || !event.cancelable || event.ctrlKey || event.metaKey) return;
    const scale = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 40 : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? window.innerHeight : 1;
    let dx = event.deltaX * scale, dy = event.deltaY * scale;
    if (!Number.isFinite(dx) || !Number.isFinite(dy)) return;
    if (event.shiftKey && dx === 0) { dx = dy; dy = 0; }
    const path = event.composedPath().filter((target): target is HTMLElement => target instanceof HTMLElement);
    const root = document.scrollingElement;
    const html = document.documentElement, body = document.body;
    const htmlStyle = getComputedStyle(html), bodyStyle = getComputedStyle(body);
    const propagatesBody = html.localName === "html" && body.parentElement === html
      && htmlStyle.overflowX === "visible" && htmlStyle.overflowY === "visible" && bodyStyle.display !== "none"
      && htmlStyle.contain === "none" && bodyStyle.contain === "none"
      && htmlStyle.contentVisibility === "visible" && bodyStyle.contentVisibility === "visible";
    const viewportStyle = propagatesBody ? bodyStyle : htmlStyle;
    if (root instanceof HTMLElement && !path.includes(root)) path.push(root);
    // shortcut: native events lose any delta past a boundary; revisit if the browser flow needs that remainder.
    const route = (axis: "x" | "y", delta: number) => {
      if (delta === 0) return "blocked";
      for (const element of path) {
        // Propagated body overflow belongs to the viewport, not a second scroll container.
        if (propagatesBody && element === body && element !== root) continue;
        const style = getComputedStyle(element);
        const viewport = element === root;
        const overflowStyle = viewport ? viewportStyle : style;
        const overflow = axis === "x" ? overflowStyle.overflowX : overflowStyle.overflowY;
        const userScrollable = viewport ? overflow !== "hidden" && overflow !== "clip" : overflow === "auto" || overflow === "scroll";
        if (!viewport && !userScrollable && overflow !== "hidden") continue;
        const maximum = axis === "x" ? element.scrollWidth - element.clientWidth : element.scrollHeight - element.clientHeight;
        const position = axis === "x"
          ? element.scrollLeft + (overflowStyle.direction === "rtl" ? maximum : 0)
          : element.scrollTop;
        if (userScrollable && maximum > 0 && (delta > 0 ? position < maximum - 1 : position > 1)) return "native";
        const boundaryStyle = viewport ? htmlStyle : style;
        const overscroll = axis === "x" ? boundaryStyle.overscrollBehaviorX : boundaryStyle.overscrollBehaviorY;
        if (overscroll === "contain" || overscroll === "none") return "blocked";
      }
      return "relay";
    };
    const x = route("x", dx), y = route("y", dy);
    if (x !== "native" && y !== "native") event.preventDefault();
    const relayX = x === "relay" ? dx : 0, relayY = y === "relay" ? dy : 0;
    if (relayX !== 0 || relayY !== 0) report(session.wheel(relayX, relayY));
  }, { passive: false, signal: abort.signal });

  const close = () => {
    if (closed) return;
    closed = true;
    abort.abort();
    observer.disconnect();
    cancelAnimationFrame(resizeFrame);
    projection?.close();
    session.free();
  };
  window.addEventListener("pagehide", close, { once: true, signal: abort.signal });
  return {
    ready: () => report(session.ready()), close,
    async execute(encoded: string) {
      if (running || closed) throw new Error("Answer is unavailable.");
      const command = session.receive(encoded);
      lastRequest = command.request_id();
      running = true;
      try {
        const kind = command.kind();
        switch (kind) {
          case GuestCommandKind.Preview:
            projection?.close(); projection = undefined;
            renderer.updateDocument(command.html(), command.css(), false);
            break;
          case GuestCommandKind.Document: {
            projection?.close(); projection = undefined;
            const scripts = renderer.updateDocument(command.html(), command.css(), true);
            const documentId = command.request_id();
            projection = new AnswerProjection({ session, initialization, module,
              documentId, scripts, followup, onFollowup: propose, onFailure: reason => {
                if (closed) return;
                report(session.runtime_failed(documentId, reason));
                onRuntimeFailure(reason);
              } });
            await projection.run({ kind: "document" });
            break;
          }
          case GuestCommandKind.Functions:
          case GuestCommandKind.Expression:
            if (!finalDocument || !projection) throw new Error("Answer document is not ready.");
            await projection.run({ kind: kind === GuestCommandKind.Functions ? "functions" : "expression", code: command.code() });
            break;
          case GuestCommandKind.FollowupConsumed:
            followupPending = false;
            if (projection) await projection.run({ kind: "consume" });
            break;
          default: { const exhaustive: never = kind; throw new Error(String(exhaustive)); }
        }
        if (!closed) report(session.ack(command.request_id()));
      } catch (error) { projection?.close(); failed(); throw error; }
      finally { running = false; command.free(); }
    },
  };
}

const saved = document.getElementById("metldr-answer-program");
function savedFailure(text: string) {
  const error = document.createElement("p");
  error.setAttribute("role", "alert");
  error.textContent = text;
  document.body.replaceChildren(error);
}
if (saved) {
  const encoded = saved.textContent ?? "";
  saved.remove();
  void (async () => {
    let program: GuestProgram | undefined;
    let answer: ReturnType<typeof start> | undefined;
    let runtimeFailure: AnswerFailureKind | undefined;
    try {
      program = new GuestProgram(encoded);
      answer = start(program.initialization(), () => {}, false, reason => {
        runtimeFailure = reason;
        answer?.close();
        savedFailure(reason === AnswerFailureKind.Timeout ? "The saved answer stopped responding." : "The saved answer could not finish its display.");
      });
      for (;;) { const command = program.next(); if (command === undefined) break; await answer.execute(command); }
    } catch {
      answer?.close();
      if (runtimeFailure === undefined) savedFailure("The saved answer could not be opened.");
    } finally { program?.free(); }
  })();
} else {
  let initialized = false;
  window.addEventListener("message", (event: MessageEvent<unknown>) => {
    if (initialized || event.source !== window.parent || event.ports.length !== 1 || typeof event.data !== "string") return;
    const port = event.ports[0];
    if (!port) return;
    let answer: ReturnType<typeof start>;
    try { answer = start(event.data, encoded => port.postMessage(encoded), true, () => {}); }
    catch { return; }
    initialized = true;
    port.onmessage = (incoming: MessageEvent<unknown>) => {
      if (typeof incoming.data === "string") void answer.execute(incoming.data).catch(() => {});
    };
    window.addEventListener("pagehide", () => { answer.close(); port.close(); }, { once: true });
    port.start();
    answer.ready();
  });
}
