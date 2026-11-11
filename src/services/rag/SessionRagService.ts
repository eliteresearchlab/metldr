// session-scoped RAG - in-memory, per-document, ephemeral
// indexes current document for chat without truncation

import { chunkingService } from './ChunkingService'
import { embeddingProvider } from './EmbeddingProvider'
import { Bm25Index } from './bm25'
import { reciprocalRankFusion, type RankedHit } from './rankFusion'

interface SessionChunk {
  id: string
  text: string
  index: number
  embedding: number[]
}

// candidate depth per leg, the same as RagService (limit * 5)
const CANDIDATE_MULTIPLIER = 5

class SessionRagService {
  private chunks: SessionChunk[] = []
  private keywordIndex = new Bm25Index<SessionChunk>()
  private indexed = false
  private indexingPromise: Promise<void> | null = null
  private contentHash = ''
  
  // quick hash to detect content changes
  private hash(text: string): string {
    return `${text.length}:${text.slice(0, 100)}:${text.slice(-100)}`
  }

  // index document - call once per session/document
  async indexDocument(content: string): Promise<void> {
    if (!content || content.trim().length < 50) return
    
    const newHash = this.hash(content)
    if (this.indexed && this.contentHash === newHash) {
      console.log('[SessionRag] Content unchanged, skipping re-index')
      return
    }

    // if already indexing, wait for it
    if (this.indexingPromise) {
      await this.indexingPromise
      if (this.contentHash === newHash) return
    }

    this.indexingPromise = this._doIndex(content, newHash)
    await this.indexingPromise
  }

  private async _doIndex(content: string, hash: string): Promise<void> {
    const start = performance.now()
    console.log('[SessionRag] Indexing document...', content.length, 'chars')
    
    this.clear()
    
    // chunk content
    const chunkResults = await chunkingService.chunkForEmbedding(content)
    if (chunkResults.length === 0) {
      console.log('[SessionRag] No chunks generated')
      return
    }
    
    console.log('[SessionRag] Generated', chunkResults.length, 'chunks, embedding...')
    
    // embed in batches of 8 for speed (GPU parallelism)
    const BATCH_SIZE = 8
    for (let i = 0; i < chunkResults.length; i += BATCH_SIZE) {
      const batch = chunkResults.slice(i, i + BATCH_SIZE)
      const texts = batch.map(c => c.text)
      
      try {
        const embeddings = await embeddingProvider.embedBatch(texts, false)
        
        for (let j = 0; j < batch.length; j++) {
          const chunk = batch[j]
          const record: SessionChunk = {
            id: `chunk:${chunk.index}`,
            text: chunk.text,
            index: chunk.index,
            embedding: Array.from(embeddings[j])
          }
          this.chunks.push(record)
          this.keywordIndex.add(record.id, record.text, record)
        }
      } catch (err) {
        console.error('[SessionRag] Embedding batch failed:', err)
      }
    }
    
    this.indexed = true
    this.contentHash = hash
    this.indexingPromise = null
    
    const elapsed = Math.round(performance.now() - start)
    console.log('[SessionRag] Indexed', this.chunks.length, 'chunks in', elapsed, 'ms')
  }

  // search for relevant chunks: exact cosine (dense) and BM25 (lexical), fused by rank with RRF
  async search(query: string, limit = 5): Promise<Array<{ text: string; score: number }>> {
    if (!this.indexed || this.chunks.length === 0) {
      return []
    }
    
    try {
      const queryEmbedding = await embeddingProvider.embedQuery(query)
      const queryVec = Array.from(queryEmbedding)
      const candidates = limit * CANDIDATE_MULTIPLIER

      const dense: Array<RankedHit<SessionChunk>> = this.chunks
        .map(chunk => ({ chunk, similarity: this.cosineSimilarity(queryVec, chunk.embedding) }))
        .sort((a, b) => b.similarity - a.similarity)
        .slice(0, candidates)
        .map(({ chunk }) => ({ id: chunk.id, item: chunk }))
      const lexical = this.keywordIndex.search(query, candidates).map(hit => ({ id: hit.id, item: hit.item }))

      return reciprocalRankFusion(dense, lexical)
        .slice(0, limit)
        .map(hit => ({ text: hit.item.text, score: hit.score }))
        .filter(r => r.text.length > 0)
        
    } catch (err) {
      console.error('[SessionRag] Search failed:', err)
      return []
    }
  }

  // format search results as context for LLM
  async searchForContext(query: string, limit = 5): Promise<string> {
    const results = await this.search(query, limit)
    if (results.length === 0) return ''
    
    return results
      .map((r, i) => `[Section ${i + 1}]\n${r.text}`)
      .join('\n\n---\n\n')
  }

  private cosineSimilarity(a: number[], b: number[]): number {
    let dot = 0, normA = 0, normB = 0
    for (let i = 0; i < a.length; i++) {
      dot += a[i] * b[i]
      normA += a[i] * a[i]
      normB += b[i] * b[i]
    }
    return dot / (Math.sqrt(normA) * Math.sqrt(normB) + 1e-8)
  }

  isIndexed(): boolean {
    return this.indexed
  }

  getChunkCount(): number {
    return this.chunks.length
  }

  clear() {
    this.chunks = []
    this.keywordIndex.clear()
    this.indexed = false
    this.contentHash = ''
    this.indexingPromise = null
  }
}

export const sessionRag = new SessionRagService()
