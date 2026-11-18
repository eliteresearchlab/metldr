import { databaseService, DB_CONFIGS, EMAIL_STORES } from './DatabaseService'
import { logger } from './LoggerService'
import type { EmailSummary, ReplySuggestion } from '@/types'

const log = logger.createScoped('CacheService')
const CACHE = DB_CONFIGS.cache
const PAGE_CACHE = 'page_cache'
const EMAIL_STORE_NAMES = Object.values(EMAIL_STORES)

interface EmailPayload<T> {
  emailId: string
  value: T
}

interface EmailUpdate {
  summary?: EmailSummary | null
  replySuggestions?: ReplySuggestion[] | null
  emailCount?: number
}

function request<T>(idbRequest: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    idbRequest.onsuccess = () => resolve(idbRequest.result)
    idbRequest.onerror = () => reject(idbRequest.error)
  })
}

export class CacheService {
  // each field given is written in one transaction; null deletes it
  async updateEmailSession(emailId: string, updates: EmailUpdate): Promise<void> {
    try {
      await databaseService.transaction(CACHE, EMAIL_STORE_NAMES, 'readwrite', tx => {
        const keep = (store: string, value: unknown) => value == null
          ? tx.objectStore(store).delete(emailId)
          : tx.objectStore(store).put({ emailId, value })
        if ('summary' in updates) keep(EMAIL_STORES.summary, updates.summary)
        if ('replySuggestions' in updates) keep(EMAIL_STORES.replies, updates.replySuggestions)
        if (updates.emailCount !== undefined) {
          tx.objectStore(EMAIL_STORES.header).put({ emailId, emailCount: updates.emailCount })
        }
      })
    } catch (err) {
      log.error('updateEmailSession failed', (err as Error).message)
    }
  }

  private async payload<T>(store: string, emailId: string): Promise<T | null> {
    try {
      const row = await databaseService.get<EmailPayload<T | null>>(CACHE, store, emailId)
      return row?.value ?? null
    } catch (err) {
      log.error(`reading ${store} failed`, (err as Error).message)
      return null
    }
  }

  getEmailSummary(emailId: string): Promise<EmailSummary | null> {
    return this.payload(EMAIL_STORES.summary, emailId)
  }

  async getEmailCache(emailId: string): Promise<{ summary: EmailSummary | null; emailCount: number }> {
    const stores = [EMAIL_STORES.header, EMAIL_STORES.summary]
    try {
      return await databaseService.transaction(CACHE, stores, 'readonly', async tx => {
        const [header, payload] = await Promise.all([
          request<{ emailCount?: number } | undefined>(tx.objectStore(stores[0]).get(emailId)),
          request<EmailPayload<EmailSummary> | undefined>(tx.objectStore(stores[1]).get(emailId))
        ])
        return { summary: payload?.value ?? null, emailCount: header?.emailCount || 0 }
      })
    } catch (err) {
      log.error('getEmailCache failed', (err as Error).message)
      return { summary: null, emailCount: 0 }
    }
  }

  async setEmailSummary(emailId: string, summary: EmailSummary): Promise<void> {
    await this.updateEmailSession(emailId, { summary })
  }

  getReplySuggestions(emailId: string): Promise<ReplySuggestion[] | null> {
    return this.payload(EMAIL_STORES.replies, emailId)
  }

  async setReplySuggestions(emailId: string, suggestions: ReplySuggestion[]): Promise<void> {
    await this.updateEmailSession(emailId, { replySuggestions: suggestions })
  }

  async deleteReplySuggestions(emailId: string): Promise<void> {
    await this.updateEmailSession(emailId, { replySuggestions: null })
  }

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
