import { databaseService, DB_CONFIGS } from './DatabaseService'
import { logger } from './LoggerService'

const log = logger.createScoped('CacheService')
const CACHE = DB_CONFIGS.cache
const PAGE_CACHE = 'page_cache'

// Page summaries the PDF tools keep by address; emails and pages are cached by the Rust core.
export class CacheService {
  async getPageSummary(url: string): Promise<unknown> {
    try {
      const result = await databaseService.get<{ summary?: unknown }>(CACHE, PAGE_CACHE, url)
      return result?.summary ?? null
    } catch (err) {
      log.error('getPageSummary failed', (err as Error).message)
      return null
    }
  }

  async setPageSummary(url: string, summary: unknown, ttlSeconds = 3600): Promise<void> {
    try {
      const ttl = ttlSeconds * 1000
      await databaseService.put(CACHE, PAGE_CACHE, { url, summary, timestamp: Date.now(), ttl })
    } catch (err) {
      log.error('setPageSummary failed', (err as Error).message)
    }
  }
}

export const cacheService = new CacheService()
