import { computed, ref } from 'vue'

// The side panel's view of the model gateway in the service worker (the `metldr-gateway` port):
// which routes can answer, the one page tools use, and streamed replies.

export interface GatewayRoute {
  route: string
  kind: 'builtin' | 'local' | 'key'
  state: 'ready' | 'downloadable' | 'downloading' | 'absent' | 'needs-key' | 'error'
  destination: string
  endpoint?: string
  models?: string[]
  model?: string
}

export interface GatewayTurn { role: 'user' | 'assistant'; content: string }

interface Pending { resolve: (message: any) => void; reject: (error: Error) => void }
interface Stream extends Pending { onDelta: (text: string) => void }

export const ROUTE_NAMES: Record<string, string> = {
  'chrome-ai': 'gemini nano',
  ollama: 'ollama',
  lmstudio: 'lm studio',
  llamacpp: 'llama.cpp',
  openai: 'openai',
  anthropic: 'anthropic',
  google: 'google',
  openrouter: 'openrouter',
  chat: 'your endpoint'
}

export function useGateway() {
  const routes = ref<GatewayRoute[]>([])
  const preferred = ref<{ route: string; model?: string } | null>(null)
  const detecting = ref(true)

  let port: chrome.runtime.Port | null = null
  let next = 1
  const waiting = new Map<number, Pending>()
  const streams = new Map<number, Stream>()

  function open(): chrome.runtime.Port {
    if (port) return port
    const opened = chrome.runtime.connect({ name: 'metldr-gateway' })
    opened.onMessage.addListener((message: any) => {
      const stream = streams.get(message.id)
      if (stream && message.type) {
        if (message.type === 'delta') return stream.onDelta(message.text)
        streams.delete(message.id)
        if (message.type === 'done') stream.resolve(message)
        else stream.reject(new Error(message.message))
        return
      }
      const pending = waiting.get(message.id)
      if (!pending) return
      waiting.delete(message.id)
      if (message.ok) pending.resolve(message)
      else pending.reject(new Error(message.error))
    })
    opened.onDisconnect.addListener(() => {
      port = null
      const closed = new Error('The model gateway closed.')
      for (const pending of [...waiting.values(), ...streams.values()]) pending.reject(closed)
      waiting.clear()
      streams.clear()
    })
    return port = opened
  }

  function request(message: Record<string, unknown>): Promise<any> {
    const id = next++
    return new Promise((resolve, reject) => {
      waiting.set(id, { resolve, reject })
      open().postMessage({ ...message, id })
    })
  }

  async function detect(): Promise<void> {
    try {
      const found = await request({ op: 'detect' })
      routes.value = found.routes
      preferred.value = found.preferred
    } finally {
      detecting.value = false
    }
  }

  async function prefer(route: string, model: string): Promise<void> {
    await request({ op: 'prefer', route, model })
    await detect()
  }

  // A reply streamed on the chosen route; `stop` ends it early with the error "stopped".
  function stream(system: string, messages: GatewayTurn[], onDelta: (text: string) => void) {
    const id = next++
    const done = new Promise<{ text: string }>((resolve, reject) => {
      streams.set(id, { resolve, reject, onDelta })
      open().postMessage({ op: 'stream', id, system, messages })
    })
    return { done, stop: () => port?.postMessage({ op: 'stop', id }) }
  }

  // The route page tools answer on: the chosen one, else the first that is ready.
  const chosen = computed(() => routes.value.find(r => r.route === preferred.value?.route))
  const active = computed(() => chosen.value ?? routes.value.find(r => r.state === 'ready') ?? null)
  const ready = computed(() => active.value?.state === 'ready')
  const usable = computed(() => routes.value.filter(r => ['ready', 'downloadable'].includes(r.state)))

  return { routes, preferred, detecting, active, ready, usable, detect, prefer, stream }
}
