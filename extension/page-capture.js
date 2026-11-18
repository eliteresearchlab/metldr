export function capturePage(find) {
  // Chrome serializes this function without its module scope: everything it uses lives inside.
  // It returns the page's main content as blocks, [kind, depth, text]; Rust renders the markdown.
  // Given words to find instead, it selects their first match and scrolls it into view.
  if (typeof find === "string") {
    const words = find.replace(/\s+/gu, " ").trim();
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const at = node.data.replace(/\s+/gu, " ").indexOf(words.slice(0, 40));
      if (at < 0) continue;
      const range = document.createRange();
      range.setStart(node, at);
      range.setEnd(node, Math.min(node.data.length, at + words.length));
      getSelection().removeAllRanges();
      getSelection().addRange(range);
      node.parentElement?.scrollIntoView({ block: "center" });
      return { found: true };
    }
    return { found: false };
  }
  const BLOCK_LIMIT = 4000;
  const TEXT_LIMIT = 65536;
  const SKIP = "script,style,noscript,template,svg,canvas,iframe,object,embed,video,audio,form,fieldset,label:has(input,textarea,select),input,textarea,select,button,nav,aside,dialog,[hidden],[inert],[aria-hidden='true'],[role='navigation'],[role='banner'],[role='contentinfo'],[role='complementary'],[role='search'],[role='dialog'],[role='textbox']";
  const CLUTTER = /(?:^|[\s_-])(share|sharing|social|comments?|related|promo|advert|advertisement|ads|sponsor(?:ed)?|cookie|consent|newsletter|subscribe|sidebar|breadcrumbs?|popup|modal|pagination|skip-link)(?:$|[\s_-])/i;
  const HEADING = { H1: "h1", H2: "h2", H3: "h3", H4: "h4", H5: "h5", H6: "h6" };
  const PARAGRAPH = new Set(["P", "DT", "DD", "FIGCAPTION", "SUMMARY", "CAPTION", "ADDRESS"]);
  const LISTS = new Set(["UL", "OL", "MENU"]);
  // Tags that are inline unless a stylesheet says otherwise; known without a style lookup.
  const INLINE = new Set(["A", "SPAN", "EM", "STRONG", "B", "I", "U", "S", "CODE", "KBD", "SAMP", "VAR", "SMALL", "SUB", "SUP", "MARK", "ABBR", "TIME", "Q", "CITE", "DFN", "LABEL", "DATA", "BDI", "BDO", "INS", "DEL", "WBR"]);
  const blocks = [];
  let size = 0;
  let truncated = false;
  let pending = "";

  const skipped = (element) => {
    if (element.matches(SKIP) || element.isContentEditable) return true;
    // A site header or footer is chrome; a header that holds the page's h1 is its title.
    if (element.tagName === "FOOTER" && !element.closest("article")) return true;
    const header = element.tagName === "HEADER";
    if (header && !element.closest("article") && !element.querySelector("h1")) return true;
    const label = `${element.id} ${typeof element.className === "string" ? element.className : ""}`;
    if (label.length > 1 && CLUTTER.test(label)) return true;
    return !element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
  };
  const clean = (text) => text.replace(/\s+/gu, " ").trim();
  const push = (kind, depth, text) => {
    if (!text || truncated) return;
    if (blocks.length >= BLOCK_LIMIT || size + text.length > TEXT_LIMIT) { truncated = true; return; }
    size += text.length;
    blocks.push([kind, depth, text]);
  };
  const flush = () => { push("p", 0, clean(pending)); pending = ""; };
  // The visible text inside an element, without nested lists (they become their own blocks).
  const inline = (element) => {
    let text = "";
    for (const node of element.childNodes) {
      if (node.nodeType === Node.TEXT_NODE) text += node.data;
      else if (node.nodeType === Node.ELEMENT_NODE && !skipped(node) && !LISTS.has(node.tagName)) {
        text += node.tagName === "BR" ? " " : ` ${inline(node)} `;
      }
    }
    return text;
  };
  const walk = (element, depth) => {
    for (const node of element.childNodes) {
      if (truncated) return;
      if (node.nodeType === Node.TEXT_NODE) { pending += node.data; continue; }
      if (node.nodeType !== Node.ELEMENT_NODE || skipped(node)) continue;
      const tag = node.tagName;
      if (HEADING[tag]) { flush(); push(HEADING[tag], 0, clean(inline(node))); }
      else if (PARAGRAPH.has(tag)) { flush(); push("p", 0, clean(inline(node))); }
      else if (tag === "LI") {
        flush();
        push("li", depth + 1, clean(inline(node)));
        for (const list of node.children) if (LISTS.has(list.tagName) && !skipped(list)) walk(list, depth + 1);
      }
      else if (tag === "PRE") { flush(); push("pre", 0, node.innerText.trimEnd()); }
      else if (tag === "BLOCKQUOTE") { flush(); push("quote", 0, clean(inline(node))); }
      // A row holding a table is page layout, not data: read inside it.
      else if (tag === "TR" && node.querySelector("table")) { flush(); walk(node, depth); flush(); }
      else if (tag === "TR") {
        flush();
        const cells = [...node.cells].map((cell) => clean(inline(cell))).filter(Boolean);
        push("row", 0, cells.join(" | "));
      }
      else if (tag === "IMG" || tag === "PICTURE") continue;
      else if (INLINE.has(tag) || getComputedStyle(node).display.startsWith("inline")) {
        pending += ` ${inline(node)} `;
      }
      else { flush(); walk(node, depth); flush(); }
    }
  };

  // The main content: the page's one article; a page of several articles (an index) is read whole.
  const articles = [...document.getElementsByTagName("article")].slice(0, 64);
  const visible = articles.filter((article) => !skipped(article));
  const main = document.querySelector("main,[role='main']") ?? document.body;
  const root = visible.length === 1 ? visible[0] : main;
  if (!root) throw new Error("Page content is unavailable.");
  walk(root, 0);
  flush();
  if (!blocks.length) throw new Error("Page content is unavailable.");
  // A page may offer its own markdown; the extension checks and fetches it.
  const offered = document.querySelector("link[rel~='alternate'][type='text/markdown']")?.href ?? "";
  const alternate = offered.startsWith("https://") ? offered.slice(0, 2048) : "";
  return { url: location.href, title: document.title, blocks, truncated, alternate };
}
