import initPdf, { pdf_read } from '../../../crates/metldr-pdf/pkg/metldr_pdf'
import pdfWasmUrl from '../../../crates/metldr-pdf/pkg/metldr_pdf_bg.wasm?url'
import { summarize } from '../RustCore'
import { ragService } from '../rag/RagService'

// PDFs are read by the Rust PDF module (PDFium), loaded the first time one is read.
const MAX_PAGES = 2000
const LEADING_SLASH_PATTERN = /^\//
let pdfReady: ReturnType<typeof initPdf> | undefined

interface PdfDocument { pages: { markdown: string }[] }

async function read(data: ArrayBuffer | Uint8Array): Promise<string> {
  await (pdfReady ??= initPdf({
    module_or_path: chrome.runtime.getURL(pdfWasmUrl.replace(LEADING_SLASH_PATTERN, ''))
  }).catch(error => {
    pdfReady = undefined
    throw error
  }))
  const document: PdfDocument = JSON.parse(await pdf_read(new Uint8Array(data), MAX_PAGES))
  return document.pages.map(page => page.markdown).filter(Boolean).join('\n\n')
}

async function fetchPdf(url: string): Promise<ArrayBuffer> {
  if (url.startsWith('file://')) throw new Error('LOCAL_PDF_NEEDS_PICKER')
  return (await fetch(url)).arrayBuffer()
}

async function summarizeText(text: string, sourceUrl: string, title: string): Promise<string> {
  const summary = (await summarize({ content: text })).text
  ragService.indexSummary(summary, { sourceId: sourceUrl, sourceUrl, sourceType: 'pdf', title })
  return summary
}

export class PdfService {
  async summarize(url: string): Promise<string> {
    const text = await read(await fetchPdf(url))
    return summarizeText(text, url, url.split('/').pop() || 'PDF Document')
  }

  async summarizeFromArrayBuffer(data: ArrayBuffer, filename?: string, sourceUrl?: string): Promise<{ summary: string; fullText: string }> {
    const fullText = await read(data)
    const url = sourceUrl || `file://${filename || 'document.pdf'}`
    return { summary: await summarizeText(fullText, url, filename || 'PDF Document'), fullText }
  }

  async extractFromUrl(url: string): Promise<string> {
    return read(await fetchPdf(url))
  }

  async extractFromArrayBuffer(data: ArrayBuffer): Promise<string> {
    return read(data)
  }

  // file picker summarize
  openAndSummarize(): Promise<{ summary: string; fullText: string; filename: string }> {
    return new Promise((resolve, reject) => {
      const input = Object.assign(document.createElement('input'), { type: 'file', accept: '.pdf,application/pdf', hidden: true })
      document.body.appendChild(input)
      input.onchange = async () => {
        const file = input.files?.[0]
        input.remove()
        if (!file) return reject(new Error('No file selected'))
        try {
          const result = await this.summarizeFromArrayBuffer(await file.arrayBuffer(), file.name)
          resolve({ ...result, filename: file.name })
        } catch (err) {
          reject(err)
        }
      }
      input.oncancel = () => {
        input.remove()
        reject(Object.assign(new Error('File selection cancelled'), { name: 'AbortError' }))
      }
      input.click()
    })
  }
}

export const pdfService = new PdfService()
