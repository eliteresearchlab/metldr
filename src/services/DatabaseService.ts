interface DatabaseConfig {
  name: string
  version: number
  stores: readonly { readonly name: string; readonly keyPath: string }[]
}

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
}

export const databaseService = new Databases()

export const DB_CONFIGS = {
  cache: {
    name: 'metldr_cache',
    version: 9,
    stores: [
      // email summaries, replies and message counts, read and written by the Rust core
      ...['email_sessions', 'email_summaries', 'email_replies'].map(name => ({ name, keyPath: 'emailId' })),
      { name: 'page_cache', keyPath: 'url' },
      { name: 'rag_metadata', keyPath: 'sourceId' }
    ]
  }
} as const
