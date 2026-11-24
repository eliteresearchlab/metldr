import type { ComposeView, InboxSDK } from '../types'

// Gmail reply suggestions: a button in each reply opens an extension frame (Rust and Rune) beside
// it. The frame asks the extension for the thread's replies; this script places the frame, tells
// it the thread, and puts the reply the reader picks into the reply as plain text.

// The frame's room around the card for its shadow, as the frame draws it (inpage.rs MARGIN).
const MARGIN = 16
const THREAD_ID_HASH_REGEX = /#[^/]+\/(?:[^/]+\/)?([A-Za-z0-9_-]{16,})$/

const threadFromUrl = (): string | null => window.location.hash.match(THREAD_ID_HASH_REGEX)?.[1] ?? null

export class ReplyFrame {
  private frame: HTMLIFrameElement | null = null
  private compose: ComposeView | null = null
  private thread: string | null = null
  private size: [number, number] = [392, 152]
  private shown = false

  init(sdk: InboxSDK): void {
    sdk.Compose.registerComposeViewHandler(view => this.attach(view))
    // The frame asks the extension for its job, and the extension asks this script.
    chrome.runtime.onMessage.addListener((message, _sender, respond) => {
      if (message?.type !== 'INPAGE_PULL' || message.frame !== 'replies') return false
      respond(this.thread ? { thread: this.thread } : null)
      return false
    })
    // Only this frame is heard: the size it needs, when to close and the reply picked.
    window.addEventListener('message', event => {
      const fromFrame = this.frame !== null && event.source === this.frame.contentWindow
        && event.origin === `chrome-extension://${chrome.runtime.id}` && event.data?.metldr === 'replies'
      if (!fromFrame) return
      if (Array.isArray(event.data.size)) {
        this.size = [Number(event.data.size[0]) || this.size[0], Number(event.data.size[1]) || this.size[1]]
        this.place()
      }
      if (typeof event.data.insert === 'string') {
        this.insert(event.data.insert)
        this.hide()
      }
      if (event.data.close) this.hide()
    })
    window.addEventListener('resize', () => this.place())
    window.addEventListener('scroll', () => this.place(), { passive: true, capture: true })
    document.addEventListener('keydown', event => { if (event.key === 'Escape' && this.shown) this.hide() })
  }

  // A reply in a thread gets the button; a new email has no thread to answer.
  private attach(view: ComposeView): void {
    const thread = threadFromUrl() ?? view.getThreadID?.() ?? null
    if (!thread) return
    view.addButton({
      title: 'Reply suggestions',
      iconUrl: chrome.runtime.getURL('assets/logo/metldr-mark-small.svg'),
      orderHint: 100,
      onClick: () => this.toggle(view, thread)
    })
    view.on('destroy', () => { if (this.compose === view) this.hide() })
  }

  private toggle(view: ComposeView, thread: string): void {
    if (this.shown && this.compose === view) return this.hide()
    this.compose = view
    this.thread = thread
    const frame = this.ensureFrame()
    // A frame that already ran pulls the new thread; a new one pulls it as it starts.
    frame.contentWindow?.postMessage({ metldr: 'next' }, '*')
    this.shown = true
    this.place()
    frame.style.visibility = 'visible'
  }

  private ensureFrame(): HTMLIFrameElement {
    if (this.frame?.isConnected) return this.frame
    const frame = document.createElement('iframe')
    frame.setAttribute('aria-label', 'metldr reply suggestions')
    frame.style.cssText = `position:fixed;top:0;left:0;width:${this.size[0]}px;height:${this.size[1]}px;`
      + 'border:0;margin:0;padding:0;background:transparent;color-scheme:normal;z-index:2147483646;visibility:hidden'
    frame.src = chrome.runtime.getURL('inpage/replies.html')
    document.documentElement.appendChild(frame)
    this.frame = frame
    return frame
  }

  hide(): void {
    this.shown = false
    if (this.frame) this.frame.style.visibility = 'hidden'
  }

  // Plain text at the cursor: the reply never carries markup, and the quoted thread stays.
  private insert(text: string): void {
    try {
      this.compose?.insertTextIntoBodyAtCursor(text)
    } catch (error) {
      console.error('metldr: the reply could not be inserted', error)
    }
  }

  // Above the reply, at its right edge, inside the viewport.
  private place(): void {
    if (!this.shown || !this.frame) return
    const anchor = this.compose?.getElement?.()?.getBoundingClientRect()
    const [width, height] = this.size
    const right = anchor ? anchor.right : window.innerWidth - 24
    const top = anchor ? anchor.top : window.innerHeight - 24
    const x = Math.min(Math.max(right - width + MARGIN, 10 - MARGIN), window.innerWidth - width + MARGIN - 10)
    const y = Math.max(top - height + MARGIN - 8, 10 - MARGIN)
    Object.assign(this.frame.style, { left: `${x}px`, top: `${y}px`, width: `${width}px`, height: `${height}px` })
  }
}

export const replyFrame = new ReplyFrame()
