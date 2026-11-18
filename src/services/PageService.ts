import { pageSummary, search } from './RustCore'
import type { ExtractedData, PageSummary } from '../types'

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
      search.index(content, source, progress).catch(failed)
    }
    const summary = await pageSummary(extractedData, force)
    if (url && !summary.timing?.cached) {
      const failed = (err: unknown) => console.warn('[PageService] summary indexing failed', err)
      search.indexSummary(summary.bullets.join('\n'), source).catch(failed)
    }
    return summary
  }
}
