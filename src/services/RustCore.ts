import initCore, {
  dictionary_request, gateway_complete, gateway_global_chat, gateway_page_summary,
  gateway_preferred, gateway_summarize, gateway_word_lookup, search_context, search_ensure,
  search_has, search_index, search_index_summary, search_is_indexing,
  stats_record, stats_summary, summary_cache_clear, summary_gate, summary_prefs_load,
  summary_prefs_save
} from '../../crates/metldr-core/pkg/metldr_core'
import type { ExtractedData, PageSummary, WordLookupResult } from '../types'
import { databaseService, DB_CONFIGS } from './DatabaseService'
import coreWasmUrl from '../../crates/metldr-core/pkg/metldr_core_bg.wasm?url'

const LEADING_SLASH_PATTERN = /^\//

let rustCoreReady: ReturnType<typeof initCore> | undefined

export function loadRustCore(): ReturnType<typeof initCore> {
  return rustCoreReady ??= initCore({
    module_or_path: chrome.runtime.getURL(coreWasmUrl.replace(LEADING_SLASH_PATTERN, ''))
  }).catch(error => {
    rustCoreReady = undefined
    throw error
  })
}

// The model gateway, for the page tools that run in this service worker. Each call throws a
// short sentence a person can read when no route can answer.
export interface ModelAnswer { text: string; route: string; model: string; ms: number }
export interface ModelTurn { role: 'user' | 'assistant'; content: string }
export interface CompleteRequest { system: string; messages: ModelTurn[]; schema?: object }
export interface SummaryRequest {
  content: string
  kind?: 'key-points' | 'tldr' | 'teaser' | 'headline'
  length?: 'short' | 'medium' | 'long'
  context?: string
}

export async function complete(request: CompleteRequest): Promise<ModelAnswer> {
  await loadRustCore()
  return gateway_complete(request)
}

// The cache the core keeps summaries and search chunks in: the extension opens it, with its
// schema, before the core reads or writes it (the core never creates it).
async function cacheReady(): Promise<void> {
  await loadRustCore()
  await databaseService.getDatabase(DB_CONFIGS.cache)
}

// The page tools' summary of an extracted page, cached by address unless `force`.
export async function pageSummary(page: ExtractedData, force: boolean): Promise<PageSummary> {
  await cacheReady()
  return gateway_page_summary(page, force)
}

export async function summarize(request: SummaryRequest): Promise<ModelAnswer> {
  await loadRustCore()
  return gateway_summarize(request)
}

export async function preferredRoute(): Promise<{ route: string; model?: string } | null> {
  await loadRustCore()
  return gateway_preferred()
}

export interface WordContext {
  fullSentence?: string
  contextBefore?: string
  contextAfter?: string
  pageLang?: string
}

// The word popup's lookup: the dictionary API, the page tools' model, then downloaded dictionaries.
export async function lookupWord(word: string, context: WordContext): Promise<WordLookupResult> {
  await loadRustCore()
  return gateway_word_lookup(word, context)
}

// The downloaded dictionaries: `{action: 'state' | 'select' | 'delete', code, on}` to their state.
export async function dictionary(request: object): Promise<unknown> {
  await loadRustCore()
  return dictionary_request(request)
}

// Counts a page tool's work in the local stats: a summary, a word lookup or reply suggestions.
interface SummaryStat {
  kind: 'summary'
  email: boolean
  wordsIn: number
  wordsOut: number
  tokensOut: number
  ms: number
  model?: string
  url?: string
}

export type StatEvent = SummaryStat | { kind: 'lookup' } | { kind: 'replies'; count: number }
  | { kind: 'run'; input?: number; output?: number; ms: number; model: string }

export async function recordStat(event: StatEvent): Promise<void> {
  await loadRustCore()
  await stats_record(event)
}

// The Stats page's numbers for the last `days` days, from the local records.
export async function statsSummary(days: number): Promise<unknown> {
  await loadRustCore()
  return stats_summary(days)
}

export async function clearSummaryCache(): Promise<void> {
  await loadRustCore()
  await summary_cache_clear()
}

export interface SummaryPrefs {
  mode: string
  allowlist: string[]
  denylist: string[]
  minAutoWords: number
  minPromptWords: number
}

export interface SummaryPrefsForm {
  mode: string
  allowlist: string
  denylist: string
  minAutoWords: number
}

export interface SummaryDecision {
  action: 'skip' | 'auto' | 'prompt' | 'wait'
  reason: string
}

// Whether an extracted page summarizes now, asks, waits or is skipped, from the saved settings.
export async function summaryGate(page: object, manual: boolean): Promise<SummaryDecision> {
  await loadRustCore()
  return summary_gate(page, manual)
}

export async function loadSummaryPrefs(): Promise<SummaryPrefs> {
  await loadRustCore()
  return summary_prefs_load()
}

export async function saveSummaryPrefs(form: SummaryPrefsForm): Promise<SummaryPrefs> {
  await loadRustCore()
  return summary_prefs_save(form)
}

// The search store in the Rust core: what was read, cut into chunks and indexed, and searches.
export interface SearchSource {
  sourceId: string
  sourceUrl: string
  sourceType: 'article' | 'email' | 'pdf'
  title?: string
}

export interface FoundSource {
  index: number
  title: string
  url: string
  type: 'email' | 'page' | 'pdf'
  score: number
  snippet: string
}

// Global search chat: the last question answered from everything read, with its sources.
export interface GlobalChatResult {
  ok: boolean
  content?: string
  sources?: FoundSource[]
  timing?: { total: number; model?: string }
  error?: string
}

type Turn = { role: string; content: string }

export async function globalChat(messages: Turn[]): Promise<GlobalChatResult> {
  await cacheReady()
  return gateway_global_chat(messages)
}

export const search = {
  async index(text: string, source: SearchSource, progress?: (percent: number) => void): Promise<void> {
    await cacheReady()
    await search_index(text, source, progress)
  },
  async indexSummary(text: string, source: SearchSource): Promise<void> {
    await cacheReady()
    await search_index_summary(text, source)
  },
  async has(sourceUrl: string): Promise<boolean> {
    await cacheReady()
    return search_has(sourceUrl)
  },
  async isIndexing(sourceId: string): Promise<boolean> {
    await cacheReady()
    return search_is_indexing(sourceId)
  },
  async ensure(text: string, source: SearchSource, progress?: (percent: number) => void): Promise<string> {
    await cacheReady()
    return search_ensure(text, source, progress)
  },
  async context(query: string, limit: number, sourceUrl?: string): Promise<string> {
    await cacheReady()
    return search_context(query, limit, sourceUrl ?? '')
  }
}
