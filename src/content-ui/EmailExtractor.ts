import type { InboxSDK, MessageView, ThreadView } from '@inboxsdk/core'
import { summaryFrame } from './SummaryFrame'
import { replyFrame } from './ReplyFrame'
import { threadFromUrl } from './gmailDom'

// The open Gmail thread, read through InboxSDK: its messages as text and the headers the
// extension's email prompts use. The thread's summary frame is placed after its last message.

interface EmailMetadata {
  subject: string
  from: string | null
  fromEmail: string | null
  to: string[]
  date: string | null
  participants: string[]
  emailCount: number
}

interface Email {
  thread: string
  content: string
  metadata: EmailMetadata
}

// The last message's header, else the thread's header: where the summary goes.
function lastHeader(): Element | null {
  const main = document.querySelector('div[role="main"]')
  if (!main) return null
  const messages = main.querySelectorAll('[data-message-id], .gs, .adn')
  const last = messages[messages.length - 1]
  if (last) return last.querySelector('.gH, .gE, .iw') ?? last
  return main.querySelector('.gH, .gE, [data-thread-perm-id]') ?? main
}

const pause = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

const visible = (element: HTMLElement): boolean => {
  const style = getComputedStyle(element)
  return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0'
    && element.offsetWidth > 0 && element.offsetHeight > 0
}

const all = (selector: string): HTMLElement[] => Array.from(document.querySelectorAll<HTMLElement>(selector))

// Clicks each element `gap` ms apart; how many it clicked.
const clickEach = async (elements: HTMLElement[], gap: number): Promise<number> => {
  for (const element of elements) {
    element.click()
    await pause(gap)
  }
  return elements.length
}

// The ways Gmail shows collapsed messages, each clicked open in turn.
const openers: ((unloaded: MessageView[]) => Promise<number>)[] = [
  unloaded => clickEach(unloaded.map(m => m.getElement()).filter(Boolean)
    .map(el => el.querySelector<HTMLElement>('.gE, .gs, [data-legacy-message-id]') ?? el), 50),
  () => clickEach(all('[data-tooltip="Expand all"], [aria-label*="Expand"]').filter(visible).slice(0, 1), 150),
  () => clickEach(all('.kx, .kQ').filter(visible), 80),
  () => clickEach(all('.h7[role="listitem"][aria-expanded="false"]').filter(visible), 50),
  () => clickEach(all('[data-legacy-message-id]').filter(el => el.clientHeight < 100 && visible(el)), 50),
  () => clickEach(all('.adn.ads').filter(el => !el.querySelector('.a3s') && visible(el)), 50),
]

// Opens the thread's collapsed messages and waits up to `timeoutMs` for them to load.
async function openAll(messages: MessageView[], timeoutMs = 10000): Promise<void> {
  const unloaded = () => messages.filter(m => !m.isLoaded())
  const waiting = unloaded()
  if (waiting.length === 0) return
  const loaded = Promise.all(waiting.map(m => new Promise<void>(resolve => m.once('load', () => resolve()))))
  let before = -1
  for (let pass = 0; pass < 5; pass++) {
    const left = unloaded()
    if (left.length === 0 || left.length === before) break
    before = left.length
    for (const open of openers) {
      // A message opened by an earlier way is clicked no more, so it is not closed again.
      const clicked = await open(unloaded()).catch(() => 0)
      if (clicked > 0) await pause(100)
    }
    await pause(150)
  }
  if (unloaded().length > 0) await Promise.race([loaded, pause(timeoutMs)])
  // Opening messages moves the thread, so the newest message is brought back into view.
  const smooth = !matchMedia('(prefers-reduced-motion: reduce)').matches
  lastHeader()?.scrollIntoView({ behavior: smooth ? 'smooth' : 'auto', block: 'start' })
}

const contact = (c: { name?: string; emailAddress: string }): string =>
  c.name ? `${c.name} <${c.emailAddress}>` : c.emailAddress

const recipients = (m: MessageView): Promise<string[]> =>
  Promise.resolve()
    .then(() => m.getRecipientsFull())
    .then(list => list.map(contact))
    .catch(() => m.getRecipientEmailAddresses())
    .catch(() => [])

// The thread's loaded messages as text and its headers; null when it has under 50 characters.
async function read(view: ThreadView, thread: string): Promise<Email | null> {
  const messages = view.getMessageViewsAll()
  if (messages.length === 0) return null
  await openAll(messages)
  const loaded = messages.filter(m => m.isLoaded())
  // Recipient lists may need a request each, so they are asked for together.
  const to = new Set((await Promise.all(loaded.map(recipients))).flat())
  const participants = new Set<string>()
  let sender: { name?: string; emailAddress: string } | null = null
  let date: string | null = null
  const parts: string[] = []
  for (const message of loaded) {
    try {
      const from = message.getSender()
      sender ??= from
      if (from?.emailAddress) participants.add(contact(from))
      const when = message.getDateString() || null
      date = when ?? date
      const body = message.getBodyElement()?.innerText ?? ''
      if (body.length > 20) {
        parts.push(`--- Message from ${from?.name || from?.emailAddress || 'Unknown'}${when ? ` (${when})` : ''} ---\n${body}`)
      }
    } catch { /* a message Gmail has not drawn */ }
  }
  const content = parts.join('\n').split('\n').filter(line => line.trim()).join('\n')
  if (content.length < 50) return null
  return {
    thread,
    content,
    metadata: {
      subject: view.getSubject(),
      from: sender?.name || null,
      fromEmail: sender?.emailAddress || null,
      to: [...to],
      date,
      participants: [...participants],
      emailCount: messages.length,
    },
  }
}

export class EmailExtractor {
  private last: Email | null = null
  private latest: ThreadView | null = null

  // The thread read last, for the extension's chat and reply suggestions.
  lastRead(): Email | null {
    return this.last
  }

  init(sdk: InboxSDK): void {
    sdk.Conversations.registerThreadViewHandler(view => void this.open(view))
    replyFrame.init(sdk)
  }

  // A thread opened later takes over, so only the newest thread places its summary.
  private async open(view: ThreadView): Promise<void> {
    this.latest = view
    const thread = threadFromUrl() ?? await view.getThreadIDAsync().catch(() => null)
    if (!thread) return
    const email = await read(view, thread)
    if (!email || this.latest !== view) return
    this.last = email
    const header = lastHeader() ?? (await pause(200), lastHeader())
    if (header && this.latest === view) {
      summaryFrame.show(header, { thread, count: email.metadata.emailCount, content: email.content, metadata: email.metadata })
    }
  }
}

export const emailExtractor = new EmailExtractor()
