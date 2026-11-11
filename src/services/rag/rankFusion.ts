// Rank-based hybrid fusion for RAG retrieval.
//
// Reciprocal Rank Fusion (Cormack, Clarke and Buettcher, SIGIR 2009) with k = 60. Fusion reads only the
// position of each hit in each list, never the raw score. The dense leg (Voy) returns no similarity value,
// and BM25 scores are not comparable with cosine similarity, so rank-only fusion is the only correct option.
// Positions are integers, so a fused score cannot be NaN. The guard below keeps that true if this changes.

export const RRF_K = 60
// fused scores are reported on a 0-100 scale: a document ranked first by both lists scores 100
const RRF_TOP_SCORE = 100
const CANDIDATE_CAP = 50

export type MatchType = 'semantic' | 'keyword' | 'hybrid'

export interface RankedHit<T> {
  id: string
  item: T
}

export interface FusedHit<T> {
  id: string
  item: T
  score: number
  matchType: MatchType
}

export interface HybridRankOptions<T> {
  // +points (on the 0-100 scale) for items marked as summaries
  summaryBoost?: { points: number; isSummary: (item: T) => boolean }
  // -points per earlier hit from the same source (reduces repeated chunks of one page)
  sourceDiversity?: { penaltyPerRepeat: number; sourceOf: (item: T) => string }
}

export function assertFinite(value: number, label: string): number {
  if (!Number.isFinite(value)) throw new RangeError(`[rankFusion] non-finite ${label}: ${value}`)
  return value
}

// rank-only Reciprocal Rank Fusion of two ranked lists (best first)
export function reciprocalRankFusion<T>(
  dense: ReadonlyArray<RankedHit<T>>,
  lexical: ReadonlyArray<RankedHit<T>>,
  k = RRF_K
): Array<FusedHit<T>> {
  const scale = RRF_TOP_SCORE * (k + 1) / 2
  const acc = new Map<string, { item: T; raw: number; inDense: boolean; inLexical: boolean }>()

  const accumulate = (hits: ReadonlyArray<RankedHit<T>>, fromDense: boolean) => {
    const seen = new Set<string>()
    hits.forEach((hit, rank) => {
      if (seen.has(hit.id)) return
      seen.add(hit.id)
      const contribution = 1 / (k + rank + 1)
      const existing = acc.get(hit.id)
      if (existing) {
        existing.raw += contribution
        if (fromDense) existing.inDense = true
        else existing.inLexical = true
      } else {
        acc.set(hit.id, { item: hit.item, raw: contribution, inDense: fromDense, inLexical: !fromDense })
      }
    })
  }
  accumulate(dense, true)
  accumulate(lexical, false)

  const fused: Array<FusedHit<T>> = []
  for (const [id, entry] of acc) {
    const matchType: MatchType = entry.inDense && entry.inLexical ? 'hybrid' : entry.inDense ? 'semantic' : 'keyword'
    fused.push({ id, item: entry.item, score: assertFinite(entry.raw * scale, `rrf score for ${id}`), matchType })
  }
  return sortByScore(fused)
}

// the guard: every score must be finite before a sort (a NaN comparator silently keeps the old order)
function sortByScore<T>(hits: Array<FusedHit<T>>): Array<FusedHit<T>> {
  for (const hit of hits) assertFinite(hit.score, `score for ${hit.id}`)
  // stable sort: equal scores keep their earlier position
  return hits.sort((a, b) => b.score - a.score)
}

export function applySummaryBoost<T>(hits: Array<FusedHit<T>>, boost: { points: number; isSummary: (item: T) => boolean }): Array<FusedHit<T>> {
  return sortByScore(hits.map(hit => boost.isSummary(hit.item)
    ? { ...hit, score: Math.min(RRF_TOP_SCORE, hit.score + boost.points) }
    : hit))
}

export function applySourceDiversity<T>(hits: Array<FusedHit<T>>, diversity: { penaltyPerRepeat: number; sourceOf: (item: T) => string }): Array<FusedHit<T>> {
  const seen = new Map<string, number>()
  const penalised = hits.map(hit => {
    const source = diversity.sourceOf(hit.item).split('?')[0].split('#')[0]
    const repeats = seen.get(source) ?? 0
    seen.set(source, repeats + 1)
    return repeats > 0 ? { ...hit, score: Math.max(0, hit.score - repeats * diversity.penaltyPerRepeat) } : hit
  })
  return sortByScore(penalised)
}

// fusion followed by the enabled post-fusion adjustments, in the order the app has always used
export function rankHybrid<T>(
  dense: ReadonlyArray<RankedHit<T>>,
  lexical: ReadonlyArray<RankedHit<T>>,
  options: HybridRankOptions<T> = {}
): Array<FusedHit<T>> {
  let ranked = reciprocalRankFusion(dense, lexical)
  if (options.summaryBoost) ranked = applySummaryBoost(ranked, options.summaryBoost)
  if (options.sourceDiversity) ranked = applySourceDiversity(ranked, options.sourceDiversity)
  return ranked.slice(0, CANDIDATE_CAP)
}
