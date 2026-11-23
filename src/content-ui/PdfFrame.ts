// The PDF tools over a PDF tab: an extension frame (Rust and Rune) in the bottom right corner. The
// frame reads the PDF and talks to the extension itself; this script only places it at the size it
// asks for.
export function showPdfTools(): void {
  const frame = document.createElement('iframe')
  frame.setAttribute('aria-label', 'metldr PDF tools')
  frame.setAttribute('allow', 'clipboard-write')
  // The viewer's own sheet places every frame in its document at the top left, full size.
  frame.style.cssText = 'position:fixed;top:auto;left:auto;right:8px;bottom:8px;width:372px;height:72px;border:0;margin:0;'
    + 'padding:0;background:transparent;color-scheme:normal;z-index:2147483646'
  frame.src = chrome.runtime.getURL('inpage/pdf.html')
  window.addEventListener('message', event => {
    // The browser sets the origin, so a page script cannot pass for the frame.
    if (event.source !== frame.contentWindow || event.origin !== `chrome-extension://${chrome.runtime.id}`) return
    const size = event.data?.metldr === 'pdf' ? event.data.size : null
    if (Array.isArray(size)) Object.assign(frame.style, { width: `${Number(size[0]) || 372}px`, height: `${Number(size[1]) || 72}px` })
  })
  document.documentElement.appendChild(frame)
}
