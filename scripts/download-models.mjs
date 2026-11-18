#!/usr/bin/env node
// The search model: potion-retrieval-32M (MIT) at a pinned revision, checked by SHA-256 and
// packed by crates/metldr-embed into public/models/potion-retrieval-32M.metldr (int8, ~33 MB).
// Run once with `npm run download:models`; the build copies public/models into the package.

import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const output = join(root, 'public', 'models', 'potion-retrieval-32M.metldr')
const cache = join(homedir(), '.cache', 'metldr-models', 'potion-retrieval-32M')
const revision = '6fc8051fab2a1e0ee76689cf08c853792ac285e7'
const files = {
  'model.safetensors': '07609e5bd33aad37900b3fd62f4ec96f6daec88ca4d46b9d8b928bfababf6ea0',
  'tokenizer.json': '7d75cbc54318138807c401b0f0c9721117c628b39de8e8e0edb6cb17e0ee7d18',
}

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')

async function fetchPinned(name, expected) {
  const path = join(cache, name)
  if (existsSync(path) && sha256(readFileSync(path)) === expected) return path
  const url = `https://huggingface.co/minishlab/potion-retrieval-32M/resolve/${revision}/${name}`
  const response = await fetch(url)
  if (!response.ok) throw new Error(`${url}: ${response.status}`)
  const bytes = Buffer.from(await response.arrayBuffer())
  const found = sha256(bytes)
  if (found !== expected) throw new Error(`${name} hash ${found} is not the pinned ${expected}`)
  mkdirSync(cache, { recursive: true })
  writeFileSync(path, bytes)
  return path
}

const weights = await fetchPinned('model.safetensors', files['model.safetensors'])
const tokenizer = await fetchPinned('tokenizer.json', files['tokenizer.json'])
mkdirSync(dirname(output), { recursive: true })
const packed = spawnSync('cargo', [
  'run', '--quiet', '--release', '--manifest-path', join(root, 'crates/metldr-embed/Cargo.toml'),
  '--bin', 'pack-model', '--', weights, tokenizer, output,
], { stdio: 'inherit' })
if (packed.status !== 0) {
  rmSync(output, { force: true })
  process.exit(packed.status ?? 1)
}
console.log(`packed ${output}`)
