import { vectorStore } from './VectorStore'
import { embeddingProvider } from './EmbeddingProvider'
import { chunkingService } from './ChunkingService'
import { databaseService, DB_CONFIGS } from '../DatabaseService'
import { aiGateway } from '../ai/AIGateway'
import { LRUCache } from 'lru-cache'
import type { VectorEntry } from '../../types'
import { expand_query_locally, focus_query, suggested_terms } from '../../../crates/metldr-core/pkg/metldr_core'
import { loadRustCore } from '../RustCore'

interface ChunkMetadata {
  sourceId: string
  sourceUrl: string
  sourceType: 'article' | 'email' | 'pdf'
  title?: string
}

interface RagMetadataEntry {
  sourceId: string
  contentHash: string
  chunkCount: number
  timestamp: number
}

interface IndexingStats {
  sourceId: string
  wallClockMs: number
  chunkCount: number
  embeddingMs: number
  storageMs: number
  skipped: boolean
  skipReason?: 'unchanged' | 'duplicate' | 'no_chunks' | 'backfilled'
}

interface Candidates {
  focused: string
  dense: string[]
}

interface SearchSource {
  index: number
  title: string
  url: string
  type: 'email' | 'page' | 'pdf'
  score: number
  snippet: string
}

const EMBEDDING_CONCURRENCY = 8
const STORE_RAG_METADATA = 'rag_metadata'
const QUERY_CACHE_MAX = 100
const QUERY_CACHE_TTL = 5 * 60 * 1000

export class RagService {
  private contentHashes = new Map<string, string>()
  private metadataLoaded = false
  private loadingPromise: Promise<void> | null = null
  private activeIndexing = new Map<string, Promise<void>>()
  // the slow half of a search (model expansion, embedding, dense neighbours), kept for repeats
  private queryCache = new LRUCache<string, Candidates>({
    max: QUERY_CACHE_MAX,
    ttl: QUERY_CACHE_TTL
  })

  private async ensureMetadataLoaded(): Promise<void> {
    if (this.metadataLoaded) return
    if (this.loadingPromise) return this.loadingPromise

    this.loadingPromise = (async () => {
      const startTime = Date.now()
      try {
        console.log('[RagService] Loading persisted metadata from IDB...')
        const entries = await databaseService.getAll<RagMetadataEntry>(DB_CONFIGS.cache, STORE_RAG_METADATA)
        
        for (const entry of entries) {
          this.contentHashes.set(entry.sourceId, entry.contentHash)
        }
        
        console.log(`[RagService] Loaded ${entries.length} metadata entries (${Date.now() - startTime}ms)`)
        this.metadataLoaded = true
      } catch (err) {
        console.error('[RagService] Failed to load metadata:', err)
        this.metadataLoaded = true // proceed anyway to avoid infinite loops
      }
    })()

    await this.loadingPromise
  }

  private async persistMetadata(sourceId: string, contentHash: string, chunkCount: number): Promise<void> {
    try {
      const entry: RagMetadataEntry = {
        sourceId,
        contentHash,
        chunkCount,
        timestamp: Date.now()
      }
      await databaseService.put(DB_CONFIGS.cache, STORE_RAG_METADATA, entry)
      console.log(`[RagService] Persisted metadata for ${sourceId.slice(0, 50)}`)
    } catch (err) {
      console.error('[RagService] Failed to persist metadata:', err)
    }
  }

  async index(entry: VectorEntry): Promise<void> {
    const startTime = Date.now()
    try {
      console.log(`[RagService] Indexing single entry: ${entry.id}`)
      const embedding = await embeddingProvider.embedDocument(entry.content)
      await vectorStore.add(entry, embedding)
      console.log(`[RagService] Indexed ${entry.id} (${Date.now() - startTime}ms)`)
    } catch (err) {
      console.error('[RagService] Single indexing failed:', err)
    }
  }

  async indexChunks(text: string, metadata: ChunkMetadata, onProgress?: (percent: number) => void): Promise<void> {
    const wallClockStart = Date.now()
    await this.ensureMetadataLoaded()

    // check for concurrent indexing of same source
    const existing = this.activeIndexing.get(metadata.sourceId)
    if (existing) {
      console.log(`[RagService] Already indexing ${metadata.sourceId.slice(0, 50)}, waiting...`)
      await existing
      return
    }

    const indexPromise = this._doIndexChunks(text, metadata, wallClockStart, onProgress)
    this.activeIndexing.set(metadata.sourceId, indexPromise)
    
    try {
      await indexPromise
    } finally {
      this.activeIndexing.delete(metadata.sourceId)
    }
  }

  private async _doIndexChunks(text: string, metadata: ChunkMetadata, wallClockStart: number, onProgress?: (percent: number) => void): Promise<void> {
    const stats: IndexingStats = {
      sourceId: metadata.sourceId,
      wallClockMs: 0,
      chunkCount: 0,
      embeddingMs: 0,
      storageMs: 0,
      skipped: false
    }

    const report = (p: number) => onProgress?.(p)

    try {
      report(5) // starting
      const contentHash = this.hashContent(text)
      const existingHash = this.contentHashes.get(metadata.sourceId)
      
      // check if content unchanged (persisted deduplication)
      if (existingHash === contentHash) {
        // verify VOY index actually has this data
        const voyHasData = await this.verifyVoyIndex(metadata.sourceId)
        if (voyHasData) {
          stats.skipped = true
          stats.skipReason = 'unchanged'
          stats.wallClockMs = Date.now() - wallClockStart
          report(100) // done (cached)
          console.log(`[RagService] SKIPPED: ${metadata.sourceId.slice(0, 50)} (content unchanged, VOY verified) [${stats.wallClockMs}ms]`)
          return
        } else {
          console.log(`[RagService] Metadata says indexed but VOY empty, re-indexing...`)
        }
      } else if (!existingHash) {
        // no metadata entry - check if VOY/IDB has this doc (backfill case)
        const voyHasData = await this.verifyVoyIndex(metadata.sourceId)
        if (voyHasData) {
          // backfill metadata and skip re-indexing
          this.contentHashes.set(metadata.sourceId, contentHash)
          await this.persistMetadata(metadata.sourceId, contentHash, 0) // chunk count unknown
          stats.skipped = true
          stats.skipReason = 'backfilled'
          stats.wallClockMs = Date.now() - wallClockStart
          report(100)
          console.log(`[RagService] SKIPPED: ${metadata.sourceId.slice(0, 50)} (VOY has data, backfilled metadata) [${stats.wallClockMs}ms]`)
          return
        }
      } else {
        // content hash changed - will re-index
        console.log(`[RagService] Content changed for ${metadata.sourceId.slice(0, 50)}, re-indexing...`)
      }

      report(10) // chunking
      const contentType = metadata.sourceType === 'email' ? 'email' : metadata.sourceType === 'pdf' ? 'pdf' : 'article'
      const chunks = await chunkingService.chunkForEmbedding(text, contentType as 'email' | 'article' | 'pdf')
      if (chunks.length === 0) {
        stats.skipped = true
        stats.skipReason = 'no_chunks'
        stats.wallClockMs = Date.now() - wallClockStart
        report(100)
        console.log(`[RagService] SKIPPED: No chunks to index [${stats.wallClockMs}ms]`)
        return
      }
      
      report(20) // chunks ready
      stats.chunkCount = chunks.length
      console.log(`[RagService] INDEXING: ${chunks.length} chunks for ${metadata.sourceId.slice(0, 50)}`)
      
      // create vector entries
      const entries: VectorEntry[] = chunks.map(chunk => ({
        id: `${metadata.sourceType}:${metadata.sourceId}:chunk:${chunk.index}`,
        type: metadata.sourceType,
        content: chunk.text,
        metadata: {
          sourceId: metadata.sourceId,
          sourceUrl: metadata.sourceUrl,
          title: metadata.title || '',
          chunkIndex: chunk.index,
          totalChunks: chunks.length
        },
        timestamp: Date.now()
      }))

      // embed and store with progress (20% -> 90%)
      const embedStart = Date.now()
      await this.embedAndStoreBatchedWithProgress(entries, EMBEDDING_CONCURRENCY, (batchPercent) => {
        const scaledProgress = 20 + Math.round(batchPercent * 0.7) // 20-90%
        report(scaledProgress)
      })
      stats.embeddingMs = Date.now() - embedStart
      
      report(92) // persisting
      const persistStart = Date.now()
      this.contentHashes.set(metadata.sourceId, contentHash)
      await this.persistMetadata(metadata.sourceId, contentHash, chunks.length)
      
      report(96)
      await vectorStore.forceSave()
      stats.storageMs = Date.now() - persistStart
      
      stats.wallClockMs = Date.now() - wallClockStart
      
      // invalidate query cache (new content indexed)
      this.queryCache.clear()
      
      report(100) // done
      console.log(`[RagService] INDEXED: ${chunks.length} chunks for ${metadata.sourceId.slice(0, 50)}`)
      console.log(`[RagService] Timing: total=${stats.wallClockMs}ms, embed=${stats.embeddingMs}ms, persist=${stats.storageMs}ms`)
    } catch (err) {
      stats.wallClockMs = Date.now() - wallClockStart
      report(100) // end on error too
      console.error(`[RagService] Chunk indexing FAILED after ${stats.wallClockMs}ms:`, err)
    }
  }

  // verify that VOY index actually has embeddings for this source
  private async verifyVoyIndex(sourceId: string): Promise<boolean> {
    try {
      const docCount = vectorStore.getDocCount()
      if (docCount === 0) {
        console.log('[RagService] VOY verification: index is empty')
        return false
      }
      
      const hasDoc = await vectorStore.hasDocument(sourceId)
      console.log(`[RagService] VOY verification: hasDoc=${hasDoc}, totalDocs=${docCount}`)
      return hasDoc
    } catch (err) {
      console.warn('[RagService] VOY verification error:', err)
      return false
    }
  }

  private hashContent(text: string): string {
    const first = text.slice(0, 200)
    const last = text.slice(-200)
    return `${text.length}:${first}:${last}`
  }

  // batch embed with progress callback for UI updates
  private async embedAndStoreBatchedWithProgress(
    entries: VectorEntry[], 
    batchSize: number, 
    onBatchProgress?: (percent: number) => void
  ): Promise<void> {
    const totalBatches = Math.ceil(entries.length / batchSize)
    
    for (let i = 0; i < entries.length; i += batchSize) {
      const batch = entries.slice(i, i + batchSize)
      const texts = batch.map(e => e.content)
      
      // true batch embedding (GPU processes all at once)
      const embeddings = await embeddingProvider.embedBatch(texts, false)
      
      // one vector-store batch per embedded batch: one IDB transaction and one sandbox message
      await vectorStore.addBatch(batch.map((entry, idx) => ({ entry, embedding: embeddings[idx] })))
      
      // report progress after each batch
      const batchNum = Math.floor(i / batchSize) + 1
      const percent = Math.round((batchNum / totalBatches) * 100)
      onBatchProgress?.(percent)
    }
  }

  // expansion and embedding for `query`, then the dense neighbours of `5 × candidates` hits
  private async candidates(query: string, candidates: number): Promise<Candidates> {
    const cacheKey = `${query}:${candidates}`
    const cached = this.queryCache.get(cacheKey)
    if (cached) return cached

    await loadRustCore()
    const focused = focus_query(await this.expandQueryWithLLM(query))
    const queryEmbedding = await embeddingProvider.embedQuery(focused)
    const found = { focused, dense: await vectorStore.dense(Array.from(queryEmbedding), candidates * 5) }
    this.queryCache.set(cacheKey, found)
    return found
  }

  // llm-based query expansion for global search, with a local synonym expansion as fallback
  private async expandQueryWithLLM(query: string): Promise<string> {
    try {
      const response = await aiGateway.complete({
        systemPrompt: `You are a search query optimizer. Output ONLY comma-separated search terms, no explanation.
RULES:
1. If user asks about what they "read", "saw", "forgot" - extract the TOPIC keywords only
2. Ignore personal phrases like "what was the pdf i read about" - focus on the subject
3. Generate 8-12 diverse terms: synonyms, related concepts, different phrasings
Example: "what was the pdf i read about islam and ummah" → "islam, ummah, muslim, islamic, community, faith, prophetic, religion"`,
        userPrompt: `Query: "${query}"`,
        maxTokens: 100
      })
      const terms = response.ok && response.content ? suggested_terms(response.content) : ''
      if (terms) return `${query} ${terms}`
    } catch (err) {
      console.log('[RagService] LLM expansion failed, using fallback:', err)
    }
    return expand_query_locally(query)
  }

  // page chat context: the best chunks for `query`, only from `sourceUrl` when given
  async searchWithContext(query: string, limit = 3, sourceUrl?: string): Promise<string> {
    try {
      const { focused, dense } = await this.candidates(query, limit * 4)
      return (await vectorStore.keywords()).context(focused, dense, limit, sourceUrl ?? '')
    } catch (err) {
      console.error('[RagService] searchWithContext failed:', err)
      return ''
    }
  }

  // global search: one chunk per page with numbered sources, scored up by the question's keywords
  async searchWithSources(query: string, limit = 5): Promise<{ context: string; sources: SearchSource[] }> {
    try {
      const { focused, dense } = await this.candidates(query, limit * 4)
      return JSON.parse((await vectorStore.keywords()).sources(query, focused, dense, limit))
    } catch (err) {
      console.error('[RagService] searchWithSources failed:', err)
      return { context: '', sources: [] }
    }
  }

  async hasIndexedContent(sourceUrl: string): Promise<boolean> {
    await this.ensureMetadataLoaded()
    
    // check metadata first (fast)
    if (this.contentHashes.has(sourceUrl)) {
      // verify VOY actually has the data
      const voyHasData = await this.verifyVoyIndex(sourceUrl)
      if (voyHasData) {
        return true
      }
      console.log(`[RagService] hasIndexedContent: metadata exists but VOY empty for ${sourceUrl.slice(0, 50)}`)
    }

    // fallback: direct document check (more reliable than keyword search)
    return await vectorStore.hasDocument(sourceUrl)
  }

  async indexSummary(summaryText: string, metadata: ChunkMetadata): Promise<void> {
    if (!summaryText?.trim()) return
    
    const summarySourceId = metadata.sourceId + ':summary'
    
    // check if already indexed
    await this.ensureMetadataLoaded()
    if (this.contentHashes.has(summarySourceId)) {
      console.log(`[RagService] Summary already indexed for ${metadata.sourceId.slice(0, 40)}`)
      return
    }
    
    const summaryMetadata: ChunkMetadata = {
      ...metadata,
      sourceId: summarySourceId,
      title: 'Summary: ' + (metadata.title || metadata.sourceUrl)
    }
    
    // create single entry for summary
    const entry: VectorEntry = {
      id: `${metadata.sourceType}:${summarySourceId}`,
      type: metadata.sourceType,
      content: summaryText,
      metadata: {
        ...summaryMetadata,
        isSummary: true
      },
      timestamp: Date.now()
    }
    
    try {
      const embedding = await embeddingProvider.embedDocument(summaryText)
      await vectorStore.add(entry, embedding)
      
      // persist metadata
      this.contentHashes.set(summarySourceId, this.hashContent(summaryText))
      await this.persistMetadata(summarySourceId, this.hashContent(summaryText), 1)
      
      console.log(`[RagService] Indexed summary for ${metadata.sourceId.slice(0, 40)}`)
    } catch (err) {
      console.error('[RagService] Summary indexing failed:', err)
    }
  }

  isIndexing(sourceId: string): boolean {
    return this.activeIndexing.has(sourceId)
  }

  // unified entry point: check if indexed, wait if in-progress, or index if needed
  async ensureIndexed(
    text: string, 
    metadata: ChunkMetadata, 
    onProgress?: (percent: number) => void
  ): Promise<'already-indexed' | 'was-indexing' | 'indexed-now'> {
    await this.ensureMetadataLoaded()

    // check if already indexed
    const hasContent = await this.hasIndexedContent(metadata.sourceId)
    if (hasContent) {
      console.log(`[RagService] ensureIndexed: already indexed ${metadata.sourceId.slice(0, 50)}`)
      return 'already-indexed'
    }

    // check if indexing in progress
    const existing = this.activeIndexing.get(metadata.sourceId)
    if (existing) {
      console.log(`[RagService] ensureIndexed: waiting on in-progress ${metadata.sourceId.slice(0, 50)}`)
      await existing
      return 'was-indexing'
    }

    await this.indexChunks(text, metadata, onProgress)
    return 'indexed-now'
  }

  async getIndexingStatus(sourceId: string): Promise<'indexed' | 'in-progress' | 'needed'> {
    if (this.activeIndexing.has(sourceId)) return 'in-progress'
    
    await this.ensureMetadataLoaded()
    const hasContent = await this.hasIndexedContent(sourceId)
    return hasContent ? 'indexed' : 'needed'
  }
}

export const ragService = new RagService()

