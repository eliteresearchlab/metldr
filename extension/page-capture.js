export function capturePage(find, about) {
  // Chrome serializes this function without its module scope: everything it uses lives inside.
  // It returns the page's main content as blocks, [kind, depth, text]; Rust renders the markdown.
  // Given words to find instead, it selects their first match and scrolls it into view. With
  // `about`, it also returns what the page tools' summary settings judge a page by (`signals`)
  // and the article's byline (`meta`).
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
  // A PDF tab shows the browser's viewer, not the document; the chat reads the file instead.
  if (document.contentType === "application/pdf") {
    return { title: document.title || decodeURIComponent(location.pathname.split("/").pop() ?? ""), pdf: true };
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
  // A block past the limit keeps the part that fits, so one long block never empties the page.
  const push = (kind, depth, text) => {
    if (!text || truncated) return;
    if (blocks.length >= BLOCK_LIMIT) { truncated = true; return; }
    if (size + text.length > TEXT_LIMIT) {
      truncated = true;
      text = text.slice(0, TEXT_LIMIT - size);
      if (text.length < 200) return;
    }
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
  // Two line breaks in a row end a paragraph (pages laid out with <br> instead of <p>).
  let breaks = 0;
  const walk = (element, depth) => {
    for (const node of element.childNodes) {
      if (truncated) return;
      if (node.nodeType === Node.TEXT_NODE) {
        pending += node.data;
        if (node.data.trim()) breaks = 0;
        continue;
      }
      if (node.nodeType !== Node.ELEMENT_NODE || skipped(node)) continue;
      const tag = node.tagName;
      if (tag === "BR") {
        pending += " ";
        if (++breaks === 2) flush();
        continue;
      }
      if (HEADING[tag]) { flush(); push(HEADING[tag], 0, clean(inline(node))); }
      else if (PARAGRAPH.has(tag)) { flush(); push("p", 0, clean(inline(node))); }
      else if (tag === "LI") {
        flush();
        push("li", depth + 1, clean(inline(node)));
        for (const list of node.children) if (LISTS.has(list.tagName) && !skipped(list)) walk(list, depth + 1);
      }
      else if (tag === "PRE") { flush(); push("pre", 0, node.innerText.trimEnd()); }
      else if (tag === "BLOCKQUOTE") { flush(); push("quote", 0, clean(inline(node))); }
      // A row holding a table, or with one cell, is page layout, not data: read inside it.
      else if (tag === "TR" && (node.cells.length < 2 || node.querySelector("table"))) { flush(); walk(node, depth); flush(); }
      else if (tag === "TR") {
        flush();
        const cells = [...node.cells].map((cell) => clean(inline(cell))).filter(Boolean);
        push("row", 0, cells.join(" | "));
      }
      else if (tag === "IMG" || tag === "PICTURE") continue;
      // Inline text that holds line breaks or blocks is read for its paragraphs.
      else if (INLINE.has(tag) || getComputedStyle(node).display.startsWith("inline")) {
        if (node.querySelector("br,p,div,ul,ol,table,pre,blockquote,h1,h2,h3,h4,h5,h6")) walk(node, depth);
        else pending += ` ${inline(node)} `;
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
  const page = { url: location.href, title: document.title, blocks, truncated, alternate };
  if (!about) return page;
  const count = (selector) => document.querySelectorAll(selector).length;
  const meta = (selector) => document.querySelector(selector)?.content?.trim().slice(0, 200) || null;
  // The page's structured data, when it describes the article.
  let described = {};
  for (const script of document.querySelectorAll("script[type='application/ld+json']")) {
    let data;
    try { data = JSON.parse(script.textContent); } catch { continue; }
    const items = [data, ...(Array.isArray(data) ? data : []), ...(Array.isArray(data?.["@graph"]) ? data["@graph"] : [])];
    const found = items.find((item) => item && typeof item === "object" && (item.author || item.datePublished));
    if (found) { described = found; break; }
  }
  const named = (value) => {
    const name = typeof value === "string" ? value : Array.isArray(value) ? value[0]?.name ?? value[0] : value?.name;
    return typeof name === "string" && name.trim() ? name.trim().slice(0, 200) : null;
  };
  const byline = document.querySelector("[rel='author'],[itemprop='author'],.byline,.author")?.innerText.trim().split("\n")[0].slice(0, 200) || null;
  const body = document.body;
  const text = body.innerText.length || 1;
  const paragraphs = count("p");
  const controls = count("button") + count("input, select, textarea");
  const forms = count("form");
  const drawn = count("canvas") + count("svg");
  let linked = 0;
  for (const link of document.getElementsByTagName("a")) linked += link.innerText.length;
  const path = location.pathname;
  const ratio = paragraphs > 0 ? controls / paragraphs : controls;
  const hashRouting = location.hash.includes("#/");
  page.signals = {
    isSPA: hashRouting || ratio > 1.4 || forms > paragraphs * 0.6,
    isDashboard: /(dashboard|admin|console|panel)/i.test(path),
    isSearch: /search/i.test(path) || /[?&](q|query|search)=/i.test(location.search),
    isCart: /(cart|checkout)/i.test(path),
    isFeed: /(\/feed|\/rss)/i.test(path),
    isSocialFeed: /(twitter\.com|facebook\.com|instagram\.com|tiktok\.com|linkedin\.com)/i.test(location.hostname),
    canvasHeavy: drawn >= 3,
    hasArticleTag: articles.length > 0,
    hasMain: document.querySelector("main") !== null,
    h1Count: count("h1"),
    textDensity: Math.min(1, text / Math.max(body.innerHTML.length, text)),
    linkDensity: linked / text,
    buttonToParagraphRatio: ratio,
  };
  page.meta = {
    author: meta("meta[name='author']") ?? named(described.author) ?? byline,
    publication: meta("meta[property='og:site_name']") ?? meta("meta[name='application-name']") ?? named(described.publisher) ?? location.hostname.replace(/^www\./, ""),
    publishDate: meta("meta[property='article:published_time']") ?? named(described.datePublished) ?? meta("meta[name='date']"),
  };
  return page;
}
