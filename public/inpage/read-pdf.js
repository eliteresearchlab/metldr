// PDFs are read in the extension page that shows them (the chat, the PDF tools) by the PDF module
// (PDFium, about 8 MB), loaded the first time one is read; a file never crosses to the service
// worker.
let reader

export async function readPdf(bytes) {
  reader ??= import('/chat/pdf/metldr_pdf.js').then(async module => {
    await module.default()
    return module
  }).catch(error => {
    reader = undefined
    throw error
  })
  return (await reader).pdf_read(bytes, 2000)
}

// The PDF at `url`, fetched with the reader's cookies for the site so a signed-in PDF reads too.
export async function readPdfAt(url) {
  const response = await fetch(url, { credentials: 'include' })
  if (!response.ok) throw new Error(`The PDF could not be downloaded (${response.status}).`)
  return readPdf(new Uint8Array(await response.arrayBuffer()))
}
