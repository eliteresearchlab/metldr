import { WordFrame } from '../content-ui/WordFrame'
import { showPdfTools } from '../content-ui/PdfFrame'

// The content script of every page but Gmail's, which has its own (gmail.ts): the word popup's
// frame, and the PDF tools over a PDF tab.

// After an update the extension injects this script into tabs already open. The previous
// version's script there is cut off from the extension: it is told to stop, and the frames it
// left go.
document.dispatchEvent(new Event('metldr:retire'))
for (const stale of document.querySelectorAll('iframe[aria-label^="metldr "]')) stale.remove()

// A PDF tab shows the browser's viewer: the PDF tools sit over it.
if (document.contentType === 'application/pdf') {
  showPdfTools()
} else {
  const words = new WordFrame()
  document.addEventListener('metldr:retire', () => words.retire(), { once: true })
}
