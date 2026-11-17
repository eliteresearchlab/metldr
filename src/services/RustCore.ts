import initCore, {
  gateway_complete, gateway_preferred, gateway_summarize, gateway_word_lookup
} from '../../crates/metldr-core/pkg/metldr_core'
import type { WordLookupResult } from '../types'
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
