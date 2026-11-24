// Gmail read from its page: the open thread's id from the address, and the thread's text for
// when InboxSDK has not loaded yet.

const threadInHash = /#[^/]+\/(?:[^/]+\/)?([A-Za-z0-9_-]{16,})$/

export function threadFromUrl(): string | null {
  const params = new URL(location.href).searchParams
  return location.hash.match(threadInHash)?.[1] ?? params.get('msgid') ?? params.get('tid')
}

export function emailFromDom(): { content: string; metadata: { subject: string; from: string } } | null {
  const subject = document.querySelector('h2[data-thread-perm-id], .hP, [role="heading"][data-thread-perm-id]')
  const sender = document.querySelector('.gD, [email], .go')
  const bodies = Array.from(document.querySelectorAll('.a3s.aiL, .ii.gt, [data-message-id] .a3s'))
  const content = bodies.map(el => el.textContent?.trim() ?? '').filter(text => text.length > 20)
    .map((text, i) => `Message ${i + 1}:\n${text}`).join('\n\n')
  if (content.length < 50) return null
  return {
    content,
    metadata: {
      subject: subject?.textContent?.trim() ?? '',
      from: sender?.getAttribute('email') || sender?.textContent?.trim() || '',
    },
  }
}
