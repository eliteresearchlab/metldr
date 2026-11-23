// The word popup's frame. It takes each job from the extension, never from the page around it,
// and tells the page only the size it needs and when to close.
// The chat's Rune build, so the browser compiles it once for the panel and every popup.
import init, { inpage_theme, start_word, word_result, word_show } from '/chat/rune/metldr_panel.js'

const post = message => parent.postMessage({ metldr: 'word', ...message }, '*')

async function fonts() {
  const manifest = await (await fetch('/chat/fonts/manifest.json')).json()
  const read = async name => new Uint8Array(await (await fetch(`/chat/fonts/${name}`)).arrayBuffer())
  return Promise.all(manifest.fonts.map(read))
}

// The current job, read from the content script that opened this frame through the extension.
async function pull() {
  const job = await chrome.runtime.sendMessage({ type: 'INPAGE_PULL' }).catch(() => null)
  if (!job?.word) return
  word_show(job.word)
  const answer = await chrome.runtime
    .sendMessage({ type: 'WORD_LOOKUP', word: job.word, lookupType: 'definition', context: job.context })
    .catch(() => null)
  word_result(JSON.stringify(answer ?? {}))
}

const [files, stored] = await Promise.all([fonts(), chrome.storage.local.get('theme'), init()])
start_word(files, stored.theme ?? 'catppuccin', (width, height) => post({ size: [width, height] }), () => post({ close: true }))
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && typeof changes.theme?.newValue === 'string') inpage_theme(changes.theme.newValue)
})
// The page's nudges carry no data: the frame pulls its next job itself, and on hide drops the
// last word so the next one never opens on it.
addEventListener('message', event => {
  if (event.source !== parent) return
  if (event.data?.metldr === 'next') pull()
  else if (event.data?.metldr === 'hide') word_show('')
})
pull()
