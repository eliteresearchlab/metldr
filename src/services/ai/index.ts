export { AIProvider, NullProvider, type ProviderCapabilities, type ProviderName } from './AIProvider'
export { ChromeAIProvider, chromeAIProvider } from './ChromeAIProvider'
export { OllamaProvider, ollamaProvider } from './OllamaProvider'
export { localModels } from './LocalModelProvider'
export { aiGateway } from './AIGateway'
export { mapReduceService } from './MapReduceService'
export { AIPrompts, type EmailContext } from './AIPrompts'

export type {
  AICompleteRequest,
  AICompleteResponse,
  AISummarizeRequest,
  AISummarizeResponse,
  AIDetectLanguageRequest,
  AIDetectLanguageResponse,
  AIWriteRequest,
  AIWriteResponse,
  ChromeAIAvailability,
  ChromeAICapabilities
} from '../../types/chrome-ai'
