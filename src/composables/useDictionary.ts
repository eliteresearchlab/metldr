import { ref } from 'vue'

// The dictionaries live in the service worker's Rust core; this mirrors what it reports.
export interface DictionaryLanguage { code: string; name: string }
interface DictionaryState {
  languages: DictionaryLanguage[]
  selected: string[]
  downloaded: string[]
  progress: Record<string, number>
}

const languages = ref<DictionaryLanguage[]>([])
const downloadedLanguages = ref<string[]>([])
const selectedLanguages = ref<string[]>([])
const downloadProgress = ref<Record<string, number>>({})
const WATCHED = ['dictDownloadProgress', 'downloadingLanguages', 'selectedLanguages']

async function ask(request: { action: string; code?: string; on?: boolean }): Promise<void> {
  const message = { type: 'DICTIONARY', ...request }
  const state: DictionaryState | undefined = await chrome.runtime.sendMessage(message)
  if (!state?.languages) return
  languages.value = state.languages
  selectedLanguages.value = state.selected
  downloadedLanguages.value = state.downloaded
  downloadProgress.value = state.progress
}

export function useDictionary() {
  async function loadDictionarySettings(): Promise<void> {
    chrome.storage.onChanged.addListener((changes, area) => {
      if (area === 'local' && WATCHED.some(key => key in changes)) ask({ action: 'state' })
    })
    await ask({ action: 'state' })
  }

  async function toggleLanguage(code: string): Promise<void> {
    await ask({ action: 'select', code, on: !selectedLanguages.value.includes(code) })
  }

  async function deleteLanguageData(code: string): Promise<void> {
    if (confirm(`delete ${code} dictionary data?`)) await ask({ action: 'delete', code })
  }

  return {
    languages,
    downloadedLanguages,
    selectedLanguages,
    downloadProgress,
    loadDictionarySettings,
    toggleLanguage,
    deleteLanguageData
  }
}
