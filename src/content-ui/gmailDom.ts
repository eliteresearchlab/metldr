// The open thread read from Gmail's DOM, for when InboxSDK has not loaded yet.
export function emailFromDom(): { content: string; metadata: { subject?: string; from?: string } } | null {
  try {
    // gmail subject selector
    const subjectEl = document.querySelector('h2[data-thread-perm-id]') || 
                      document.querySelector('.hP') ||
                      document.querySelector('[role="heading"][data-thread-perm-id]')
    const subject = subjectEl?.textContent?.trim() || ''
    
    // gmail sender - look for sender element  
    const senderEl = document.querySelector('.gD') ||
                     document.querySelector('[email]') ||
                     document.querySelector('.go')
    const from = senderEl?.getAttribute('email') || senderEl?.textContent?.trim() || ''
    
    // gmail message bodies - multiple selectors for different views
    const messageContainers = document.querySelectorAll('.a3s.aiL, .ii.gt, [data-message-id] .a3s')
    
    if (messageContainers.length === 0) {
      console.log('metldr: no email bodies found in DOM')
      return null
    }
    
    let content = ''
    messageContainers.forEach((container, i) => {
      const text = container.textContent?.trim() || ''
      if (text.length > 20) {
        content += `Message ${i + 1}:\n${text}\n\n`
      }
    })
    
    content = content.trim()
    if (content.length < 50) {
      console.log('metldr: extracted content too short')
      return null
    }
    
    console.log('metldr: DOM extraction successful', { subject: subject.slice(0, 30), len: content.length })
    
    return {
      content,
      metadata: { subject, from }
    }
  } catch (err) {
    console.error('metldr: DOM extraction error:', err)
    return null
  }
}
