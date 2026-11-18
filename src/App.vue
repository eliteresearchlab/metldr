<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from 'vue'
import { useThemeStore } from '@/stores/theme'
import { analyticsService } from '@/services/AnalyticsService'

import { 
  useGateway,
  usePageSummary, 
  useChat, 
  useSettings, 
  useDictionary, 
  useTabSession
} from '@/composables'

import HistoryManager from '@/components/HistoryManager.vue'
import SummaryCard from '@/components/SummaryCard.vue'
import ChatPanel from '@/components/ChatPanel.vue'
import SettingsPanel from '@/components/SettingsPanel.vue'
import HoverRevealNav from '@/components/HoverRevealNav.vue'
import PdfDropZone from '@/components/PdfDropZone.vue'
import GlobalSearch from '@/components/GlobalSearch.vue'
// progress now shown inline in ChatPanel
import { pdfService } from '@/services/pdf/PdfService'


import { ScrollArea } from '@/components/ui'
import { Loader2, X, FileText, BarChart3, Settings, Search } from 'lucide-vue-next'

const themeStore = useThemeStore()

const { routes, active: activeRoute, ready: aiReady, detecting: aiChecking, detect, prefer, stream } = useGateway()

const {
  pageSummary,
  pageMetadata,
  summaryLoading,
  summaryError,
  currentTabId,
  currentTabUrl,
  summaryPrompt,
  summaryCollapsed,
  isEmailClient,
  isViewingEmailThread,
  chatDisabled,
  chatDisabledReason,
  fetchCurrentPageSummary,
  resetSummaryState,
  setCurrentTabUrl,
  refreshCurrentTabUrl,
  openLocalPdf
} = usePageSummary()

const {
  chatMessages,
  chatInput,
  chatLoading,
  chatIndexing,
  sendChatMessage: baseSendChatMessage,
  clearChat: baseClearChat,
  resetChatState,
  switchToUrl,
  syncIndexingStatus,
  setUrlMessages
} = useChat()

const {
  summaryMode,
  allowlistInput,
  denylistInput,
  minAutoWords,
  wordPopupEnabled,
  loadSummaryPrefs,
  loadWordPopupSetting,
  toggleWordPopup,
  loadFontSize,
  setFontSize,
  fontSize,
  setupSettingsWatcher,
  setupStorageListener
} = useSettings()

const {
  languages,
  downloadedLanguages,
  selectedLanguages,
  downloadProgress,
  loadDictionarySettings,
  toggleLanguage,
  deleteLanguageData
} = useDictionary()

const { saveTabSession, setupTabListener } = useTabSession()

const activeTab = ref<string>('summary')
const historyRef = ref<InstanceType<typeof HistoryManager> | null>(null)
const chatPanelRef = ref<InstanceType<typeof ChatPanel> | null>(null)
const navOpen = ref(false)

async function doSaveTabSession(): Promise<void> {
  await saveTabSession(currentTabUrl, chatMessages, pageSummary, summaryCollapsed)
}

async function doFetchSummary(force = false, trigger = 'auto'): Promise<void> {
  await fetchCurrentPageSummary(force, trigger, doSaveTabSession)
}

function triggerManualSummary(): void {
  doFetchSummary(true, 'manual')
}
function acceptSummaryPrompt(): void {
  if (!summaryPrompt.value) return
  summaryPrompt.value = null
  doFetchSummary(true, 'manual')
}

function declineSummaryPrompt(): void {
  summaryError.value = 'summary dismissed'
  summaryPrompt.value = null
}

async function sendChatMessage(): Promise<void> {
  await baseSendChatMessage(pageSummary, stream, ref(null), ref(null), doSaveTabSession)
  chatPanelRef.value?.focusInput()
}

function clearChat(): void {
  baseClearChat(undefined, doSaveTabSession)
  chatPanelRef.value?.focusInput()
}

function openWelcomePage(): void {
  chrome.tabs.create({ url: chrome.runtime.getURL('welcome.html') })
}
async function clearCache(): Promise<void> {
  if (!confirm('clear all cached summaries?')) return
  
  try {
    await chrome.runtime.sendMessage({ type: 'CLEAR_SUMMARY_CACHE' })
    alert('cache cleared!')
    if (historyRef.value) {
      historyRef.value.refresh()
    }
  } catch (error) {
    console.error('failed to clear cache:', error)
    alert('failed to clear cache')
  }
}

function switchTab(tab: string): void {
  if (activeTab.value === tab) return
  activeTab.value = tab
  analyticsService.trackTabSwitch(tab as 'summary' | 'stats' | 'settings').catch(() => {})
}

function handleKeydown(e: KeyboardEvent): void {
  if (e.ctrlKey && e.shiftKey && e.key?.toLowerCase?.() === 'l') {
    e.preventDefault()
    triggerManualSummary()
  }
}

// pdf drag-drop handler
const pdfDropLoading = ref(false)
const pdfDropError = ref<string | null>(null)

async function handlePdfDropped(file: File): Promise<void> {
  pdfDropLoading.value = true
  pdfDropError.value = null
  
  try {
    console.log('[App] Processing dropped PDF:', file.name)
    const arrayBuffer = await file.arrayBuffer()
    const result = await pdfService.summarizeFromArrayBuffer(arrayBuffer, file.name)
    
    // update page summary state with the PDF result
    pageSummary.value = {
      bullets: result.summary.split('\n').filter(l => l.trim()),
      timing: { total: 0, model: 'pdf' }
    } as any // type assertion for PDF-generated summary
    
    console.log('[App] PDF processed successfully')
  } catch (err) {
    console.error('[App] PDF processing failed:', err)
    pdfDropError.value = (err as Error).message
  } finally {
    pdfDropLoading.value = false
  }
}

function handlePdfDropError(message: string): void {
  console.warn('[App] PDF drop error:', message)
  pdfDropError.value = message
}

let cleanupTabListener: (() => void) | null = null
let statusCheckInterval: ReturnType<typeof setInterval> | null = null
let sidePanelMessageListener: ((msg: any) => void) | null = null

// listen for messages to close/toggle the side panel
function setupSidePanelMessageListener() {
  sidePanelMessageListener = (msg: any) => {
    if (msg.type === 'CLOSE_SIDE_PANEL' || msg.type === 'TOGGLE_SIDE_PANEL') {
      window.close()
    }
  }
  chrome.runtime.onMessage.addListener(sidePanelMessageListener)
}

onMounted(async () => {
  await themeStore.loadSavedTheme()
  
  await Promise.all([
    loadWordPopupSetting(),
    loadSummaryPrefs(),
    loadFontSize()
  ])
  await loadDictionarySettings()
  
  await detect()
  
  setupSettingsWatcher()
  setupStorageListener()
  
  cleanupTabListener = setupTabListener(
    currentTabId,
    currentTabUrl,
    chatMessages,
    pageSummary,
    summaryCollapsed,
    aiReady,
    summaryMode,
    doFetchSummary,
    switchToUrl,
    setUrlMessages,
    resetSummaryState
  )
  
  window.addEventListener('keydown', handleKeydown)
  
  
  // picks up a server started later or a model downloaded in setup
  statusCheckInterval = setInterval(() => { detect().catch(() => {}) }, 5000)
  
  const context = isEmailClient.value ? 'gmail' : 'article'
  analyticsService.startSession(context, 'summary', currentTabUrl.value || undefined).catch(() => {})
  
  syncIndexingStatus()
  setupSidePanelMessageListener()
})

watch(currentTabUrl, (newUrl) => {
  if (newUrl) {
    const context = newUrl.includes('mail.google.com') ? 'gmail' : 'article'
    analyticsService.updateSessionContext(context, newUrl).catch(() => {})
  }
})

onUnmounted(() => {
  if (statusCheckInterval) clearInterval(statusCheckInterval)
  if (cleanupTabListener) cleanupTabListener()
  if (sidePanelMessageListener) chrome.runtime.onMessage.removeListener(sidePanelMessageListener)
  window.removeEventListener('keydown', handleKeydown)
  analyticsService.endSession().catch(() => {})
})
</script>

<template>
  <div class="h-screen flex flex-col bg-background text-foreground overflow-hidden">
    <div id="dropdown-portal" class="fixed inset-0 pointer-events-none z-50"></div>



    <HoverRevealNav 
      v-if="aiReady"
      :active-tab="activeTab"
      :tabs="[
        { key: 'summary', icon: FileText, label: 'Summary' },
        { key: 'search', icon: Search, label: 'Search' },
        { key: 'stats', icon: BarChart3, label: 'Stats' },
        { key: 'settings', icon: Settings, label: 'Settings' }
      ]"
      @switch="switchTab"
      @open="navOpen = true"
      @close="navOpen = false"
    />

    <PdfDropZone
      @pdf-dropped="handlePdfDropped"
      @error="handlePdfDropError"
    >
      <main 
        class="flex-1 overflow-hidden"
        :class="navOpen ? 'content-down' : 'content-up'"
      >
      <Transition name="fade" mode="out-in">
        <KeepAlive include="GlobalSearch">
          <div v-if="aiChecking" key="loading" class="flex flex-col items-center justify-center h-full p-6">
            <Loader2 class="w-8 h-8 mb-3 animate-spin text-primary" :stroke-width="2" />
            <p class="text-[12px] text-foreground/70">connecting to ai...</p>
          </div>
          
          <GlobalSearch 
            v-else-if="aiReady && activeTab === 'search'" 
            key="search"
            class="h-full" 
          />

          <div 
            v-else-if="aiReady && activeTab === 'summary'" 
            key="summary"
            :class="['flex flex-col h-full min-h-0 relative', summaryPrompt ? 'pb-16' : '']"
          >
            <SummaryCard 
              v-if="!isViewingEmailThread"
              :page-summary="pageSummary"
              :summary-loading="summaryLoading"
              :summary-error="summaryError"
              :summary-prompt="summaryPrompt"
              :summary-collapsed="summaryCollapsed"
              :is-email-client="isEmailClient"
              :is-viewing-email-thread="isViewingEmailThread"
              @update:collapsed="(v) => summaryCollapsed = v"
              @refresh="() => doFetchSummary(true, 'manual')"
              @manual-summary="triggerManualSummary"
              @accept-prompt="acceptSummaryPrompt"
              @decline-prompt="declineSummaryPrompt"
              @open-local-pdf="() => openLocalPdf(doSaveTabSession)"
            />

            <ChatPanel 
              ref="chatPanelRef"
              v-model:chat-input="chatInput"
              :chat-messages="chatMessages"
              :chat-loading="chatLoading"
              :chat-indexing="chatIndexing"
              :summary-loading="summaryLoading"
              :chat-disabled="chatDisabled"
              :disabled-reason="chatDisabledReason"
              :is-viewing-email-thread="isViewingEmailThread"
              :current-url="currentTabUrl"
              @send="sendChatMessage"
              @clear="clearChat"
            />
          </div>

          <ScrollArea v-else-if="aiReady && activeTab === 'stats'" key="stats" class="h-full">
            <div class="p-3">
              <HistoryManager ref="historyRef" :limit="10" />
            </div>
          </ScrollArea>

          <SettingsPanel 
            :routes="routes"
            :active-route="activeRoute"
            :detecting="aiChecking"
            @prefer="prefer"
            v-else-if="activeTab === 'settings'"
            key="settings"
            :summary-mode="summaryMode"
            :min-auto-words="minAutoWords"
            :allowlist-input="allowlistInput"
            :denylist-input="denylistInput"
            :word-popup-enabled="wordPopupEnabled"
            :languages="languages"
            :downloaded-languages="downloadedLanguages"
            :selected-languages="selectedLanguages"
            :download-progress="downloadProgress"
            @update:summary-mode="(v) => summaryMode = v"
            @update:min-auto-words="(v) => minAutoWords = v"
            @update:allowlist-input="(v) => allowlistInput = v"
            @update:denylist-input="(v) => denylistInput = v"
            @toggle-word-popup="toggleWordPopup"
            @toggle-language="toggleLanguage"
            @delete-language="deleteLanguageData"
            @clear-cache="clearCache"
            @open-welcome="openWelcomePage"
            :font-size="fontSize"
            @update:font-size="setFontSize"
          />

          <div v-else key="setup" class="flex flex-col items-center justify-center h-full p-6">
            <X :size="32" class="mb-3 text-foreground/30" />
            <p class="text-[13px] font-medium text-foreground/80 mb-2">no ai yet</p>
            <p class="text-[11px] text-foreground/50 text-center mb-4 max-w-[220px]">metldr can use chrome's built-in ai, a model server on this computer, or your own key.</p>
            <div class="flex gap-2">
              <button @click="openWelcomePage" class="px-3 py-1.5 rounded-md text-[11px] font-medium bg-primary/20 text-primary hover:bg-primary/30 transition-colors active:scale-[0.97]">
                set up ai
              </button>
              <button @click="switchTab('settings')" class="px-3 py-1.5 rounded-md text-[11px] bg-muted text-foreground hover:bg-muted/80 transition-colors flex items-center gap-1.5 active:scale-[0.97]">
                <Settings :size="12" />
                settings
              </button>
            </div>
          </div>
        </KeepAlive>
      </Transition>
    </main>
    </PdfDropZone>
  </div>
</template>

<style scoped>
.content-down {
  padding-top: 44px;
  transition: padding-top 200ms cubic-bezier(0.4, 0, 0.2, 1);
}

.content-up {
  padding-top: 16px;
  transition: padding-top 180ms cubic-bezier(0.4, 0, 0.2, 1);
}

.fade-enter-active,
.fade-leave-active {
  transition: all 150ms ease;
}

.fade-enter-from {
  opacity: 0;
  transform: translateY(4px);
}

.fade-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
