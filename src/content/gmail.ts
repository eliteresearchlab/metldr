import * as InboxSDK from '@inboxsdk/core'
import { INBOXSDK_APP_ID } from '../config/secrets'
import { UIService } from '../content-ui/UIService'
import { emailExtractor } from '../content-ui/EmailExtractor'
import { emailFromDom } from '../content-ui/gmailDom'

// Gmail's content script: InboxSDK, the thread summaries and reply suggestions. Only Gmail loads
// it, so no other page carries InboxSDK.

// The listener comes first, so the extension can read the open thread while InboxSDK loads.
chrome.runtime.onMessage.addListener((msg, _sender, respond) => {
  if (msg?.type === 'EXTRACT_ARTICLE') {
    respond({ success: true, data: { skip: true, reason: 'email client' } })
    return false
  }
  if (msg?.type !== 'GET_EMAIL_CONTENT') return false
  try {
    const email = emailExtractor.getLastExtracted() ?? emailFromDom()
    respond(email ? { success: true, content: email.content, metadata: email.metadata } : { success: false, error: 'no email content found' })
  } catch (err) {
    console.error('metldr: email content extraction failed:', err)
    respond({ success: false, error: (err as Error).message })
  }
  return false
})

UIService.init()
UIService.onChange(() => UIService.updateSummaryTheme())
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && changes.theme) UIService.setTheme(changes.theme.newValue as string)
})
Promise.all([InboxSDK.load(2, INBOXSDK_APP_ID), UIService.loadFromStorage()])
  .then(([sdk]) => emailExtractor.init(sdk, () => {}))
  .catch(err => console.error('metldr: gmail setup failed:', err))
