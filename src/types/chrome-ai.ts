export type ChromeAIAvailability = 'available' | 'downloadable' | 'downloading' | 'unavailable'

// summarizer api
export interface SummarizerOptions {
  type?: 'key-points' | 'tldr' | 'teaser' | 'headline'
  format?: 'markdown' | 'plain-text'
  length?: 'short' | 'medium' | 'long'
  sharedContext?: string
  expectedInputLanguages?: string[]
  outputLanguage?: string
  expectedContextLanguages?: string[]
}

export interface SummarizerSession {
  summarize(input: string, options?: { context?: string; signal?: AbortSignal }): Promise<string>
  summarizeStreaming(input: string, options?: { context?: string }): ReadableStream<string>
  destroy(): void
}

// language detector api
export interface LanguageDetection {
  detectedLanguage: string
  confidence: number
}

export interface LanguageDetectorSession {
  detect(text: string): Promise<LanguageDetection[]>
  destroy(): void
}

// writer api
export interface WriterOptions {
  tone?: 'formal' | 'neutral' | 'casual'
  format?: 'plain-text' | 'markdown'
  length?: 'short' | 'medium' | 'long'
  sharedContext?: string
}

export interface WriterSession {
  write(prompt: string, options?: { context?: string; signal?: AbortSignal }): Promise<string>
  writeStreaming(prompt: string, options?: { context?: string }): ReadableStream<string>
  destroy(): void
}

// prompt api (language model)
export interface PromptOptions {
  initialPrompts?: Array<{ role: 'system' | 'user' | 'assistant'; content: string }>
  systemPrompt?: string
  temperature?: number
  topK?: number
}

export interface LanguageModelSession {
  prompt(input: string, options?: { signal?: AbortSignal }): Promise<string>
  promptStreaming(input: string, options?: { signal?: AbortSignal }): ReadableStream<string>
  clone(): Promise<LanguageModelSession>
  destroy(): void
  readonly inputUsage: number
  readonly inputQuota: number
}

// capabilities report
export interface ChromeAICapabilities {
  summarizer: ChromeAIAvailability
  languageDetector: ChromeAIAvailability
  writer: ChromeAIAvailability
  promptAPI: ChromeAIAvailability
}

export interface DownloadProgressEvent {
  loaded: number
  total?: number
}

export interface DownloadMonitor {
  addEventListener(event: 'downloadprogress', callback: (e: DownloadProgressEvent) => void): void
}

export interface CreateOptions {
  monitor?: (m: DownloadMonitor) => void
}

export interface AICompleteRequest {
  systemPrompt: string
  userPrompt: string
  messages?: Array<{ role: 'user' | 'assistant'; content: string }>
  temperature?: number
  maxTokens?: number
  responseConstraint?: object
}

export interface AICompleteResponse {
  ok: boolean
  content?: string
  error?: string
  provider?: 'chrome-ai' | 'ollama'
  model?: string
  timing?: number
}

export interface AISummarizeRequest {
  content: string
  context?: string
  type?: 'key-points' | 'tldr' | 'teaser' | 'headline'
  length?: 'short' | 'medium' | 'long'
}

export interface AISummarizeResponse {
  ok: boolean
  summary?: string
  error?: string
  provider?: 'chrome-ai' | 'ollama'
  timing?: number
}

export interface AIDetectLanguageRequest {
  text: string
}

export interface AIDetectLanguageResponse {
  ok: boolean
  language?: string
  confidence?: number
  allResults?: LanguageDetection[]
  error?: string
  provider?: 'chrome-ai' | 'ollama' | 'regex'
}

export interface AIWriteRequest {
  prompt: string
  tone?: 'formal' | 'neutral' | 'casual'
  length?: 'short' | 'medium' | 'long'
  context?: string
}

export interface AIWriteResponse {
  ok: boolean
  content?: string
  error?: string
  provider?: 'chrome-ai' | 'ollama'
  timing?: number
}

