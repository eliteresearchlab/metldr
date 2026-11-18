import { cacheService } from './CacheService'
import {
  clearSummaryCache, dictionary, globalChat, lookupWord, recordStat, search, statsSummary, summaryGate
} from './RustCore'
import { EmailService } from './EmailService'
import { PageService } from './PageService'
import { pdfService } from './pdf/PdfService'
import { logger } from './LoggerService'
import { concurrencyManager } from './ConcurrencyManager'


import type {
  EmailSummaryMessage,
  GetReplySuggestionsMessage,
  GenerateReplySuggestionsMessage,
  ExtractAndSummarizeMessage,
  WordLookupMessage,
  BackgroundMessage,
  ResponseCallback,
  SummaryResult,
  ExtractedData,
  GetEmailCacheMessage,
  SetEmailCacheMessage
} from '../types'

const log = logger.createScoped('BackgroundBootstrap')
const SUMMARY_BULLET_PREFIX_PATTERN = /^[•\-*]\s*/
const SUMMARY_SENTENCE_SEPARATOR_PATTERN = /[.!?]+/
const WHITESPACE_PATTERN = /\s+/

export class BackgroundBootstrap {
  static isInitialized = false
  static initPromise: Promise<void> | null = null
  static summaryQueue: Promise<unknown> = Promise.resolve()
  
  static async init(): Promise<void> {
    if (this.initPromise) return this.initPromise
    
    this.initPromise = this._doInit()
    return this.initPromise
  }
  
  static async _doInit(): Promise<void> {
    log.log('starting initialization...')
    
    chrome.sidePanel
      .setPanelBehavior({ openPanelOnActionClick: true })
      .catch(err => log.error('side panel error', err))

    log.log('cache service ready (lazy init)')

    this.isInitialized = true

    chrome.runtime.onMessage.addListener((msg: BackgroundMessage, _sender, respond: ResponseCallback) => {
      if (msg.type === 'SUMMARIZE_EMAIL') {
        this._onEmailSummary(msg, respond)
        return true
      }

      if (msg.type === 'GET_REPLY_SUGGESTIONS') {
        this._onGetReplySuggestions(msg, respond)
        return true
      }

      if (msg.type === 'GENERATE_REPLY_SUGGESTIONS') {
        this._onGenerateReplySuggestions(msg as GenerateReplySuggestionsMessage, respond)
        return true
      }

      if (msg.type === 'EXTRACT_ONLY') {
        this._onExtractOnly(msg, respond)
        return true
      }

      if (msg.type === 'EXTRACT_AND_SUMMARIZE') {
        this._onExtractAndSummarize(msg, respond)
        return true
      }

      if (msg.type === 'WORD_LOOKUP') {
        this._onWordLookup(msg, respond)
        return true
      }

      if (msg.type === 'DICTIONARY') {
        dictionary(msg).then(respond, err => respond({ error: (err as Error).message }))
        return true
      }

      if (msg.type === 'STATS_RECORD') {
        recordStat(msg.event).then(() => respond(true), () => respond(false))
        return true
      }

      if (msg.type === 'STATS_SUMMARY') {
        statsSummary(msg.days).then(respond, () => respond(null))
        return true
      }

      if (msg.type === 'CLEAR_SUMMARY_CACHE') {
        const failed = (err: Error) => respond({ ok: false, error: err.message })
        clearSummaryCache().then(() => respond({ ok: true }), failed)
        return true
      }

      if (msg.type === 'GLOBAL_CHAT') {
        this._onGlobalChat(msg as { type: string; messages: import('../types').ChatMessage[] }, respond)
        return true
      }

      if (msg.type === 'GET_EMAIL_CACHE') {
        this._onGetEmailCache(msg as GetEmailCacheMessage, respond)
        return true
      }

      if (msg.type === 'SET_EMAIL_CACHE') {
        this._onSetEmailCache(msg as SetEmailCacheMessage, respond)
        return true
      }

      if (msg.type === 'RAG_HAS_INDEXED_CONTENT') {
        this._onRagHasIndexedContent(msg as { type: string; sourceUrl: string }, respond)
        return true
      }

      if (msg.type === 'RAG_SEARCH_WITH_CONTEXT') {
        this._onRagSearchWithContext(msg as { type: string; query: string; limit?: number; sourceUrl?: string }, respond)
        return true
      }

      if (msg.type === 'RAG_IS_INDEXING') {
        const { sourceId } = msg as { type: string; sourceId: string }
        search.isIndexing(sourceId).then(
          isIndexing => respond({ success: true, isIndexing }),
          () => respond({ success: false, isIndexing: false })
        )
        return true
      }

      if (msg.type === 'RAG_ENSURE_INDEXED') {
        this._onRagEnsureIndexed(msg as { type: string; text: string; metadata: any }, respond)
        return true
      }

      if (msg.type === 'PDF_SUMMARIZE') {
        this._onPdfSummarize(msg as { type: string; url: string }, respond)
        return true
      }

      if (msg.type === 'PDF_EXTRACT_TEXT') {
        this._onPdfExtractText(msg as { type: string; url: string }, respond)
        return true
      }

      if (msg.type === 'OPEN_SIDE_PANEL') {
        this._onOpenSidePanel(msg as { type: string; focus?: string }, _sender, respond)
        return true
      }

      if (msg.type === 'TOGGLE_SIDE_PANEL') {
        this._onToggleSidePanel(msg as { type: string; focus?: string }, _sender, respond)
        return true
      }

      // handler for processing PDFs from ArrayBuffer (local file picker)
      if (msg.type === 'PDF_PROCESS_ARRAYBUFFER') {
        this._onPdfProcessArrayBuffer(msg as { type: string; data: number[]; filename: string; action: 'summarize' | 'copy' }, respond)
        return true
      }

      // handler for checking page cache (used by PdfToolbar)
      if (msg.type === 'GET_PAGE_CACHE') {
        this._onGetPageCache(msg as { type: string; url: string }, respond)
        return true
      }


      return false
    })

    log.log('initialized')
  }

  static _onEmailSummary(msg: EmailSummaryMessage, respond: ResponseCallback): void {
    (async () => {
      try {
        const { emailContent, emailId, metadata, forceRegenerate } = msg
        const summary = await EmailService.summarize(
          emailContent, 
          emailId || null, 
          metadata as Record<string, unknown> | null, 
          forceRegenerate || false
        )
        respond({ summary })
      } catch (err) {
        log.error('onEmailSummary', (err as Error).message)
        const errMsg = (err as Error)?.message || 'unknown error'
        respond({
          error: errMsg,
          summary: {
            summary: `error: ${errMsg}`,
            action_items: [],
            dates: [],
            confidence: 'low'
          }
        })
      }
    })()
  }

  static _onGetReplySuggestions(msg: GetReplySuggestionsMessage, respond: ResponseCallback): void {
    (async () => {
      try {
        const { emailId } = msg
        if (!emailId) {
          respond({ success: false, error: 'no emailId' })
          return
        }

        const suggestions = await EmailService.getCachedReplies(emailId)
        if (suggestions && suggestions.length > 0) {
          respond({ success: true, suggestions })
        } else {
          respond({ success: false, error: 'no suggestions available' })
        }
      } catch (err) {
        log.error('onGetReplySuggestions', (err as Error).message)
        respond({ success: false, error: (err as Error).message })
      }
    })()
  }

  static _onGenerateReplySuggestions(msg: GenerateReplySuggestionsMessage, respond: ResponseCallback): void {
    (async () => {
      try {
        const { emailId, forceRegenerate } = msg
        if (!emailId) {
          respond({ success: false, error: 'no emailId' })
          return
        }

        log.log('generating reply suggestions', { emailId, force: forceRegenerate })

        if (forceRegenerate) {
          try {
            await cacheService.deleteReplySuggestions(emailId)
          } catch (err) {
            log.warn('failed to delete cached suggestions', (err as Error).message)
          }
        }

        if (!forceRegenerate) {
          const cached = await EmailService.getCachedReplies(emailId)
          // filter out empty suggestions before returning
          const validCached = (cached || []).filter(s => s.body && s.body.trim().length > 10)
          if (validCached.length > 0) {
            respond({ success: true, suggestions: validCached })
            return
          }
        }

        const [tab] = await chrome.tabs.query({ active: true, currentWindow: true })
        if (!tab?.id) {
          respond({ success: false, error: 'no active tab' })
          return
        }

        // get email content from content script
        let emailContent = ''
        let metadata: Record<string, unknown> | null = null
        try {
          const resp = await chrome.tabs.sendMessage(tab.id, { type: 'GET_EMAIL_CONTENT', emailId }) as { 
            success: boolean; content?: string; metadata?: Record<string, unknown> 
          }
          if (resp?.success && resp.content) {
            emailContent = resp.content
            metadata = resp.metadata || null
          }
        } catch (err) {
          log.log('could not extract email', (err as Error).message)
        }

        // get cached summary if available (optional)
        const cachedSummary = await cacheService.getEmailSummary(emailId) as import('../types').EmailSummary | null

        // fallback: use summary as context if no direct content
        if (!emailContent && cachedSummary?.summary) {
          emailContent = cachedSummary.summary
        }

        // generate suggestions
        const suggestions = await EmailService.generateReplySuggestions(
          emailId, 
          emailContent || '',
          cachedSummary,
          metadata
        )

        const valid = (suggestions || []).filter(s => s.body?.trim().length > 10)
        respond(valid.length > 0 
          ? { success: true, suggestions: valid }
          : { success: false, error: 'no valid suggestions generated' }
        )
      } catch (err) {
        log.error('onGenerateReplySuggestions', (err as Error).message)
        respond({ success: false, error: (err as Error).message })
      }
    })()
  }

  static _onExtractOnly(msg: { tabId: number }, respond: ResponseCallback): void {
    (async () => {
      try {
        const { tabId } = msg
        if (!tabId) {
          respond({ success: false, error: 'no tabId' })
          return
        }
        const response = await chrome.tabs.sendMessage(tabId, { type: 'EXTRACT_ARTICLE' }) as { success: boolean; data?: ExtractedData; error?: string }
        if (!response?.success || !response.data) {
          respond({ success: false, error: response?.error || 'extraction failed' })
          return
        }
        respond({ success: true, data: response.data })
      } catch (err) {
        respond({ success: false, error: (err as Error).message })
      }
    })()
  }

  static _onExtractAndSummarize(msg: ExtractAndSummarizeMessage, respond: ResponseCallback): void {
    const { tabId, force, trigger } = msg
    this._enqueueSummary(() => this._performExtractAndSummarize(tabId, force || false, trigger || 'auto'))
      .then(result => respond(result))
  }

  static _onWordLookup(msg: WordLookupMessage, respond: ResponseCallback): void {
    const { word, context } = msg
    if (!word || typeof word !== 'string') {
      respond({ success: false, error: 'invalid word' })
      return
    }
    lookupWord(word, context ?? {})
      .then(result => {
        recordStat({ kind: 'lookup' }).catch(err => log.error('word lookup stats', (err as Error).message))
        respond({ success: true, result })
      })
      .catch(err => respond({ success: false, error: (err as Error).message || 'lookup failed' }))
  }

  static _onGlobalChat(msg: { type: string; messages: import('../types').ChatMessage[] }, respond: ResponseCallback): void {
    (async () => {
      try {
        const { messages } = msg
        if (!messages?.length) {
          respond({ ok: false, error: 'no messages provided' })
          return
        }
        
        log.log(`onGlobalChat processing ${messages.length} messages`)
        const result = await globalChat(messages)
        
        log.log(`onGlobalChat result has ${result.sources?.length || 0} sources`)
        
        respond({
          ...result,
          timing: result.timing || null
        })
      } catch (err) {
        log.error('onGlobalChat error:', (err as Error).message)
        respond({ ok: false, error: (err as Error).message || 'global chat failed' })
      }
    })()
  }

  static _enqueueSummary<T>(task: () => Promise<T>): Promise<T | { success: false; error: string }> {
    this.summaryQueue = this.summaryQueue.then(() => task()).catch(err => {
      log.error('summary queue error', (err as Error).message)
      return { success: false, error: (err as Error).message }
    })
    return this.summaryQueue as Promise<T | { success: false; error: string }>
  }

  static async _performExtractAndSummarize(tabId: number, force: boolean, trigger = 'auto'): Promise<SummaryResult> {
    try {
      log.log('_performExtractAndSummarize:', { tabId, force, trigger })
      let extracted: ExtractedData | undefined
      try {
        const response = await chrome.tabs.sendMessage(tabId, { type: 'EXTRACT_ARTICLE' }) as { success: boolean; data?: ExtractedData; error?: string }
        log.log('EXTRACT_ARTICLE response:', { success: response?.success, hasData: !!response?.data, error: response?.error })
        if (!response?.success) {
          return { success: false, error: response?.error || 'extraction failed' }
        }
        extracted = response.data
      } catch (err) {
        log.error('content script error:', (err as Error).message)
        return { success: false, error: 'content script not ready - please refresh the page' }
      }

      if (!extracted) return { success: false, error: 'extraction failed' }
      if (extracted.skip) return { success: false, skip: true, reason: extracted.reason }
      const gating = await summaryGate(extracted, trigger === 'manual' || force)
      log.log('classification:', { action: gating.action, reason: gating.reason, wordCount: extracted?.wordCount })
      if (gating.action === 'skip') return { success: false, skip: true, reason: gating.reason }
      if (gating.action === 'prompt') return { success: false, prompt: true, reason: gating.reason }

      const summary = await PageService.summarize(extracted as Parameters<typeof PageService.summarize>[0], force)
      return { success: true, summary }
    } catch (err) {
      log.error('performExtractAndSummarize]', (err as Error).message)
      return { success: false, error: (err as Error).message }
    }
  }

  static _onGetEmailCache(msg: GetEmailCacheMessage, respond: ResponseCallback): void {
    (async () => {
      try {
        const cached = await cacheService.getEmailCache(msg.emailId)
        respond({ cached: cached.summary, emailCount: cached.emailCount })
      } catch (err) {
        log.error('onGetEmailCache]', (err as Error).message)
        respond({ cached: null, emailCount: 0 })
      }
    })()
  }

  static _onSetEmailCache(msg: SetEmailCacheMessage & { emailCount?: number }, respond: ResponseCallback): void {
    (async () => {
      try {
        await cacheService.updateEmailSession(msg.emailId, {
          summary: msg.summary,
          ...(msg.emailCount !== undefined && { emailCount: msg.emailCount })
        })
        respond({ success: true })
      } catch (err) {
        log.error('onSetEmailCache]', (err as Error).message)
        respond({ success: false })
      }
    })()
  }

  static _onRagHasIndexedContent(msg: { type: string; sourceUrl: string }, respond: ResponseCallback): void {
    (async () => {
      try {
        const hasContent = await search.has(msg.sourceUrl)
        respond({ success: true, hasContent })
      } catch (err) {
        log.error('onRagHasIndexedContent', (err as Error).message)
        respond({ success: false, error: (err as Error).message, hasContent: false })
      }
    })()
  }

  static _onRagSearchWithContext(msg: { type: string; query: string; limit?: number; sourceUrl?: string }, respond: ResponseCallback): void {
    (async () => {
      try {
        const context = await search.context(msg.query, msg.limit || 5, msg.sourceUrl)
        respond({ success: true, context })
      } catch (err) {
        log.error('onRagSearchWithContext', (err as Error).message)
        respond({ success: false, error: (err as Error).message, context: '' })
      }
    })()
  }

  static _onRagEnsureIndexed(msg: { type: string; text: string; metadata: any }, respond: ResponseCallback): void {
    (async () => {
      try {
        const sourceUrl = msg.metadata?.sourceUrl || msg.metadata?.sourceId || 'unknown'
        const sourceId = msg.metadata?.sourceId || sourceUrl
        
        const broadcastProgress = (percent: number) => {
          console.log('[BackgroundBootstrap] Broadcasting progress:', percent, 'for', sourceId.slice(0, 40))
          chrome.runtime.sendMessage({
            type: 'INDEXING_PROGRESS',
            sourceId,
            percent
          }).catch(() => {})
        }
        
        const result = await concurrencyManager.execute('indexing', sourceUrl, async (signal) => {
          if (signal.aborted) throw new Error('Indexing aborted')
          return await search.ensure(msg.text, msg.metadata, broadcastProgress)
        })
        
        respond({ success: true, result })
      } catch (err) {
        const errMsg = (err as Error).message
        if (errMsg !== 'Indexing aborted' && errMsg !== 'Operation cancelled') {
          log.error('onRagEnsureIndexed', errMsg)
        }
        respond({ success: false, error: errMsg })
      }
    })()
  }

  static _onPdfSummarize(msg: { type: string; url: string }, respond: ResponseCallback): void {
    (async () => {
      try {
        const { url } = msg
        if (!url) {
          respond({ success: false, error: 'no url provided' })
          return
        }

        log.log('pdf summarize request:', url.slice(0, 80))

        const result = await this._enqueueSummary(() => pdfService.summarize(url))
        
        if (typeof result === 'object' && 'success' in result && !result.success) {
          respond({ success: false, error: result.error })
          return
        }
        
        const summary = result as string
        
        const bullets = summary
          .split('\n')
          .filter(line => line.trim().startsWith('•') || line.trim().startsWith('-') || line.trim().startsWith('*'))
          .map(line => line.replace(SUMMARY_BULLET_PREFIX_PATTERN, '').trim())
          .filter(Boolean)

        const finalBullets = bullets.length > 0 
          ? bullets 
          : summary.split(SUMMARY_SENTENCE_SEPARATOR_PATTERN).filter(s => s.trim().length > 20).slice(0, 5)

        respond({ success: true, summary: { bullets: finalBullets, raw: summary } })
      } catch (err) {
        log.error('onPdfSummarize', (err as Error).message)
        respond({ success: false, error: (err as Error).message })
      }
    })()
  }

  static _onPdfExtractText(msg: { type: string; url: string }, respond: ResponseCallback): void {
    (async () => {
      try {
        const { url } = msg
        if (!url) {
          respond({ success: false, error: 'no url provided' })
          return
        }

        log.log('pdf extract text request:', url.slice(0, 80))

        const text = await pdfService.extractFromUrl(url)
        const wordCount = text.split(WHITESPACE_PATTERN).filter(Boolean).length

        respond({ success: true, text, wordCount })
      } catch (err) {
        log.error('onPdfExtractText', (err as Error).message)
        respond({ success: false, error: (err as Error).message })
      }
    })()
  }

  static _onOpenSidePanel(
    msg: { type: string; focus?: string },
    sender: chrome.runtime.MessageSender,
    respond: ResponseCallback
  ): void {

    const windowId = sender?.tab?.windowId
    
    if (!windowId) {
      chrome.windows.getCurrent((win) => {
        if (win?.id) {
          this._openPageTools(win.id)
            .then(() => {
              log.log('side panel opened (fallback)', msg.focus ? `focus: ${msg.focus}` : '')
              respond({ success: true })
            })
            .catch((err) => {
              log.error('onOpenSidePanel', err.message)
              respond({ success: false, error: err.message })
            })
        } else {
          respond({ success: false, error: 'no window available' })
        }
      })
      return
    }
    
    this._openPageTools(windowId, sender.tab?.id)
      .then(() => {
        log.log('side panel opened', msg.focus ? `focus: ${msg.focus}` : '')
        respond({ success: true })
      })
      .catch((err) => {
        log.error('onOpenSidePanel', err.message)
        respond({ success: false, error: err.message })
      })
  }

  static _onToggleSidePanel(
    msg: { type: string; focus?: string },
    sender: chrome.runtime.MessageSender,
    respond: ResponseCallback
  ): void {
    const windowId = sender?.tab?.windowId
    
    if (!windowId) {
      chrome.windows.getCurrent((win) => {
        if (win?.id) {
          this._openPageTools(win.id)
            .then(() => {
              log.log('side panel opened (toggle)', msg.focus ? `focus: ${msg.focus}` : '')
              respond({ success: true, action: 'opened' })
            })
            .catch(() => {
              chrome.runtime.sendMessage({ type: 'TOGGLE_SIDE_PANEL' }).catch(() => {})
              respond({ success: true, action: 'closed' })
            })
        } else {
          respond({ success: false, error: 'no window available' })
        }
      })
      return
    }
    
    this._openPageTools(windowId, sender.tab?.id)
      .then(() => {
        chrome.runtime.sendMessage({ type: 'TOGGLE_SIDE_PANEL' }).catch(() => {})
        log.log('side panel toggle', msg.focus ? `focus: ${msg.focus}` : '')
        respond({ success: true })
      })
      .catch((err) => {
        log.error('onToggleSidePanel', err.message)
        respond({ success: false, error: err.message })
      })
  }

  static _openPageTools(windowId: number, tabId?: number): Promise<void> {
    return new Promise((resolve, reject) => {
      const open = (selected?: number): void => {
        if (selected === undefined) {
          reject(new Error('No active tab is available.'))
          return
        }
        chrome.sidePanel.setOptions({ tabId: selected, path: 'chat.html', enabled: true }, () => {
          if (chrome.runtime.lastError) {
            reject(new Error(chrome.runtime.lastError.message))
            return
          }
          chrome.sidePanel.open({ tabId: selected }).then(resolve, reject)
        })
      }
      if (tabId !== undefined) open(tabId)
      else chrome.tabs.query({ active: true, windowId }, tabs => {
        if (chrome.runtime.lastError) reject(new Error(chrome.runtime.lastError.message))
        else open(tabs[0]?.id)
      })
    })
  }

  static _onPdfProcessArrayBuffer(
    msg: { type: string; data: number[]; filename: string; action: 'summarize' | 'copy'; sourceUrl?: string },
    respond: ResponseCallback
  ): void {
    (async () => {
      try {
        const { data, filename, action, sourceUrl } = msg
        const pdfUrl = sourceUrl || `file:///${filename}`
        log.log(`pdf process arraybuffer request: ${filename} (${action})`)
        
        const arrayBuffer = new Uint8Array(data).buffer
        
        if (action === 'summarize') {
          const result = await pdfService.summarizeFromArrayBuffer(arrayBuffer, filename)
          
          const bullets = result.summary
            .split('\n')
            .filter(line => line.trim().startsWith('•') || line.trim().startsWith('-') || line.trim().startsWith('*'))
            .map(line => line.replace(SUMMARY_BULLET_PREFIX_PATTERN, '').trim())
            .filter(Boolean)
          
          const finalBullets = bullets.length > 0 
            ? bullets 
            : result.summary.split(SUMMARY_SENTENCE_SEPARATOR_PATTERN).filter(s => s.trim().length > 20).slice(0, 5)
          
          respond({ success: true, summary: { bullets: finalBullets }, text: result.fullText })
          
          ;(async () => {
            try {
              const wordCount = result.fullText.split(WHITESPACE_PATTERN).length
              const readTimeMin = Math.max(1, Math.round(wordCount / 200))
              const summaryData = {
                title: filename.replace('.pdf', ''),
                bullets: finalBullets,
                readTime: `${readTimeMin} min`,
                wordCount,
                timestamp: Date.now(),
                timing: { total: 0, cached: false }
              }
              
              await cacheService.setPageSummary(pdfUrl, summaryData, 3600)
              log.log(`cached PDF summary: ${filename}`)
              
              const broadcastProgress = (percent: number) => {
                chrome.runtime.sendMessage({
                  type: 'INDEXING_PROGRESS',
                  sourceId: pdfUrl,
                  percent
                }).catch(() => {})
              }
              
              await concurrencyManager.execute('indexing', pdfUrl, async (signal) => {
                if (signal.aborted) throw new Error('Indexing aborted')
                await search.index(result.fullText, {
                  sourceId: pdfUrl,
                  sourceUrl: pdfUrl,
                  sourceType: 'pdf',
                  title: filename.replace('.pdf', '')
                }, broadcastProgress)
              })
              
              log.log(`background indexing complete: ${filename}`)
            } catch (bgErr) {
              log.warn('background PDF indexing failed', (bgErr as Error).message)
            }
          })()
          
        } else {
          const text = await pdfService.extractFromArrayBuffer(arrayBuffer)
          const wordCount = text.split(WHITESPACE_PATTERN).filter(Boolean).length
          respond({ success: true, text, wordCount })
        }
      } catch (err) {
        log.error('onPdfProcessArrayBuffer', (err as Error).message)
        respond({ success: false, error: (err as Error).message })
      }
    })()
  }

  static _onGetPageCache(msg: { type: string; url: string }, respond: ResponseCallback): void {
    (async () => {
      try {
        const { url } = msg
        if (!url) {
          respond({ summary: null })
          return
        }
        
        const cached = await cacheService.getPageSummary(url)
        respond({ summary: cached })
      } catch (err) {
        log.error('onGetPageCache', (err as Error).message)
        respond({ summary: null })
      }
    })()
  }
}

