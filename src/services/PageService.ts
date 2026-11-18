import { cacheService } from './CacheService'
import { complete, pageSummary } from './RustCore'
import type { ExtractedData, PageSummary, ChatMessage, ChatResult } from '../types'

import { ragService } from './rag/RagService'

export class PageService {
  // The page tools' summary: the Rust core reads the cache, asks the model, keeps and counts it;
  // the page and a new summary are indexed for search here.
  static async summarize(extractedData: ExtractedData, force = false): Promise<PageSummary> {
    const { title, url, content } = extractedData
    const source = {
      sourceId: url || '',
      sourceUrl: url || '',
      sourceType: 'article' as const,
      title: title || url || 'Untitled Article'
    }
    if (url && content) {
      const progress = (percent: number) => {
        chrome.runtime.sendMessage({ type: 'INDEXING_PROGRESS', sourceId: url, percent }).catch(() => {})
      }
      const failed = (err: unknown) => console.warn('[PageService] chunk indexing failed', err)
      ragService.indexChunks(content, source, progress).catch(failed)
    }
    const summary = await pageSummary(extractedData, force)
    if (url && !summary.timing?.cached) {
      const failed = (err: unknown) => console.warn('[PageService] summary indexing failed', err)
      ragService.indexSummary(summary.bullets.join('\n'), source).catch(failed)
    }
    return summary
  }

  private static async _enhanceContextWithSummaries(
    context: string,
    sources: Array<{ index: number; url: string; type: string }>
  ): Promise<string> {
    const summaryParts: string[] = []

    for (let i = 0; i < sources.length; i += 8) {
      const summaries = await Promise.all(sources.slice(i, i + 8).map(async source => {
        if (!source.url) return null

        try {
          let summary: string | null = null

          if (source.type === 'email') {
            const emailId = source.url.replace('email://', '').split('/').pop()
            if (emailId) {
              const cached = await cacheService.getEmailSummary(emailId) as { bullets?: string[]; keyPoints?: string[] } | null
              if (cached?.bullets?.length) {
                summary = cached.bullets.join('. ')
              } else if (cached?.keyPoints?.length) {
                summary = cached.keyPoints.join('. ')
              }
            }
          } else {
            const cached = await cacheService.getPageSummary(source.url) as { bullets?: string[] } | null
            if (cached?.bullets?.length) {
              summary = cached.bullets.join('. ')
            }
          }

          return summary ? `[${source.index}] SUMMARY: ${summary}` : null
        } catch {
          return null
        }
      }))
      summaryParts.push(...summaries.filter((summary): summary is string => summary !== null))
    }
    
    if (summaryParts.length === 0) return context
    
    return `SUMMARIES:\n${summaryParts.join('\n')}\n\nDETAILED SOURCES:\n${context}`
  }

  static async globalChat(
    messages: ChatMessage[]
  ): Promise<ChatResult & { 
    sources?: Array<{ index: number; title: string; url: string; type: 'email' | 'page' | 'pdf'; score: number; snippet: string }>
  }> {
    const startTime = performance.now()
    const lastUserMsg = messages.filter(m => m.role === 'user').pop()?.content || ''
    
    if (!lastUserMsg) {
      return { ok: false, error: 'no user message' }
    }
    
    const { context, sources } = await ragService.searchWithSources(lastUserMsg, 8)
    console.log('[PageService.globalChat] sources from RAG:', sources?.length, sources)
    
    if (!context) {
      return { 
        ok: true, 
        content: "I don't have any indexed content to search. Try summarizing some emails, articles, or PDFs first.",
        sources: [],
        timing: { total: Math.round(performance.now() - startTime), model: 'none' }
      }
    }
    
    const enhancedContext = await this._enhanceContextWithSummaries(context, sources)
    
    const systemPrompt = `You are metldr, answering questions from the user's saved emails, articles, and documents.

SOURCES:
${enhancedContext}

RULES:
- Extract and state the key facts from each source (dates, amounts, names, order IDs, etc.)
- For each relevant source, write 1-2 sentences with the specific details found
- Cite once at end of each item: [1], [2], etc.
- Don't say "details not available" - if info exists in the source, extract it
- Be direct and factual, no fluff`

    // build chat history (exclude current message, it's passed as userPrompt)
    const chatHistory = messages
      .filter(m => m.content !== lastUserMsg)
      .slice(-6)
      .map(m => ({ role: m.role as 'user' | 'assistant', content: m.content }))

    try {
      const result = await complete({
        system: systemPrompt,
        messages: [...chatHistory, { role: 'user', content: lastUserMsg }]
      })
      return {
        ok: true,
        content: result.text,
        sources,
        timing: { total: Math.round(performance.now() - startTime), model: result.model }
      }
    } catch (err) {
      return { ok: false, error: (err as Error).message }
    }
  }
}



