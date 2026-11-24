import { watchChat } from './scripts/build-chat.mjs'
import { defineConfig, normalizePath, type Plugin } from 'vite'
import { copyFileSync, existsSync, mkdirSync, readFileSync, watch, writeFileSync } from 'node:fs'
import { basename, dirname, resolve } from 'node:path'
import { execFileSync } from 'node:child_process'

// Each content script is its own build: an IIFE inlines its imports, so it cannot share chunks.
const contentEntry = ({ content: 'main', gmail: 'gmail' } as Record<string, string>)[process.env.BUILD_TARGET ?? '']
const isContentBuild = contentEntry !== undefined
const privateFiles = [
  '**/.tmp/**', '**/prototypes/**', '**/docs/**',
  '**/{AGENTS,CLAUDE,PRODUCT,DESIGN}.md', '**/crates/**/target/**', '**/target/**',
]
// build-rust.mjs builds the generator with the same profile as the wasm (METLDR_PROFILE).
const welcomeGenerator = resolve(process.env.CARGO_TARGET_DIR || 'target',
  process.env.METLDR_PROFILE === 'fast' ? 'fast' : 'release',
  process.platform === 'win32' ? 'welcome-guide.exe' : 'welcome-guide')

const rustWelcome = (): Plugin => ({
  name: 'rust-welcome',
  configureServer(server) {
    const directory = normalizePath(resolve('public/welcome')) + '/'
    let pending: ReturnType<typeof setTimeout> | undefined
    const refresh = () => {
      clearTimeout(pending)
      pending = setTimeout(() => {
        server.ws.send({ type: 'full-reload', path: '/welcome.html' })
        server.config.logger.info('Welcome preview updated.', { timestamp: true })
      }, 100)
    }
    const reload = (file: string) => {
      if (normalizePath(file).startsWith(directory)) refresh()
    }
    const generator = watch(dirname(welcomeGenerator), (_, file) => {
      if (file === basename(welcomeGenerator)) refresh()
    })
    generator.on('error', error => server.config.logger.error(
      `Welcome generator watch stopped: ${error.message}. Restart the development server.`,
    ))
    server.watcher.on('change', reload).on('add', reload)
    server.httpServer?.once('close', () => {
      clearTimeout(pending)
      generator.close()
      server.watcher.off('change', reload).off('add', reload)
    })
  },
  transformIndexHtml: {
    order: 'pre',
    handler(html, context) {
      if (resolve(context.filename) !== resolve('welcome.html')) return html
      return execFileSync(welcomeGenerator, [
        resolve('welcome.html'), '-',
      ], { encoding: 'utf8', timeout: 5_000, maxBuffer: 1_048_576, windowsHide: true })
    },
  },
})


const chatDevelopment = (): Plugin => ({
  name: 'metldr-chat-development',
  async configureServer(server) {
    const dispose = await watchChat()
    server.httpServer?.once('close', () => { void dispose() })
  },
})

const copyInboxSDKPlugin = () => ({
  name: 'copy-inboxsdk-pageworld',
  writeBundle() {
    if (!isContentBuild) {
      const src = resolve('node_modules/@inboxsdk/core/pageWorld.js')
      const dest = resolve('dist/pageWorld.js')
      if (existsSync(src)) {
        copyFileSync(src, dest)
        console.log('copied pageWorld.js to dist/')
      }
    }
  }
})

// Browse's page scripts ship as files: the companion sends each one's hash, the extension the
// script. helpers.js is filled exactly as browse-page's scripts::helpers() fills it.
const copyBrowseScripts = (): Plugin => ({
  name: 'copy-browse-scripts',
  writeBundle() {
    const js = (name: string) => readFileSync(resolve('vendor/browse/crates/browse-page/js', name), 'utf8')
    mkdirSync(resolve('dist/browse'), { recursive: true })
    copyFileSync(resolve('vendor/browse/crates/browse-page/js/injected-script.js'), resolve('dist/browse/injected-script.js'))
    writeFileSync(resolve('dist/browse/helpers.js'), js('helpers.js').replaceAll('__NAMING__', js('naming.js')).replaceAll('__CAPTURE__', js('capture.js')))
  }
})

const originalConsoleWarn = console.warn
console.warn = (...args: unknown[]) => {
  const message = args.join(' ')
  if (message.includes('@property') || message.includes('Unknown at rule')) {
    return
  }
  originalConsoleWarn(...args)
}

export default defineConfig({
  server: {
    watch: { ignored: privateFiles },
    fs: { deny: ['.env', '.env.*', '*.{crt,pem}', '**/.git/**', ...privateFiles] },
  },
  plugins: isContentBuild ? [] : [rustWelcome(), chatDevelopment(), copyInboxSDKPlugin(), copyBrowseScripts()],
  resolve: {
    alias: {
      '@': resolve(__dirname, './src')
    }
  },
  css: {
    lightningcss: {
      errorRecovery: true,
      targets: {
        chrome: 85,
      }
    }
  },
  build: {
    modulePreload: false,
    outDir: 'dist',
    emptyOutDir: !isContentBuild,
    rollupOptions: isContentBuild ? {
      input: `src/content/${contentEntry}.ts`,
      output: {
        entryFileNames: `${process.env.BUILD_TARGET}.js`,
        format: 'iife',
        name: 'MeTLDRContent',
        inlineDynamicImports: true,
      },
    } : {
      input: {
        chat: 'chat.html',
        welcome: 'welcome.html',
        background: 'src/background/index.ts',
      },
      output: {
        entryFileNames: '[name].js',
        chunkFileNames: 'chunks/[name].js',
        assetFileNames: 'assets/[name][extname]',
      }
    }
  }
})
