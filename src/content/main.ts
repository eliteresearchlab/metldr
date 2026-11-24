import { WordFrame } from '../content-ui/WordFrame'
import { showPdfTools } from '../content-ui/PdfFrame'
import { ArticleExtractor } from '../content-ui/ArticleExtractor'

// The content script of every page but Gmail's, which has its own (gmail.ts): the word popup's
// frame, the PDF tools over a PDF tab and the article the page tools read.

const isPdf = document.contentType === 'application/pdf'

chrome.runtime.onMessage.addListener((msg, _sender, respond) => {
  if (msg?.type !== 'EXTRACT_ARTICLE') return false
  // The extension page asking reads a PDF's file itself.
  if (isPdf) {
    respond({ success: true, data: { skip: true, reason: 'pdf' } })
    return false
  }
  try {
    respond({ success: true, data: ArticleExtractor.extract() })
  } catch (err) {
    console.error('metldr: extraction failed:', err)
    respond({ success: false, error: (err as Error).message })
  }
  return false
})

// A PDF tab shows the browser's viewer: the PDF tools sit over it, and nothing reads the page.
if (isPdf) {
  showPdfTools()
} else {
  const words = new WordFrame()
  document.addEventListener('dblclick', event => words.handleTextSelection(event))
}
