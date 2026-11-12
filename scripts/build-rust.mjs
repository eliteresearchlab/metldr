import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, cpSync, mkdirSync, readFileSync, rmSync, statSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { recordReceipt, sourceDigest } from './answer-contract.mjs';

const repoRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
// Ordered longest first so the parallel pool below finishes as early as it can.
const crates = [
  { name: 'metldr-panel', module: 'metldr_panel', pkg: 'crates/metldr-panel/pkg', features: 'rune-ui' },
  { name: 'metldr-welcome', module: 'welcome_runtime', pkg: 'public/welcome/pkg' },
  { name: 'metldr-core', module: 'metldr_core', pkg: 'crates/metldr-core/pkg' },
  { name: 'metldr-answer-codec', artifact: 'metldr_answer_codec', module: 'answer_codec', pkg: 'crates/metldr-answer-codec/pkg', features: 'guest' },
];
const isCheck = process.argv.includes('--check');
const isWindows = process.platform === 'win32';
const panelAssetFields = ['name', 'url', 'sha256', 'bytes'];
const rootTargetDir = process.env.CARGO_TARGET_DIR || 'target';
const defaultBuildJobs = 2;
// sccache is opt-in: contributors without it keep the plain compiler. Pass it on
// explicitly so every cargo child below sees the same wrapper.
const cargoEnv = { ...process.env };

function buildJobs() {
  const raw = process.env.METLDR_BUILD_JOBS;
  if (raw === undefined || raw === '') return defaultBuildJobs;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isInteger(parsed) || parsed < 1 || String(parsed) !== raw.trim()) {
    console.error(`METLDR_BUILD_JOBS must be a positive integer, found: ${raw}`);
    process.exit(1);
  }
  return parsed;
}

for (const asset of JSON.parse(readFileSync(join(repoRoot, 'scripts/panel-assets.json'), 'utf8'))) {
  if (Object.keys(asset).length !== panelAssetFields.length
      || !panelAssetFields.every(field => Object.hasOwn(asset, field))
      || typeof asset.url !== 'string') {
    throw new Error('Panel asset entries must contain name, url, sha256 and bytes.');
  }
  const { name, bytes, sha256 } = asset;
  const file = join(repoRoot, 'public/rust-panel/assets', name);
  if (basename(name) !== name || bytes > 256 * 1024 || statSync(file).size !== bytes
      || createHash('sha256').update(readFileSync(file)).digest('hex') !== sha256) {
    throw new Error(`Panel asset verification failed: ${name}`);
  }
}

const running = new Set();
const failedGroups = new Set();
let failure;
let cleanup;

function cancel(error) {
  if (failure) return;
  failure = error;
  const children = [...running, ...failedGroups];
  const signal = (child, name) => {
    try {
      if (isWindows) child.kill(name);
      else if (child.pid) process.kill(-child.pid, name);
    } catch (error) {
      if (error.code !== 'ESRCH') throw error;
    }
  };
  for (const child of children) signal(child, 'SIGTERM');
  cleanup = new Promise(resolve => setTimeout(resolve, 1000)).then(() => {
    if (!isWindows) for (const child of children) signal(child, 'SIGKILL');
  });
}

function runAsync(command, args, label, capture = false) {
  const executable = isWindows ? `${command}.exe` : command;
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(executable, args, {
      cwd: repoRoot,
      env: cargoEnv,
      stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
      shell: false,
      detached: !isWindows,
    });
    running.add(child);
    let spawnError;
    let output = '';
    if (capture) {
      child.stdout.on('data', chunk => { output = (output + chunk).slice(0, 1024); });
      child.stderr.resume();
    }
    child.on('error', error => { spawnError = error; });
    child.on('close', (status, signal) => {
      running.delete(child);
      if (!isWindows && status !== 0 && child.pid) failedGroups.add(child);
      if (spawnError) {
        rejectPromise(Object.assign(
          new Error(`Unable to run ${label} (${executable}): ${spawnError.message}`),
          { exitCode: spawnError.code === 'ENOENT' ? 127 : 1 },
        ));
      } else if (status === 0) {
        resolvePromise(output);
      } else {
        rejectPromise(Object.assign(
          new Error(status === null
            ? `${label} terminated by signal ${signal ?? 'unknown'}`
            : `${label} failed with exit code ${status}`),
          { exitCode: status ?? 1 },
        ));
      }
    });
  });
}

async function runPool(tasks, limit) {
  const queue = [...tasks];
  const workers = Array.from({ length: Math.min(limit, queue.length) }, async () => {
    while (!failure && queue.length) {
      try { await queue.shift()(); } catch (error) { cancel(error); }
    }
  });
  await Promise.allSettled(workers);
  if (cleanup) await cleanup;
  if (failure) throw failure;
}

function cargoBuildArgs(crate) {
  const args = ['--locked', '--manifest-path', join('crates', crate.name, 'Cargo.toml'), '--lib', '--target', 'wasm32-unknown-unknown'];
  if (crate.features) args.push('--features', crate.features);
  return args;
}

function wasmTargetDir(crate) {
  return join(rootTargetDir, 'wasm', crate.name);
}

const onInterrupt = () => cancel(Object.assign(new Error('Interrupted by SIGINT'), { exitCode: 130 }));
const onTerminate = () => cancel(Object.assign(new Error('Interrupted by SIGTERM'), { exitCode: 143 }));
process.on('SIGINT', onInterrupt);
process.on('SIGTERM', onTerminate);

try {
  if (!isCheck) {
    const version = await runAsync('wasm-bindgen', ['--version'], 'wasm-bindgen', true);
    if (failure) throw failure;
    if (!/^wasm-bindgen 0\.2\.128(?:\s|$)/.test(version.trim())) {
      throw Object.assign(new Error(`Expected wasm-bindgen 0.2.128, found: ${version.trim() || 'unknown version'}`), { exitCode: 1 });
    }
  }
  const answerSourceHash = isCheck ? undefined : sourceDigest();
  if (isCheck) {
    for (const crate of crates) {
      await runAsync('cargo', ['check', ...cargoBuildArgs(crate)], `Cargo (${crate.name})`);
      if (failure) throw failure;
    }
  } else {
    await runPool(crates.map(crate => async () => {
      const targetDir = wasmTargetDir(crate);
      await runAsync('cargo', ['build', '--release', ...cargoBuildArgs(crate), '--target-dir', targetDir], `Cargo (${crate.name})`);
      if (failure) return;
      await runAsync('wasm-bindgen', [
        join(targetDir, 'wasm32-unknown-unknown', 'release', `${crate.artifact ?? crate.module}.wasm`),
        '--target', 'web', '--typescript', '--out-dir', crate.pkg, '--out-name', crate.module,
      ], `wasm-bindgen (${crate.name})`);
    }), buildJobs());
  }
  if (failure) throw failure;
  if (answerSourceHash) recordReceipt(answerSourceHash);
  if (!isCheck) {
    const destination = join(repoRoot, 'public/chat/wasm');
    mkdirSync(destination, { recursive: true });
    for (const name of ['metldr_panel.js', 'metldr_panel_bg.wasm']) {
      copyFileSync(join(repoRoot, 'crates/metldr-panel/pkg', name), join(destination, name));
    }
    rmSync(join(destination, 'snippets'), { recursive: true, force: true });
    cpSync(join(repoRoot, 'crates/metldr-panel/pkg/snippets'), join(destination, 'snippets'), { recursive: true });
  }
  await runAsync('cargo', [
    isCheck ? 'check' : 'build', '--locked', '--release',
    '--manifest-path', 'crates/metldr-welcome/Cargo.toml', '--bin', 'welcome-guide',
    '--target-dir', rootTargetDir,
  ], 'welcome generator');
  if (failure) throw failure;
} catch (error) {
  cancel(error);
  if (cleanup) await cleanup;
  console.error(failure.message);
  process.exitCode = failure.exitCode ?? 1;
} finally {
  process.off('SIGINT', onInterrupt);
  process.off('SIGTERM', onTerminate);
}
