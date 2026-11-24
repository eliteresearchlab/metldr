import type { ContextData } from '../types'

// The word popup: a double-clicked word looked up in an extension frame over the page. The frame
// draws it (Rust and Rune, under the extension's policy on every site) and pulls its job through
// the extension; this script reads the selection, places the frame and closes it.

const LANGUAGE_SUBDOMAIN_REGEX = /^([a-z]{2})\./
const WHITESPACE_REGEX = /\s/
const VALID_WORD_REGEX = /^[\p{L}\p{M}'-]+$/u
const SENTENCE_END_REGEX = /[.!?\n]/
const EDITABLE_TAGS = ['input', 'textarea', 'select']
// The frame's room around the card for its shadow, as the frame draws it (metldr-inpage MARGIN).
const MARGIN = 16
// A double-click waits this long before showing, so a triple-click selection is not a lookup.
const SETTLE_MS = 250
// A hidden frame keeps its wasm instance and GPU device; one unused this long is removed.
const IDLE_MS = 60_000

interface Job {
  word: string
  context: ContextData
}

export class WordFrame {
  private frame: HTMLIFrameElement | null = null
  private job: Job | null = null
  private anchor: Range | null = null
  private size: [number, number] = [352, 160]
  private shown = false
  private pending: ReturnType<typeof setTimeout> | null = null
  private idle: ReturnType<typeof setTimeout> | null = null
  private lastDoubleClick = 0
  // Every listener this script adds, removed together when it retires.
  private readonly listening = new AbortController()
  private readonly answerPull = (message: { type?: string }, _sender: unknown, respond: (job: Job | null) => void): boolean => {
    if (message?.type !== 'INPAGE_PULL') return false
    respond(this.job)
    return false
  }

  constructor() {
    const signal = this.listening.signal
    document.addEventListener('dblclick', event => this.handleTextSelection(event), { signal })
    document.addEventListener('click', event => {
      if (Date.now() - this.lastDoubleClick < 300 && this.pending) this.cancelPending()
      else if (this.shown && event.target !== this.frame) this.hide()
    }, { signal })
    document.addEventListener('keydown', event => { if (event.key === 'Escape' && this.shown) this.hide() }, { signal })
    window.addEventListener('scroll', () => this.place(), { passive: true, capture: true, signal })
    window.addEventListener('resize', () => this.place(), { signal })
    // Only this frame is heard: it says the size it needs and when to close.
    window.addEventListener('message', event => {
      // The browser sets the origin, so a page script cannot pass for the frame.
      if (!this.frame || event.origin !== `chrome-extension://${chrome.runtime.id}` || event.data?.metldr !== 'word') return
      if (Array.isArray(event.data.size)) {
        this.size = [Number(event.data.size[0]) || this.size[0], Number(event.data.size[1]) || this.size[1]]
        this.place()
      }
      if (event.data.close) this.hide()
    }, { signal })
    // The frame asks the extension for its job, and the extension asks this script.
    chrome.runtime.onMessage.addListener(this.answerPull)
  }

  // The extension was updated and its new script took over this tab: this one stops listening
  // and takes its frame away.
  retire(): void {
    this.listening.abort()
    chrome.runtime.onMessage.removeListener(this.answerPull)
    this.cancelPending()
    if (this.idle) clearTimeout(this.idle)
    this.frame?.remove()
    this.frame = null
  }

  private pageLanguage(): string | undefined {
    const declared = document.documentElement.lang?.split('-')[0]?.toLowerCase()
    if (declared && declared.length === 2) return declared
    return window.location.hostname.match(LANGUAGE_SUBDOMAIN_REGEX)?.[1]
  }

  private cancelPending(): void {
    if (this.pending) clearTimeout(this.pending)
    this.pending = null
  }

  private handleTextSelection(event: MouseEvent): void {
    this.lastDoubleClick = Date.now()
    this.cancelPending()
    const target = event.target as HTMLElement
    for (let node: HTMLElement | null = target; node; node = node.parentElement) {
      if (node.isContentEditable || EDITABLE_TAGS.includes(node.tagName?.toLowerCase())) return
    }
    const selection = window.getSelection()
    const word = selection?.toString().trim() || ''
    if (!word || WHITESPACE_REGEX.test(word) || word.length > 30 || word.length < 2 || !VALID_WORD_REGEX.test(word)) {
      return
    }
    if (!selection || selection.rangeCount === 0) return
    const range = selection.getRangeAt(0)
    const text = range.commonAncestorContainer.textContent || ''
    const offset = range.startOffset
    let start = 0
    for (let i = offset; i > 0; i--) {
      if (SENTENCE_END_REGEX.test(text[i])) { start = i + 1; break }
    }
    let end = text.length
    for (let i = offset + word.length; i < text.length; i++) {
      if (SENTENCE_END_REGEX.test(text[i])) { end = i + 1; break }
    }
    const context: ContextData = {
      contextBefore: text.substring(start, offset).trim(),
      contextAfter: text.substring(offset + word.length, end).trim(),
      fullSentence: text.substring(start, end).trim(),
      pageLang: this.pageLanguage()
    }
    const anchor = range.cloneRange()
    // The frame starts while the double-click settles; it gets the job only once it has.
    this.ensureFrame()
    this.pending = setTimeout(() => {
      this.pending = null
      this.show({ word, context }, anchor)
    }, SETTLE_MS)
  }

  private ensureFrame(): HTMLIFrameElement {
    if (this.idle) clearTimeout(this.idle)
    this.idle = null
    if (this.frame?.isConnected) return this.frame
    const frame = document.createElement('iframe')
    frame.setAttribute('aria-label', 'metldr word lookup')
    frame.style.cssText = `position:fixed;top:0;left:0;width:${this.size[0]}px;height:${this.size[1]}px;`
      + 'border:0;margin:0;padding:0;background:transparent;color-scheme:normal;z-index:2147483646;visibility:hidden'
    frame.setAttribute('allowtransparency', 'true')
    frame.src = chrome.runtime.getURL('inpage/word.html')
    document.documentElement.appendChild(frame)
    this.frame = frame
    return frame
  }

  private async show(job: Job, anchor: Range): Promise<void> {
    const settings = await chrome.storage.local.get('wordPopupEnabled') as { wordPopupEnabled?: boolean }
    if (settings.wordPopupEnabled === false) return
    this.job = job
    this.anchor = anchor
    const frame = this.ensureFrame()
    // A frame that already ran pulls the new job; a new one pulls it as it starts.
    frame.contentWindow?.postMessage({ metldr: 'next' }, '*')
    this.shown = true
    this.place()
    frame.style.visibility = 'visible'
  }

  private hide(): void {
    this.shown = false
    this.job = null
    this.anchor = null
    if (!this.frame) return
    this.frame.style.visibility = 'hidden'
    // The next word must not open on this one's card.
    this.frame.contentWindow?.postMessage({ metldr: 'hide' }, '*')
    if (this.idle) clearTimeout(this.idle)
    this.idle = setTimeout(() => {
      this.idle = null
      if (this.shown || this.pending) return
      this.frame?.remove()
      this.frame = null
    }, IDLE_MS)
  }

  // Below the word, centred on it, inside the viewport; above it when there is no room below.
  private place(): void {
    if (!this.shown || !this.frame || !this.anchor) return
    const word = this.anchor.getBoundingClientRect()
    const [width, height] = this.size
    let x = word.left + word.width / 2 - width / 2
    let y = word.bottom + 12 - MARGIN
    x = Math.min(Math.max(x, 10 - MARGIN), window.innerWidth - width + MARGIN - 10)
    if (y + height - MARGIN > window.innerHeight - 10) y = word.top - height + MARGIN - 8
    y = Math.max(y, 10 - MARGIN)
    Object.assign(this.frame.style, { left: `${x}px`, top: `${y}px`, width: `${width}px`, height: `${height}px` })
  }
}
