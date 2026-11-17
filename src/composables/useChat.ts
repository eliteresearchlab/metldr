import { ref, nextTick, type Ref, computed } from 'vue'
import type { AppPageSummary, AppChatMessage, AppChatResponse } from '@/types'
import type { GatewayTurn } from './useGateway'
import { sendToBackground } from './useMessaging'
import { logger } from '@/services/LoggerService'
import { analyticsService } from '@/services/AnalyticsService'
import { cacheService } from '@/services/CacheService'

const trailingSlashPattern = /\/+$/
const gmailThreadPattern = /#[^/]+\/(?:[^/]+\/)?([A-Za-z0-9_-]{16,})$/


const log = logger.createScoped('Chat')

let saveTimeout: ReturnType<typeof setTimeout> | null = null
const SAVE_DEBOUNCE_MS = 2000

function debouncedSaveChat(url: string, messages: AppChatMessage[]): void {
  if (saveTimeout) clearTimeout(saveTimeout)
  saveTimeout = setTimeout(() => {
    const normalizedUrl = url.split('?')[0].replace(trailingSlashPattern, '')
    cacheService.setTabSession(normalizedUrl, messages, null, false)
      .catch(err => log.warn('auto-save failed', err.message))
  }, SAVE_DEBOUNCE_MS)
}

function flushSaveChat(): void {
  if (saveTimeout) {
    clearTimeout(saveTimeout)
    saveTimeout = null
  }
}

// URL-scoped state - each URL has its own isolated state
interface ChatState {
  messages: AppChatMessage[]
  loading: boolean
  indexing: boolean
  input: string
}

// Map of URL -> ChatState, this is the source of truth
const urlStates = new Map<string, ChatState>()

// current active URL - determines which state the shared refs point to  
let activeUrl: string | null = null

// shared refs that point to current URL's state (for Vue reactivity)
const chatMessages = ref<AppChatMessage[]>([])
const chatInput = ref<string>('')
const chatLoading = ref<boolean>(false)
const chatIndexing = ref<boolean>(false)
const emailContext = ref<{ summary?: string; subject?: string; sender?: string } | null>(null)

// get or create state for a URL
function getUrlState(url: string): ChatState {
  if (!urlStates.has(url)) {
    urlStates.set(url, {
      messages: [],
      loading: false,
      indexing: false,
      input: ''
    })
  }
  return urlStates.get(url)!
}

// sync shared refs TO the URL state (save before switch)
function saveCurrentToUrlState(): void {
  if (activeUrl) {
    const state = getUrlState(activeUrl)
    // only overwrite messages if chatMessages has more (urlStates may have been updated directly during streaming)
    if (chatMessages.value.length >= state.messages.length) {
      state.messages = [...chatMessages.value]
    }
    state.loading = chatLoading.value
    state.indexing = chatIndexing.value
    state.input = chatInput.value
    log.log(`[State] saved to ${activeUrl.slice(0, 40)}: ${state.messages.length} msgs`)
  }
}

// sync shared refs FROM the URL state (load after switch)
function loadFromUrlState(url: string): void {
  const state = getUrlState(url)
  chatMessages.value = [...state.messages]
  chatLoading.value = state.loading
  chatIndexing.value = state.indexing
  chatInput.value = state.input
  activeUrl = url
  log.log(`[State] loaded from ${url.slice(0, 40)}: ${state.messages.length} msgs`)
}

// switch state to a new URL (called on tab change)
function switchToUrl(newUrl: string): void {
  if (newUrl === activeUrl) return
  saveCurrentToUrlState()
  loadFromUrlState(newUrl)
}

// update state for a specific URL (may be different from current)
function updateUrlState(targetUrl: string, updater: (state: ChatState) => void): void {
  const state = getUrlState(targetUrl)
  updater(state)
  
  // if this is the active URL, also update shared refs
  if (targetUrl === activeUrl) {
    chatMessages.value = [...state.messages]
    chatLoading.value = state.loading
    chatIndexing.value = state.indexing
  }
}

// set messages for a URL from external source (IDB load) - syncs with urlStates
function setUrlMessages(url: string, messages: AppChatMessage[]): void {
  const state = getUrlState(url)
  state.messages = [...messages]
  
  // if this is the active URL, also update shared refs
  if (url === activeUrl) {
    chatMessages.value = [...messages]
  }
  log.log(`[State] set ${url.slice(0, 40)}: ${messages.length} msgs from IDB`)
}

// get messages for a URL from in-memory state
function getUrlMessages(url: string): AppChatMessage[] {
  const state = urlStates.get(url)
  return state ? [...state.messages] : []
}

export function useChat() {
  // get current active URL
  async function getActiveUrl(): Promise<string> {
    try {
      const tabs = await chrome.tabs.query({ active: true, currentWindow: true })
      return tabs[0]?.url || ''
    } catch {
      return ''
    }
  }

  async function sendChatMessage(
    pageSummary: Ref<AppPageSummary | null>,
    stream: (system: string, messages: GatewayTurn[], onDelta: (text: string) => void) => { done: Promise<{ text: string }> },
    chatContainer: Ref<HTMLElement | null>,
    chatInputRef: Ref<HTMLElement | null>,
    saveTabSession?: () => Promise<void>
  ): Promise<void> {
    if (!chatInput.value.trim() || chatLoading.value) return
    
    // CRITICAL: capture target URL at message start
    const targetUrl = await getActiveUrl()
    if (!targetUrl) {
      log.warn('no active URL, cannot start chat')
      return
    }
    
    log.log(`[Chat] starting for ${targetUrl.slice(0, 50)}`)
    
    const userMessage = chatInput.value.trim()
    const messageStartTime = performance.now()  // track total time from message send
    chatInput.value = ''
    
    updateUrlState(targetUrl, state => {
      state.messages.push({ role: 'user', content: userMessage })
      state.input = ''
    })
    
    analyticsService.trackChat('user', userMessage).catch(() => {})
    
    await nextTick()
    if (chatContainer.value) {
      chatContainer.value.scrollTop = chatContainer.value.scrollHeight
    }
    
    if (chatInputRef.value) {
      const el = chatInputRef.value as unknown as { focus?: () => void }
      el.focus?.()
    }
    
    try {
      let contextText = pageSummary.value?.fullContent || ''
      
      if (!contextText) {
        try {
          const tabs = await chrome.tabs.query({ active: true, currentWindow: true })
          const tab = tabs[0]
          if (!tab?.id || !tab.url) throw new Error('no active tab')
          
          if (tab.url.includes('mail.google.com')) {
            log.log('no fullContent, fetching email from content script')
            
            let response: { success: boolean; content?: string; metadata?: { subject?: string; from?: string } } | undefined
            
            try {
              response = await chrome.tabs.sendMessage(tab.id, { type: 'GET_EMAIL_CONTENT' })
            } catch {
              log.log('content script not ready, injecting and retrying')
              await chrome.scripting.executeScript({
                target: { tabId: tab.id },
                files: ['content.js']
              })
              await new Promise(r => setTimeout(r, 500))
              response = await chrome.tabs.sendMessage(tab.id, { type: 'GET_EMAIL_CONTENT' })
            }
            
            if (response?.success && response.content) {
              let emailCtx = ''
              if (response.metadata?.subject) {
                emailCtx += `Subject: ${response.metadata.subject}\n`
              }
              if (response.metadata?.from) {
                emailCtx += `From: ${response.metadata.from}\n`
              }
              emailCtx += `\nEMAIL CONTENT:\n${response.content}`
              contextText = emailCtx
              log.log('fetched email content', contextText.length)
            }
          } else {
            log.log('no fullContent, extracting article from page')
            try {
              const extractResponse = await chrome.runtime.sendMessage({
                type: 'EXTRACT_ONLY',
                tabId: tab.id
              }) as { success: boolean; data?: { title?: string; author?: string; content?: string } } | undefined
              
              if (extractResponse?.success && extractResponse.data?.content) {
                const d = extractResponse.data
                let articleContext = 'ARTICLE METADATA:\n'
                if (d.title) articleContext += `- Title: "${d.title}"\n`
                if (d.author) articleContext += `- Author: ${d.author}\n`
                articleContext += '\n---\nARTICLE CONTENT:\n\n' + d.content
                contextText = articleContext
                log.log('fetched article content', contextText.length)
              } else {
                log.log('EXTRACT_ONLY failed, trying direct extraction')
                const [result] = await chrome.scripting.executeScript({
                  target: { tabId: tab.id! },
                  func: () => document.body.innerText.slice(0, 15000)
                })
                if (result?.result) {
                  contextText = `PAGE CONTENT:\n\n${result.result}`
                  log.log('direct extraction got', `${contextText.length} chars`)
                }
              }
            } catch (extractErr) {
              log.log('extraction error:', (extractErr as Error).message)
            }
          }
        } catch (err) {
          log.log('failed to fetch page content', (err as Error).message)
        }
      }
      
      const currentUrl = targetUrl
      
      const isEmail = contextText.includes('EMAIL CONTENT:')
      const docType = isEmail ? 'email thread' : 'article'
      
      let emailId: string | null = null
      if (isEmail && currentUrl.includes('mail.google.com')) {
        const hashMatch = currentUrl.match(gmailThreadPattern)
        emailId = hashMatch?.[1] || null
        if (emailId) log.log(`extracted emailId: ${emailId.slice(0, 20)}...`)
      }
      
      const sourceId = isEmail && emailId ? emailId : currentUrl
      const sourceUrl = isEmail && emailId ? `email://${emailId}` : currentUrl
      const sourceType = isEmail ? 'email' : 'article'
      
      const CONTEXT_THRESHOLD = 8000
      let relevantContext = ''
      
      if (contextText && currentUrl) {
        const checkRes = await sendToBackground({
          type: 'RAG_HAS_INDEXED_CONTENT',
          sourceUrl
        }) as { success: boolean; hasContent?: boolean }
        const alreadyIndexed = checkRes?.success && checkRes?.hasContent
        
        if (contextText.length > CONTEXT_THRESHOLD) {
          if (!alreadyIndexed) {
            updateUrlState(targetUrl, s => { s.indexing = true })
          }
          log.log(`ensuring large doc indexed for chat... ${contextText.length} chars, sourceId: ${sourceId.slice(0, 40)}`)
          try {
            const ensureRes = await sendToBackground({
              type: 'RAG_ENSURE_INDEXED',
              text: contextText,
              metadata: { sourceId, sourceUrl, sourceType, title: '' }
            }) as { success: boolean; result?: string }
            log.log(`ensureIndexed result: ${ensureRes?.result}`)
          } catch (indexErr) {
            const msg = (indexErr as Error).message
            if (msg !== 'Indexing aborted' && msg !== 'Operation cancelled') {
              log.warn('indexing failed', msg)
            }
          } finally {
            if (!alreadyIndexed) {
              updateUrlState(targetUrl, s => { s.indexing = false })
            }
          }
          
          // indexing done, now show typing indicator
          updateUrlState(targetUrl, s => { s.loading = true })
          
          // search RAG for relevant chunks
          const ragStart = performance.now()
          const searchRes = await sendToBackground({ 
            type: 'RAG_SEARCH_WITH_CONTEXT', 
            query: userMessage, 
            limit: 5, 
            sourceUrl 
          }) as { success: boolean; context?: string }
          const ragTime = Math.round(performance.now() - ragStart)
          relevantContext = searchRes?.success ? (searchRes.context || '') : ''
          log.log(`RAG search: ${relevantContext.length} chars (${ragTime}ms)`)
          
          // fallback to truncated if RAG returns nothing
          if (!relevantContext) {
            relevantContext = contextText.slice(0, CONTEXT_THRESHOLD)
            log.log('RAG empty, using truncated content')
          }
        } else {
          // only show indexing indicator if not already indexed
          if (!alreadyIndexed) {
            updateUrlState(targetUrl, s => { s.indexing = true })
          }
          log.log(`ensuring small doc indexed for chat... ${contextText.length} chars, sourceId: ${sourceId.slice(0, 40)}`)
          try {
            const ensureRes = await sendToBackground({
              type: 'RAG_ENSURE_INDEXED',
              text: contextText,
              metadata: { sourceId, sourceUrl, sourceType, title: '' }
            }) as { success: boolean; result?: string }
            log.log(`ensureIndexed result: ${ensureRes?.result}`)
          } catch (indexErr) {
            const msg = (indexErr as Error).message
            if (msg !== 'Indexing aborted' && msg !== 'Operation cancelled') {
              log.warn('indexing failed', msg)
            }
          } finally {
            if (!alreadyIndexed) {
              updateUrlState(targetUrl, s => { s.indexing = false })
            }
          }
          
          updateUrlState(targetUrl, s => { s.loading = true })
          
          relevantContext = contextText
          log.log(`using full context (${contextText.length} chars < ${CONTEXT_THRESHOLD} threshold)`)
        }
      } else if (contextText) {
        updateUrlState(targetUrl, s => { s.loading = true })
        relevantContext = contextText
        log.log(`no URL, using full context (${contextText.length} chars)`)
      } else {
        updateUrlState(targetUrl, s => { s.loading = true })
      }
      
      const systemPrompt = relevantContext  
        ? `You are an assistant helping the user understand a ${docType}.

RELEVANT SECTIONS FROM THE DOCUMENT:
${relevantContext}

RULES:
1. Answer based ONLY on the sections above
2. If the info isn't in these sections, say "I don't see that in the document"
3. Be concise and helpful`
        : contextText
          ? `You are an assistant helping the user understand a ${docType}.

DOCUMENT:
${contextText.slice(0, 8000)}

RULES:
1. Answer based on the document
2. Be concise`
          : 'You are a helpful assistant. Be concise.'


      // the reply streams from the model gateway on whichever route the user chose
      const history = getUrlState(targetUrl).messages.slice(-7)
        .filter(m => m.role === 'user' || m.role === 'assistant')
        .map(m => ({ role: m.role as 'user' | 'assistant', content: m.content }))
      const start = performance.now()
      let fullResponse = ''
      const reply = stream(systemPrompt, history, delta => {
        fullResponse += delta
        updateUrlState(targetUrl, state => {
          const lastMsg = state.messages[state.messages.length - 1]
          if (lastMsg?.role === 'assistant') {
            lastMsg.content = fullResponse
          } else {
            state.messages.push({ role: 'assistant', content: fullResponse })
          }
        })
        debouncedSaveChat(targetUrl, getUrlState(targetUrl).messages)
        void nextTick(() => {
          if (activeUrl === targetUrl && chatContainer.value) {
            chatContainer.value.scrollTop = chatContainer.value.scrollHeight
          }
        })
      })
      const done = await reply.done
      flushSaveChat()

      const llmTime = Math.round(performance.now() - start)
      const totalTime = Math.round(performance.now() - messageStartTime)
      const ragTime = totalTime - llmTime
      updateUrlState(targetUrl, state => {
        const lastMsg = state.messages[state.messages.length - 1]
        if (lastMsg?.role === 'assistant') {
          lastMsg.content = done.text
          lastMsg.timing = { total: totalTime, llm: llmTime, rag: ragTime > 100 ? ragTime : undefined }
        } else {
          state.messages.push({ role: 'assistant', content: done.text, timing: { total: totalTime, llm: llmTime } })
        }
      })

      // save to IDB immediately - prevents lost response if user switched tabs
      const completedState = getUrlState(targetUrl)
      const normalizedUrl = targetUrl.split('?')[0].replace(trailingSlashPattern, '')
      cacheService.setTabSession(normalizedUrl, completedState.messages, null, false)
        .catch(err => log.warn('post-stream save failed', err.message))

      analyticsService.trackChat('assistant', done.text, totalTime).catch(() => {})
    } catch (error) {
      log.warn('chat error', (error as Error).message)
      updateUrlState(targetUrl, state => {
        state.messages.push({
          role: 'assistant',
          content: 'sorry, something went wrong. try again.'
        })
      })
    } finally {
      updateUrlState(targetUrl, state => {
        state.loading = false
        state.indexing = false
      })
      
      await nextTick()
      if (activeUrl === targetUrl && chatContainer.value) {
        chatContainer.value.scrollTop = chatContainer.value.scrollHeight
      }
      
      if (saveTabSession) {
        await saveTabSession()
      }
      
      if (chatInputRef.value) {
        const el = chatInputRef.value as unknown as { focus?: () => void }
        el.focus?.()
      }
    }
  }

  function clearChat(chatInputRef?: Ref<HTMLElement | null>, saveTabSession?: () => Promise<void>) {
    if (activeUrl) {
      updateUrlState(activeUrl, state => {
        state.messages = []
      })
    }
    chatMessages.value = []
    if (saveTabSession) saveTabSession()
    nextTick(() => {
      if (chatInputRef?.value) {
        const el = chatInputRef.value as unknown as { focus?: () => void }
        el.focus?.()
      }
    })
  }

  function resetChatState() {
    if (activeUrl) {
      updateUrlState(activeUrl, state => {
        state.messages = []
        state.loading = false
        state.indexing = false
        state.input = ''
      })
    }
    chatMessages.value = []
    chatInput.value = ''
    chatLoading.value = false
    chatIndexing.value = false
    emailContext.value = null
  }
  
  async function syncIndexingStatus(): Promise<void> {
    const url = await getActiveUrl()
    if (!url) return
    
    try {
      const res = await sendToBackground({ type: 'RAG_IS_INDEXING', sourceId: url }) as { success: boolean; isIndexing?: boolean }
      if (res?.success && res.isIndexing) {
        log.log(`[Sync] background is indexing ${url.slice(0, 40)}`)
        updateUrlState(url, s => { s.indexing = true })
      }
    } catch (err) {
      log.warn('[Sync] failed to query indexing status', (err as Error).message)
    }
  }

  return {
    chatMessages,
    chatInput,
    chatLoading,
    chatIndexing,
    emailContext,
    
    sendChatMessage,
    clearChat,
    resetChatState,
    syncIndexingStatus,
    
    switchToUrl,
    getActiveUrl,
    saveCurrentToUrlState,
    loadFromUrlState,
    setUrlMessages,
    getUrlMessages
  }
}
