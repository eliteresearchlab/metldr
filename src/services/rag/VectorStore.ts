import { databaseService, DB_CONFIGS } from '../DatabaseService'
import type { VectorEntry } from '../../types'
import { SearchIndex, search_model_fetch } from '../../../crates/metldr-core/pkg/metldr_core'
import { loadRustCore } from '../RustCore'

// The search model downloads once on first use (see the core's model.rs); until it is ready,
// search runs on keywords, and the download is retried at most this often.
const MODEL_RETRY_MS = 30_000
// Every chunk's vector, saved as one blob so a restart embeds only new or changed chunks.
const VECTORS_KEY = 'system:vectors'
const SAVE_DELAY_MS = 300

interface SavedVectors { url: string; value: Uint8Array; timestamp: number }
const CHUNK_TTL_MS = 24 * 60 * 60 * 1000 * 60 // 60 days

// The stored chunks in IndexedDB and the Rust search index over them (keywords, meaning, fusion).
export class VectorStore {
  private operationQueue: Promise<void> = Promise.resolve()
  private pendingSave: ReturnType<typeof setTimeout> | null = null
  private loading: Promise<SearchIndex> | null = null
  private modelTried = 0

  // the index, loaded once: the model, the saved vectors, then every stored chunk
  private async index(): Promise<SearchIndex> {
    const index = await (this.loading ??= this.load().catch(error => {
      this.loading = null
      throw error
    }))
    if (!index.has_model() && Date.now() - this.modelTried > MODEL_RETRY_MS) this.fetchModel(index)
    return index
  }

  // a model that arrives after the chunks embeds them all, then saves their vectors
  private async fetchModel(index: SearchIndex): Promise<void> {
    this.modelTried = Date.now()
    if (!(await search_model_fetch()) || !index.use_search_model()) return
    await this.eachStored(entry => index.embed(entry.id, entry.content))
    this.scheduleSave()
  }

  private async load(): Promise<SearchIndex> {
    await loadRustCore()
    const index = new SearchIndex()
    this.modelTried = Date.now()
    if (await search_model_fetch()) index.use_search_model()
    const saved = await databaseService.get<SavedVectors>(DB_CONFIGS.cache, 'page_cache', VECTORS_KEY)
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
      const record: SavedVectors = { url: VECTORS_KEY, value, timestamp: Date.now() }
      await databaseService.put(DB_CONFIGS.cache, 'page_cache', record)
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
