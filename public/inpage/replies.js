// The reply suggestions' frame beside a Gmail reply. It takes the thread from the extension, never
// from the page around it, and tells the page only the size it needs, when to close, and the reply
// the reader picked.
import init, { inpage_theme, replies_phase, start_replies } from '/chat/rune/metldr_panel.js'

const post = message => parent.postMessage({ metldr: 'replies', ...message }, '*')
const phase = report => replies_phase(JSON.stringify(report))
let thread = null
let suggestions = []
let asked = 0

async function fonts() {
  const manifest = await (await fetch('/chat/fonts/manifest.json')).json()
  const read = async name => new Uint8Array(await (await fetch(`/chat/fonts/${name}`)).arrayBuffer())
  return Promise.all(manifest.fonts.map(read))
}

// The thread's replies: the kept ones, or new ones the extension writes from the thread.
async function load(fresh) {
  if (!thread) return
  const ask = ++asked
  phase({ phase: 'writing' })
  const request = { type: 'GENERATE_REPLY_SUGGESTIONS', emailId: thread, forceRegenerate: fresh }
  const reply = await chrome.runtime.sendMessage(request).catch(error => ({ error: error.message }))
  // A newer thread was asked for while this one was written.
  if (ask !== asked) return
  suggestions = (reply?.suggestions ?? []).filter(s => typeof s?.body === 'string' && s.body.trim().length > 10)
  if (suggestions.length) phase({ phase: 'ready', suggestions })
  else if (reply?.error && reply.error !== 'no valid suggestions generated') phase({ phase: 'failed', error: reply.error })
  else phase({ phase: 'empty' })
}

// The current job, read from the Gmail tab's script through the extension.
async function pull() {
  const job = await chrome.runtime.sendMessage({ type: 'INPAGE_PULL' }).catch(() => null)
  if (!job?.thread || job.thread === thread) return
  thread = job.thread
  suggestions = []
  load(false)
}

const actions = name => {
  if (name === 'retry') return load(true)
  const index = Number(name.split(':')[1])
  const picked = suggestions[index]
  if (name.startsWith('insert:') && picked) post({ insert: picked.body })
}

const [files, stored] = await Promise.all([fonts(), chrome.storage.local.get('theme'), init()])
const sized = (width, height) => post({ size: [width, height] })
start_replies(files, stored.theme ?? 'catppuccin', sized, () => post({ close: true }), actions)
chrome.storage.onChanged.addListener((changes, area) => {
  const theme = area === 'local' ? changes.theme?.newValue : undefined
  if (typeof theme === 'string') inpage_theme(theme)
})
// The page's nudges carry no data: the frame pulls its next job itself.
addEventListener('message', event => {
  if (event.source === parent && event.data?.metldr === 'next') pull()
})
pull()
