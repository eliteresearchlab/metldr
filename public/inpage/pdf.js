// The PDF tools' frame over a PDF tab. The extension names the tab's PDF, never the page around
// the frame; this page reads it, has the extension summarize its text, and tells the page only
// the size the tools need.
import init, { inpage_theme, pdf_phase, start_pdf } from '/chat/rune/metldr_panel.js'
import { readPdf, readPdfAt } from './read-pdf.js'

const post = message => parent.postMessage({ metldr: 'pdf', ...message }, '*')
const phase = report => pdf_phase(JSON.stringify(report))
let tab = null
let bullets = []

async function fonts() {
  const manifest = await (await fetch('/chat/fonts/manifest.json')).json()
  const read = async name => new Uint8Array(await (await fetch(`/chat/fonts/${name}`)).arrayBuffer())
  return Promise.all(manifest.fonts.map(read))
}

// The text's summary from the extension's page summary pipeline: cached per PDF, counted in stats.
async function summarize(text) {
  if (!text) return phase({ phase: 'failed', error: 'This PDF has no text to read. Scanned pages need text recognition, which is not available yet.' })
  phase({ phase: 'summarizing' })
  const reply = await chrome.runtime.sendMessage({ type: 'EXTRACT_AND_SUMMARIZE', trigger: 'manual', text }).catch(() => null)
  bullets = reply?.success ? reply.summary?.bullets ?? [] : []
  if (bullets.length) phase({ phase: 'summary', bullets })
  else phase({ phase: 'failed', error: reply?.error || (reply?.skip ? 'This PDF has too little text to summarize.' : 'The summary failed.') })
}

async function run(read) {
  phase({ phase: 'reading' })
  try {
    await summarize(await read())
  } catch (error) {
    phase({ phase: 'failed', error: error?.message || 'This PDF could not be read.' })
  }
}

// A PDF on this computer is the reader's to hand over: only a file they choose can be read.
const picker = Object.assign(document.createElement('input'), { type: 'file', accept: 'application/pdf,.pdf', hidden: true })
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
  copy: () => navigator.clipboard.writeText(bullets.map(point => `• ${point}`).join('\n')).catch(() => {}),
  chat: () => chrome.runtime.sendMessage({ type: 'TOGGLE_SIDE_PANEL', focus: 'chat' }).catch(() => {})
}

const [files, stored, pulled] = await Promise.all([
  fonts(),
  chrome.storage.local.get('theme'),
  chrome.runtime.sendMessage({ type: 'INPAGE_PULL' }).catch(() => null),
  init()
])
tab = pulled
start_pdf(files, stored.theme ?? 'catppuccin', (width, height) => post({ size: [width, height] }), name => actions[name]?.())
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && typeof changes.theme?.newValue === 'string') inpage_theme(changes.theme.newValue)
})
