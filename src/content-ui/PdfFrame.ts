// The PDF tools over a PDF tab: an extension frame (Rust and Rune) in the bottom right corner. The
// frame reads the PDF and talks to the extension itself; this script only places it at the size it
// asks for.
// The frame's size before it reports one: the bar and the room for its shadow.
const SIZE = [372, 72]

export function showPdfTools(): void {
  const frame = document.createElement('iframe')
  frame.setAttribute('aria-label', 'metldr PDF tools')
  // The frame's address is the extension's dynamic one, so the default 'src' list never matches it.
  frame.setAttribute('allow', 'clipboard-write *')
  // The viewer's own sheet places every frame in its document at the top left, full size.
  frame.style.cssText = [
    'position:fixed', 'top:auto', 'left:auto', 'right:8px', 'bottom:8px', `width:${SIZE[0]}px`, `height:${SIZE[1]}px`,
    'border:0', 'margin:0', 'padding:0', 'background:transparent', 'color-scheme:normal', 'z-index:2147483646'
  ].join(';')
  frame.src = chrome.runtime.getURL('inpage/pdf.html')
  window.addEventListener('message', event => {
    // The browser sets the origin, so a page script cannot pass for the frame.
    const fromFrame = event.source === frame.contentWindow && event.origin === `chrome-extension://${chrome.runtime.id}`
    const size = fromFrame && event.data?.metldr === 'pdf' ? event.data.size : null
    if (!Array.isArray(size)) return
    frame.style.width = `${Number(size[0]) || SIZE[0]}px`
    frame.style.height = `${Number(size[1]) || SIZE[1]}px`
  })
  document.documentElement.appendChild(frame)
}
