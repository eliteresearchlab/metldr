// A Gmail thread's summary: an extension frame (Rust and Rune) inside the thread, after its
// header. The frame asks the extension for the kept summary or a new one; this script places it,
// gives it the thread it reads, and sets the height it asks for.

interface Job {
  thread: string
  count: number
  content: string
  metadata: unknown
}

export class SummaryFrame {
  private frame: HTMLIFrameElement | null = null
  private job: Job | null = null

  constructor() {
    // The frame asks the extension for its job, and the extension asks this script.
    chrome.runtime.onMessage.addListener((message, _sender, respond) => {
      if (message?.type !== 'INPAGE_PULL' || message.frame !== 'summary') return false
      respond(this.job)
      return false
    })
    // Only this frame is heard, and only for its height.
    window.addEventListener('message', event => {
      const frame = this.frame
      const fromFrame = frame !== null && event.source === frame.contentWindow
        && event.origin === `chrome-extension://${chrome.runtime.id}` && event.data?.metldr === 'summary'
      if (fromFrame && Number.isFinite(event.data.height)) frame.style.height = `${Math.ceil(event.data.height)}px`
    })
  }

  // The thread's card after `header`; a thread opened later takes the card over.
  show(header: Element, job: Job): void {
    this.job = job
    if (this.frame?.isConnected && this.frame.previousElementSibling === header) {
      this.frame.contentWindow?.postMessage({ metldr: 'next' }, '*')
      return
    }
    this.frame?.remove()
    const frame = document.createElement('iframe')
    frame.setAttribute('aria-label', 'metldr email summary')
    frame.style.cssText = 'display:block;width:100%;height:80px;border:0;margin:0;padding:0;background:transparent;color-scheme:normal'
    frame.src = chrome.runtime.getURL('inpage/summary.html')
    header.insertAdjacentElement('afterend', frame)
    this.frame = frame
  }

  hide(): void {
    this.frame?.remove()
    this.frame = null
    this.job = null
  }
}

export const summaryFrame = new SummaryFrame()
