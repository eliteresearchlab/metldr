/// <reference types="chrome" />

// offscreen.ts - bridge between Service Worker and DOM contexts
// relays page-tool requests to the sandbox iframe

console.log('[Offscreen] Initializing bridge...')

const iframe = document.getElementById('sandbox-frame') as HTMLIFrameElement
const pendingResponses = new Map<string, (response: any) => void>()

// main message listener
chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (msg.target !== 'offscreen') return false

  console.log('[Offscreen] Received:', msg.type)

  // handle ping
  if (msg.type === 'PING') {
    sendResponse({ status: 'pong' })
    return false
  }

  // forward to sandbox iframe for GPU/ML operations
  if (!iframe || !iframe.contentWindow) {
    console.error('[Offscreen] Sandbox iframe not ready')
    sendResponse({ ok: false, error: 'Sandbox iframe not ready' })
    return false
  }

  const requestId = msg.data?.requestId
  if (requestId) {
    // set timeout for sandbox response (3 minute timeout for model loading)
    const timeout = setTimeout(() => {
      if (pendingResponses.has(requestId)) {
        console.error('[Offscreen] Sandbox request timed out:', msg.type)
        pendingResponses.delete(requestId)
        sendResponse({ ok: false, error: `Sandbox timeout for ${msg.type}` })
      }
    }, 180000)
    
    pendingResponses.set(requestId, (response) => {
      clearTimeout(timeout)
      sendResponse(response)
    })
  }
  
  iframe.contentWindow.postMessage(msg.data, '*')
  return true
})

// listen for sandbox responses
window.addEventListener('message', (event) => {
  if (event.source !== iframe?.contentWindow) return
  
  const { requestId, ...data } = event.data
  
  // handle ready signal
  if (event.data?._gpuBridgeReady) {
    console.log('[Offscreen] GPU Bridge signaled ready')
    return
  }
  
  if (requestId && pendingResponses.has(requestId)) {
    const resolve = pendingResponses.get(requestId)!
    pendingResponses.delete(requestId)
    
    console.log('[Offscreen] Sandbox response for', requestId, '- ok:', data.ok, 'error:', data.error)
    
    // ensure error field exists if not ok
    if (!data.ok && !data.error) {
      console.warn('[Offscreen] Response has ok=false but no error message')
      data.error = 'Unknown sandbox error'
    }
    
    resolve(data)
  }
})

console.log('[Offscreen] Ready')

