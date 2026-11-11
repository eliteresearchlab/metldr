import { databaseService, DB_CONFIGS } from '../DatabaseService'
import { localModels } from '../ai/LocalModelProvider'
import type { VectorEntry } from '../../types'
import { Bm25Index } from './bm25'
import type { RankedHit } from './rankFusion'

export class VectorStore {
  private operationQueue: Promise<void> = Promise.resolve()
  private pendingSaveTimeout: ReturnType<typeof setTimeout> | null = null
  private saveDebounceMs = 300
  private loaded = false
  private loadingPromise: Promise<void> | null = null
  private invertedIndex = new Bm25Index<VectorEntry>()
  private pendingAdds = 0
  
  // sandbox lifecycle tracking
  private lastSandboxId: string | null = null
  private voyIndexLoadedAt: number = 0

  async add(entry: VectorEntry, embedding: Float32Array): Promise<void> {
    this.pendingAdds++
    this.operationQueue = this.operationQueue.then(async () => {
      await this.ensureIndexLoaded()
      await this.storeDocument(entry)
      this.invertedIndex.add(entry.id, entry.content, entry)
      await localModels.voyAdd(entry.id, Array.from(embedding), String(entry.metadata?.title || entry.id), String(entry.metadata?.sourceUrl || ''))
      this.pendingAdds--
      this.scheduleDebouncedSave()
    }).catch(err => {
      this.pendingAdds--
      console.error('[VectorStore] Add failed:', err)
    })
    return this.operationQueue
  }

  // dense leg: Voy returns neighbours nearest-first and no similarity value, so the position is the only signal
  async search(queryEmbedding: number[], limit: number): Promise<Array<RankedHit<VectorEntry>>> {
    await this.ensureIndexLoaded()
    const ids = await localModels.voySearch(queryEmbedding, limit)

    const hydrated = await Promise.all(ids.map(async id => {
      const entry = await this.retrieveDocument(id)
      return entry ? { id, item: entry } : null
    }))

    return hydrated.filter((hit): hit is RankedHit<VectorEntry> => hit !== null)
  }

  // lexical leg: BM25 hits, best first
  async searchKeyword(query: string, limit: number): Promise<Array<RankedHit<VectorEntry>>> {
    await this.ensureIndexLoaded()
    return this.invertedIndex.search(query, limit).map(({ id, item }) => ({ id, item }))
  }

  async addBatch(entries: Array<{ entry: VectorEntry; embedding: Float32Array }>): Promise<void> {
    console.log(`[VectorStore] Batch adding ${entries.length} entries...`)
    if (entries.length > 0) {
      // one queue operation: one IDB transaction and one sandbox message for the whole batch
      this.pendingAdds += entries.length
      this.operationQueue = this.operationQueue.then(async () => {
        await this.ensureIndexLoaded()
        await this.storeDocuments(entries.map(({ entry }) => entry))
        for (const { entry } of entries) this.invertedIndex.add(entry.id, entry.content, entry)
        await localModels.voyAddBatch(entries.map(({ entry, embedding }) => ({
          id: entry.id,
          embedding: Array.from(embedding),
          title: String(entry.metadata?.title || entry.id),
          url: String(entry.metadata?.sourceUrl || '')
        })))
        // same save schedule as add(): one debounced save request per entry, never a synchronous save
        for (let i = 0; i < entries.length; i++) {
          this.pendingAdds--
          this.scheduleDebouncedSave()
        }
      }).catch(err => {
        this.pendingAdds -= entries.length
        console.error('[VectorStore] Batch add failed:', err)
      })
      await this.operationQueue
    }
    console.log(`[VectorStore] Batch add complete.`)
  }

  // check if sandbox was recreated and we need to reload voy index
  private async checkSandboxLifecycle(): Promise<boolean> {
    try {
      const currentSandboxId = localModels.getSandboxId()
      if (this.lastSandboxId && currentSandboxId !== this.lastSandboxId) {
        console.log('[VectorStore] Sandbox was recreated, need to reload VOY index')
        this.lastSandboxId = currentSandboxId
        return true
      }
      this.lastSandboxId = currentSandboxId
      return false
    } catch {
      return false
    }
  }

  private async ensureIndexLoaded(): Promise<void> {
    // if already loaded and sandbox hasn't changed, return immediately
    if (this.loaded) {
      const sandboxRecreated = await this.checkSandboxLifecycle()
      if (!sandboxRecreated) return
      // sandbox changed, need to reload - reset state
      console.log('[VectorStore] Forcing reload due to sandbox recreation')
      this.loaded = false
      this.loadingPromise = null
    }

    // if another load is in progress, wait for it
    if (this.loadingPromise) return this.loadingPromise

    // set loading promise SYNCHRONOUSLY before any async work
    // this prevents race conditions when multiple calls come in
    this.loadingPromise = this._doLoad()
    await this.loadingPromise
  }

  private async _doLoad(): Promise<void> {
    const startTime = Date.now()
    console.log('[VectorStore] Loading index...')
    
    // load voy index from IDB
    const record = await databaseService.get<any>(DB_CONFIGS.cache, 'page_cache', 'system:voy_index')
    if (record && record.value) {
      console.log('[VectorStore] Found persisted VOY index, loading into sandbox...')
      try {
        // handle both compressed and legacy uncompressed data
        let indexData = record.value as Uint8Array
        if (record.compressed) {
          indexData = await this.decompress(indexData)
          console.log(`[VectorStore] Decompressed VOY index: ${record.value.length} → ${indexData.length} bytes`)
        }
        // convert bytes back to string for voyLoad
        const indexStr = new TextDecoder().decode(indexData)
        await localModels.voyLoad(indexStr)
        this.voyIndexLoadedAt = Date.now()
        console.log(`[VectorStore] VOY index loaded from IDB (${Date.now() - startTime}ms)`)
      } catch (err) {
        console.error('[VectorStore] VOY load failed, starting fresh:', err)
      }
    } else {
      console.log('[VectorStore] No persisted VOY index found, starting fresh.')
    }
    
    // rebuild inverted index from stored documents
    await this.rebuildInvertedIndex()
    
    this.loaded = true
    this.lastSandboxId = localModels.getSandboxId()
    console.log(`[VectorStore] Index ready. ${this.invertedIndex.size} docs in keyword index. (${Date.now() - startTime}ms total)`)
  }

  // force reload from IDB - call when index seems stale
  async forceReload(): Promise<void> {
    console.log('[VectorStore] Force reloading index...')
    this.loaded = false
    this.loadingPromise = null
    await this.ensureIndexLoaded()
  }

  async hasDocument(sourceId: string): Promise<boolean> {
    await this.ensureIndexLoaded()
    
    // direct metadata check in the keyword index (O(n) but reliable)
    if (this.hasIndexedSource(sourceId)) {
      return true
    }
    
    const counts = await Promise.all(['article', 'pdf', 'email'].map(type =>
      databaseService.count(DB_CONFIGS.cache, 'page_cache', `rag:${type}:${sourceId}:chunk:0`)
    ))
    return counts.some(count => count > 0)
  }

  // get doc count for verification
  getDocCount(): number {
    return this.invertedIndex.size
  }

  // check if any document has this sourceUrl in metadata (fast O(n) scan, but docs are small)
  private hasIndexedSource(sourceUrl: string): boolean {
    for (const entry of this.invertedIndex.values()) {
      if (entry.metadata?.sourceUrl === sourceUrl || entry.metadata?.sourceId === sourceUrl) {
        return true
      }
    }
    return false
  }

  private async rebuildInvertedIndex(): Promise<void> {
    console.log('[VectorStore] Rebuilding inverted index from IDB...')
    this.invertedIndex.clear()
    const db = await databaseService.getDatabase(DB_CONFIGS.cache)
    const tx = db.transaction('page_cache', 'readonly')
    const store = tx.objectStore('page_cache')
    const cursorRequest = store.openCursor(IDBKeyRange.bound('rag:', 'rag;', false, true))

    return new Promise<void>((resolve) => {
      let count = 0
      cursorRequest.onsuccess = () => {
        const cursor = cursorRequest.result
        if (cursor) {
          const entry = cursor.value as VectorEntry
          this.invertedIndex.add(entry.id, entry.content, entry)
          count++
          cursor.continue()
        } else {
          console.log(`[VectorStore] Inverted index rebuilt with ${count} documents.`)
          resolve()
        }
      }
      cursorRequest.onerror = () => {
        console.error('[VectorStore] Cursor error during rebuild')
        resolve()
      }
    })
  }

  private scheduleDebouncedSave(): void {
    if (this.pendingSaveTimeout) clearTimeout(this.pendingSaveTimeout)
    
    const delay = this.pendingAdds > 0 ? this.saveDebounceMs : 50
    this.pendingSaveTimeout = setTimeout(() => {
      this.saveIndex()
      this.pendingSaveTimeout = null
    }, delay)
  }

  async forceSave(): Promise<void> {
    if (this.pendingSaveTimeout) {
      clearTimeout(this.pendingSaveTimeout)
      this.pendingSaveTimeout = null
    }
    await this.saveIndex()
  }

  private async saveIndex(): Promise<void> {
    if (!this.loaded) return
    
    console.log('[VectorStore] Saving VOY index (compressed) to IDB...')
    try {
      // voySerialize now returns string directly (survives chrome messaging)
      const serializedStr = await localModels.voySerialize()
      
      if (!serializedStr) {
        console.log('[VectorStore] VOY index empty, skipping save')
        return
      }
      
      // convert string to bytes for compression
      const index = new TextEncoder().encode(serializedStr)
      
      // gzip compress for 60-80% smaller storage
      const compressed = await this.compress(index)
      
      await databaseService.put(DB_CONFIGS.cache, 'page_cache', {
        url: 'system:voy_index',
        value: compressed,
        compressed: true,
        timestamp: Date.now()
      })
      console.log(`[VectorStore] VOY index saved: ${index.length} → ${compressed.length} bytes (${Math.round((1 - compressed.length / index.length) * 100)}% reduction)`)
    } catch (err) {
      console.error('[VectorStore] Save failed:', err)
    }
  }

  private async compress(data: Uint8Array): Promise<Uint8Array> {
    const stream = new Blob([data as BlobPart]).stream().pipeThrough(new CompressionStream('gzip'))
    return new Uint8Array(await new Response(stream).arrayBuffer())
  }

  private async decompress(data: Uint8Array): Promise<Uint8Array> {
    const stream = new Blob([data as BlobPart]).stream().pipeThrough(new DecompressionStream('gzip'))
    return new Uint8Array(await new Response(stream).arrayBuffer())
  }

  private async storeDocument(entry: VectorEntry): Promise<void> {
    await databaseService.put(DB_CONFIGS.cache, 'page_cache', {
      url: `rag:${entry.id}`,
      ...entry,
      timestamp: Date.now(),
      ttl: 24 * 60 * 60 * 1000 * 60 // 60 days
    })
  }

  // one IDB transaction for the whole batch (putBatch splits only above its chunk size)
  private async storeDocuments(entries: VectorEntry[]): Promise<void> {
    await databaseService.putBatch(DB_CONFIGS.cache, 'page_cache', entries.map(entry => ({
      url: `rag:${entry.id}`,
      ...entry,
      timestamp: Date.now(),
      ttl: 24 * 60 * 60 * 1000 * 60 // 60 days
    })), entries.length)
  }

  private async retrieveDocument(id: string): Promise<VectorEntry | undefined> {
    const item = await databaseService.get<any>(DB_CONFIGS.cache, 'page_cache', `rag:${id}`)
    return item as VectorEntry | undefined
  }
}

export const vectorStore = new VectorStore()
