// The PDF tools' frame over a PDF tab. The extension names the tab's PDF, never the page around
// the frame; this page reads it, has the extension summarize its text, and tells the page only
// the size the tools need.
import init, { inpage_theme, pdf_copied, pdf_phase, start_pdf } from '/chat/rune/metldr_panel.js'
import { readPdf, readPdfAt } from './read-pdf.js'

const post = message => parent.postMessage({ metldr: 'pdf', ...message }, '*')
const phase = report => pdf_phase(JSON.stringify(report))
const failed = error => phase({ phase: 'failed', error })
const SCANNED = 'This PDF has no text to read. Scanned pages need text recognition, which is not available yet.'
let tab = null
let bullets = []

async function fonts() {
  const manifest = await (await fetch('/chat/fonts/manifest.json')).json()
  const read = async name => new Uint8Array(await (await fetch(`/chat/fonts/${name}`)).arrayBuffer())
  return Promise.all(manifest.fonts.map(read))
}

// The text's summary from the extension's page summary pipeline: cached per PDF, counted in stats.
async function summarize(text) {
  if (!text) return failed(SCANNED)
  phase({ phase: 'summarizing' })
  const request = { type: 'EXTRACT_AND_SUMMARIZE', trigger: 'manual', text }
  const reply = await chrome.runtime.sendMessage(request).catch(error => ({ error: error.message }))
  bullets = reply?.success ? reply.summary?.bullets ?? [] : []
  if (bullets.length) return phase({ phase: 'summary', bullets })
  failed(reply?.error || (reply?.skip ? 'This PDF has too little text to summarize.' : 'The summary failed.'))
}

async function run(read) {
  phase({ phase: 'reading' })
  try {
    await summarize(await read())
  } catch (error) {
    failed(error?.message || 'This PDF could not be read.')
  }
}

// A PDF on this computer is the reader's to hand over: only a file they choose can be read.
const picker = document.createElement('input')
Object.assign(picker, { type: 'file', accept: 'application/pdf,.pdf', hidden: true })
picker.addEventListener('change', () => {
  const file = picker.files[0]
  picker.value = ''
  if (file) run(async () => readPdf(new Uint8Array(await file.arrayBuffer())))
})
document.body.append(picker)

const actions = {
  summarize() {
    if (!tab?.url) return
    if (tab.url.startsWith('file:')) phase({ phase: 'choose' })
    else run(() => readPdfAt(tab.url))
  },
  choose: () => picker.click(),
  async copy() {
    const text = bullets.map(point => `• ${point}`).join('\n')
    try {
      await navigator.clipboard.writeText(text)
      pdf_copied()
    } catch {
      failed('The summary could not be copied.')
    }
  },
  async chat() {
    const reply = await chrome.runtime.sendMessage({ type: 'TOGGLE_SIDE_PANEL', focus: 'chat' }).catch(() => null)
    if (reply?.success === false) failed(reply.error || 'The chat could not open.')
  }
}

const [files, stored, pulled] = await Promise.all([
  fonts(),
  chrome.storage.local.get('theme'),
  chrome.runtime.sendMessage({ type: 'INPAGE_PULL' }).catch(() => null),
  init()
])
tab = pulled
const sized = (width, height) => post({ size: [width, height] })
start_pdf(files, stored.theme ?? 'catppuccin', sized, name => actions[name]?.())
chrome.storage.onChanged.addListener((changes, area) => {
  const theme = area === 'local' ? changes.theme?.newValue : undefined
  if (typeof theme === 'string') inpage_theme(theme)
})
