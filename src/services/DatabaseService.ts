interface DatabaseConfig {
  name: string
  version: number
  stores: readonly { readonly name: string; readonly keyPath: string }[]
}

type TransactionMode = 'readonly' | 'readwrite'

// the schema is exactly the config: stores it no longer names are deleted, and no store keeps an index
function upgrade(request: IDBOpenDBRequest, config: DatabaseConfig): void {
  const db = request.result
  const wanted = new Map(config.stores.map(store => [store.name, store.keyPath]))
  for (const name of Array.from(db.objectStoreNames)) {
    if (!wanted.has(name)) {
      db.deleteObjectStore(name)
      continue
    }
    const store = request.transaction!.objectStore(name)
    for (const index of Array.from(store.indexNames)) store.deleteIndex(index)
  }
  for (const [name, keyPath] of wanted) {
    if (!db.objectStoreNames.contains(name)) db.createObjectStore(name, { keyPath })
  }
}

class Databases {
  private databases: Map<string, IDBDatabase> = new Map()
  private initPromises: Map<string, Promise<IDBDatabase>> = new Map()

  async getDatabase(config: DatabaseConfig): Promise<IDBDatabase> {
    const existing = this.databases.get(config.name)
    if (existing?.version === config.version) return existing
    if (existing) {
      existing.close()
      this.databases.delete(config.name)
    }

    const pending = this.initPromises.get(config.name)
    if (pending) return pending

    const promise = this.openDatabase(config)
    this.initPromises.set(config.name, promise)

    try {
      const db = await promise
      this.databases.set(config.name, db)
      return db
    } finally {
      this.initPromises.delete(config.name)
    }
  }

  private openDatabase(config: DatabaseConfig): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(config.name, config.version)
      request.onerror = () => {
        console.error('[DatabaseService] open failed:', config.name, request.error)
        reject(request.error)
      }
      request.onsuccess = () => {
        const db = request.result
        db.onversionchange = () => {
          db.close()
          if (this.databases.get(config.name) === db) this.databases.delete(config.name)
        }
        resolve(db)
      }
      request.onupgradeneeded = () => upgrade(request, config)
    })
  }

  async get<T>(config: DatabaseConfig, storeName: string, key: string): Promise<T | null> {
    const db = await this.getDatabase(config)
    return new Promise((resolve) => {
      try {
        const request = db.transaction([storeName], 'readonly').objectStore(storeName).get(key)
        request.onsuccess = () => resolve(request.result ?? null)
        request.onerror = () => resolve(null)
      } catch {
        resolve(null)
      }
    })
  }

  async put<T>(config: DatabaseConfig, storeName: string, value: T): Promise<void> {
    const db = await this.getDatabase(config)
    return new Promise((resolve, reject) => {
      try {
        const request = db.transaction([storeName], 'readwrite').objectStore(storeName).put(value)
        request.onsuccess = () => resolve()
        request.onerror = () => reject(request.error)
      } catch (err) {
        reject(err)
      }
    })
  }

  async transaction<T>(
    config: DatabaseConfig,
    storeNames: readonly string[],
    mode: TransactionMode,
    callback: (transaction: IDBTransaction) => T | Promise<T>
  ): Promise<T> {
    const db = await this.getDatabase(config)
    const tx = db.transaction([...storeNames], mode)
    const complete = new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve()
      tx.onabort = () => reject(tx.error ?? new Error('Database transaction aborted'))
    })
    try {
      const [value] = await Promise.all([callback(tx), complete])
      return value
    } catch (error) {
      try { tx.abort() } catch { /* The transaction may already have finished. */ }
      await complete.catch(() => {})
      throw error
    }
  }
}

export const databaseService = new Databases()

export const EMAIL_STORES = {
  header: 'email_sessions',
  summary: 'email_summaries',
  replies: 'email_replies'
} as const

export const DB_CONFIGS = {
  cache: {
    name: 'metldr_cache',
    version: 9,
    stores: [
      ...Object.values(EMAIL_STORES).map(name => ({ name, keyPath: 'emailId' })),
      { name: 'page_cache', keyPath: 'url' },
      { name: 'rag_metadata', keyPath: 'sourceId' }
    ]
  }
} as const
