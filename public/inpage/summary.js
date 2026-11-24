// A Gmail thread's summary card, inside the thread. It takes the thread from the extension, never
// from the page around it, and tells the page only the height it needs.
import init, { inpage_theme, start_summary, summary_phase } from '/chat/rune/metldr_panel.js'

const post = message => parent.postMessage({ metldr: 'summary', ...message }, '*')
const phase = report => summary_phase(JSON.stringify(report))
let job = null
let asked = 0

async function fonts() {
  const manifest = await (await fetch('/chat/fonts/manifest.json')).json()
  const read = async name => new Uint8Array(await (await fetch(`/chat/fonts/${name}`)).arrayBuffer())
  return Promise.all(manifest.fonts.map(read))
}

// A summary from the extension: the kept one unless `fresh`, else written now and kept.
async function summarize(fresh) {
  if (!job) return
  const ask = ++asked
  phase({ phase: 'summarizing' })
  const request = { type: 'SUMMARIZE_EMAIL', emailContent: job.content, emailId: job.thread, metadata: job.metadata, forceRegenerate: fresh }
  const reply = await chrome.runtime.sendMessage(request).catch(error => ({ error: error.message }))
  if (ask !== asked) return
  if (reply?.summary && !reply.error) phase({ phase: 'ready', summary: reply.summary })
  else phase({ phase: 'failed', error: reply?.error || 'The summary failed.' })
}

// A kept summary shows at once; one written before the thread grew is written again.
async function open() {
  const ask = ++asked
  const kept = await chrome.runtime.sendMessage({ type: 'GET_EMAIL_CACHE', emailId: job.thread }).catch(() => null)
  if (ask !== asked) return
  if (!kept?.cached) return phase({ phase: 'offer' })
  const grew = kept.emailCount > 0 && job.count > 0 && kept.emailCount !== job.count
  if (grew) summarize(true)
  else phase({ phase: 'ready', summary: kept.cached })
}

// The current job, read from the Gmail tab's script through the extension.
async function pull() {
  const next = await chrome.runtime.sendMessage({ type: 'INPAGE_PULL' }).catch(() => null)
  if (!next?.thread || typeof next.content !== 'string') return
  job = next
  open()
}

const actions = { summarize: () => summarize(false), again: () => summarize(true) }
const [files, stored] = await Promise.all([fonts(), chrome.storage.local.get('theme'), init()])
start_summary(files, stored.theme ?? 'catppuccin', (_width, height) => post({ height }), name => actions[name]?.())
chrome.storage.onChanged.addListener((changes, area) => {
  const theme = area === 'local' ? changes.theme?.newValue : undefined
  if (typeof theme === 'string') inpage_theme(theme)
})
// The page's nudges carry no data: the frame pulls its next job itself.
addEventListener('message', event => {
  if (event.source === parent && event.data?.metldr === 'next') pull()
})
pull()
