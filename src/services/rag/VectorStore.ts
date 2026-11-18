import { databaseService, DB_CONFIGS } from '../DatabaseService'
import type { VectorEntry } from '../../types'
import { SearchIndex } from '../../../crates/metldr-core/pkg/metldr_core'
import { loadRustCore } from '../RustCore'

// The static embedding model (potion-retrieval-32M, int8), packed by crates/metldr-embed.
const MODEL_PATH = 'models/potion-retrieval-32M.metldr'
// Every chunk's vector, saved as one blob so a restart embeds only new or changed chunks.
const VECTORS_KEY = 'system:vectors'
const SAVE_DELAY_MS = 300
const CHUNK_TTL_MS = 24 * 60 * 60 * 1000 * 60 // 60 days

// The stored chunks in IndexedDB and the Rust search index over them (keywords, meaning, fusion).
export class VectorStore {
  private operationQueue: Promise<void> = Promise.resolve()
  private pendingSave: ReturnType<typeof setTimeout> | null = null
  private loading: Promise<SearchIndex> | null = null

  // the index, loaded once: the model, the saved vectors, then every stored chunk
  private index(): Promise<SearchIndex> {
    return this.loading ??= this.load().catch(error => {
      this.loading = null
      throw error
    })
  }

  private async load(): Promise<SearchIndex> {
    await loadRustCore()
    const index = new SearchIndex()
    const model = await fetch(chrome.runtime.getURL(MODEL_PATH))
    if (model.ok) index.load_model(new Uint8Array(await model.arrayBuffer()))
    const saved = await databaseService.get<{ value: Uint8Array }>(DB_CONFIGS.cache, 'page_cache', VECTORS_KEY)
    if (saved?.value) index.restore_vectors(saved.value)
    await this.eachStored(entry => this.remember(index, entry))
    this.scheduleSave()
    return index
  }

  async add(entry: VectorEntry): Promise<void> {
    return this.addBatch([entry])
  }

  // one IDB transaction for the batch; the index embeds each chunk as it is added
  async addBatch(entries: VectorEntry[]): Promise<void> {
    if (entries.length === 0) return
    this.operationQueue = this.operationQueue.then(async () => {
      const index = await this.index()
      await this.storeDocuments(entries)
      for (const entry of entries) this.remember(index, entry)
      this.scheduleSave()
    }).catch(err => console.error('[VectorStore] Add failed:', err))
    return this.operationQueue
  }

  // the loaded Rust index, which ranks, filters and assembles search results
  keywords(): Promise<SearchIndex> {
    return this.index()
  }

  async hasDocument(sourceId: string): Promise<boolean> {
    if ((await this.index()).has_source(sourceId)) return true
    const counts = await Promise.all(['article', 'pdf', 'email'].map(type =>
      databaseService.count(DB_CONFIGS.cache, 'page_cache', `rag:${type}:${sourceId}:chunk:0`)
    ))
    return counts.some(count => count > 0)
  }

  async forceSave(): Promise<void> {
    if (this.pendingSave) clearTimeout(this.pendingSave)
    this.pendingSave = null
    await this.save()
  }

  private remember(index: SearchIndex, entry: VectorEntry): void {
    const meta = entry.metadata ?? {}
    index.add(entry.id, entry.type, String(meta.title ?? ''), entry.content,
      String(meta.sourceUrl ?? ''), String(meta.sourceId ?? ''), meta.isSummary === true)
  }

  private scheduleSave(): void {
    if (this.pendingSave) clearTimeout(this.pendingSave)
    this.pendingSave = setTimeout(() => {
      this.pendingSave = null
      this.save()
    }, SAVE_DELAY_MS)
  }

  private async save(): Promise<void> {
    if (!this.loading) return
    try {
      const value = (await this.index()).vectors()
      await databaseService.put(DB_CONFIGS.cache, 'page_cache', { url: VECTORS_KEY, value, timestamp: Date.now() })
    } catch (err) {
      console.error('[VectorStore] Save failed:', err)
    }
  }

  private eachStored(visit: (entry: VectorEntry) => void): Promise<void> {
    return databaseService.getDatabase(DB_CONFIGS.cache).then(db => new Promise<void>(resolve => {
      const range = IDBKeyRange.bound('rag:', 'rag;', false, true)
      const cursor = db.transaction('page_cache', 'readonly').objectStore('page_cache').openCursor(range)
      cursor.onsuccess = () => {
        const at = cursor.result
        if (!at) return resolve()
        visit(at.value as VectorEntry)
        at.continue()
      }
      cursor.onerror = () => resolve()
    }))
  }

  private async storeDocuments(entries: VectorEntry[]): Promise<void> {
    const timestamp = Date.now()
    await databaseService.putBatch(DB_CONFIGS.cache, 'page_cache', entries.map(entry => ({
      url: `rag:${entry.id}`,
      ...entry,
      timestamp,
      ttl: CHUNK_TTL_MS
    })), entries.length)
  }
}

export const vectorStore = new VectorStore()
