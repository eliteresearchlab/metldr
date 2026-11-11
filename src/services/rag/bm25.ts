// BM25 keyword index for hybrid retrieval (Robertson/Sparck Jones, k1 = 1.2, b = 0.75).
// The IDF uses the Lucene/tantivy form ln(1 + (N - df + 0.5) / (df + 0.5)), which stays positive for common terms.
// Term frequency and document length are both used, unlike the IDF-only index it replaces.

export const BM25_K1 = 1.2
export const BM25_B = 0.75

const TOKEN_PUNCTUATION_PATTERN = /[^\w\s]/g
const WHITESPACE_PATTERN = /\s+/

// tokenizer rules unchanged from the previous keyword index: lowercase, punctuation to spaces, tokens of 3+ chars
export function tokenize(text: string): string[] {
  return text.toLowerCase()
    .replace(TOKEN_PUNCTUATION_PATTERN, ' ')
    .split(WHITESPACE_PATTERN)
    .filter(t => t.length > 2)
}

export interface Bm25Hit<T> {
  id: string
  item: T
  score: number
}

export class Bm25Index<T> {
  // term -> (doc id -> term frequency)
  private postings = new Map<string, Map<string, number>>()
  // doc id -> document length in tokens, and the doc's unique terms (for removal)
  private docLengths = new Map<string, number>()
  private docTerms = new Map<string, string[]>()
  private items = new Map<string, T>()
  private totalLength = 0

  constructor(private readonly tokenizer: (text: string) => string[] = tokenize) {}

  add(id: string, text: string, item: T): void {
    if (this.docLengths.has(id)) this.remove(id)
    const tokens = this.tokenizer(text)
    const tf = new Map<string, number>()
    for (const token of tokens) tf.set(token, (tf.get(token) ?? 0) + 1)
    for (const [term, count] of tf) {
      let posting = this.postings.get(term)
      if (!posting) {
        posting = new Map()
        this.postings.set(term, posting)
      }
      posting.set(id, count)
    }
    this.docLengths.set(id, tokens.length)
    this.docTerms.set(id, [...tf.keys()])
    this.items.set(id, item)
    this.totalLength += tokens.length
  }

  remove(id: string): void {
    const length = this.docLengths.get(id)
    if (length === undefined) return
    for (const term of this.docTerms.get(id) ?? []) {
      const posting = this.postings.get(term)
      if (!posting) continue
      posting.delete(id)
      if (posting.size === 0) this.postings.delete(term)
    }
    this.docLengths.delete(id)
    this.docTerms.delete(id)
    this.items.delete(id)
    this.totalLength -= length
  }

  clear(): void {
    this.postings.clear()
    this.docLengths.clear()
    this.docTerms.clear()
    this.items.clear()
    this.totalLength = 0
  }

  get size(): number {
    return this.docLengths.size
  }

  values(): IterableIterator<T> {
    return this.items.values()
  }

  // unique query terms, BM25 score per document, best first; ties keep insertion order
  search(query: string, limit: number): Array<Bm25Hit<T>> {
    const terms = new Set(this.tokenizer(query))
    const n = this.docLengths.size
    if (terms.size === 0 || n === 0) return []
    const avgLength = this.totalLength / n

    const scores = new Map<string, number>()
    for (const term of terms) {
      const posting = this.postings.get(term)
      if (!posting) continue
      const df = posting.size
      const idf = Math.log(1 + (n - df + 0.5) / (df + 0.5))
      for (const [id, tf] of posting) {
        const length = this.docLengths.get(id) ?? 0
        const norm = BM25_K1 * (1 - BM25_B + BM25_B * (length / avgLength))
        const contribution = idf * (tf * (BM25_K1 + 1)) / (tf + norm)
        scores.set(id, (scores.get(id) ?? 0) + contribution)
      }
    }

    const hits: Array<Bm25Hit<T>> = []
    for (const [id, score] of scores) {
      if (!Number.isFinite(score)) throw new RangeError(`[bm25] non-finite score for ${id}`)
      const item = this.items.get(id)
      if (item !== undefined) hits.push({ id, item, score })
    }
    hits.sort((a, b) => b.score - a.score)
    return hits.slice(0, limit)
  }
}
