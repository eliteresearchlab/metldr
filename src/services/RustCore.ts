import initCore, {
  dictionary_request, gateway_complete, gateway_page_summary, gateway_preferred, gateway_summarize, gateway_word_lookup,
  stats_record, stats_summary, summary_cache_clear, summary_gate, summary_prefs_load, summary_prefs_save
} from '../../crates/metldr-core/pkg/metldr_core'
import type { ExtractedData, PageSummary, WordLookupResult } from '../types'
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

// The page tools' summary of an extracted page, cached by address unless `force`.
export async function pageSummary(page: ExtractedData, force: boolean): Promise<PageSummary> {
  await loadRustCore()
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
