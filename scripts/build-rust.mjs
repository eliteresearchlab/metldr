import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, mkdirSync, readFileSync, statSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { recordReceipt, sourceDigest } from './answer-contract.mjs';

const repoRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
// Ordered longest first so the parallel pool below finishes as early as it can.
const crates = [
  { name: 'metldr-panel', module: 'metldr_panel', pkg: 'crates/metldr-panel/pkg' },
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

function run(command, args, label) {
  const executable = isWindows ? `${command}.exe` : command;
  const result = spawnSync(executable, args, {
    cwd: repoRoot,
    env: cargoEnv,
    stdio: 'inherit',
    shell: false,
  });

  if (result.error) {
    console.error(`Unable to run ${label} (${executable}): ${result.error.message}`);
    process.exit(result.error.code === 'ENOENT' ? 127 : 1);
  }
  if (result.status !== 0) {
    if (result.status === null) {
      console.error(`${label} terminated by signal ${result.signal ?? 'unknown'}`);
      process.exit(1);
    }
    process.exit(result.status);
  }
}

const running = new Set();

function runAsync(command, args, label) {
  const executable = isWindows ? `${command}.exe` : command;
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(executable, args, {
      cwd: repoRoot,
      env: cargoEnv,
      stdio: ['ignore', 'pipe', 'pipe'],
      shell: false,
    });
    running.add(child);
    let output = '';
    child.stdout.on('data', chunk => { output += chunk; });
    child.stderr.on('data', chunk => { output += chunk; });
    child.on('error', error => {
      running.delete(child);
      rejectPromise(Object.assign(
        new Error(`Unable to run ${label} (${executable}): ${error.message}`),
        { exitCode: error.code === 'ENOENT' ? 127 : 1 },
      ));
    });
    child.on('close', (status, signal) => {
      running.delete(child);
      if (output) process.stdout.write(output.endsWith('\n') ? output : `${output}\n`);
      if (status === 0) {
        resolvePromise();
        return;
      }
      rejectPromise(Object.assign(
        new Error(status === null
          ? `${label} terminated by signal ${signal ?? 'unknown'}`
          : `${label} failed with exit code ${status}`),
        { exitCode: status ?? 1 },
      ));
    });
  });
}

// Runs at most `limit` tasks at once. Cargo takes an exclusive lock on a target
// directory, so each task below builds in its own directory under target/wasm.
async function runPool(tasks, limit) {
  const queue = [...tasks];
  const workers = Array.from({ length: Math.min(limit, queue.length) }, async () => {
    for (let task = queue.shift(); task; task = queue.shift()) await task();
  });
  await Promise.all(workers);
}

if (!isCheck) {
  const versionResult = spawnSync(isWindows ? 'wasm-bindgen.exe' : 'wasm-bindgen', ['--version'], {
    cwd: repoRoot,
    env: cargoEnv,
    encoding: 'utf8',
    shell: false,
  });
  if (versionResult.error) {
    console.error(`Unable to run wasm-bindgen${isWindows ? '.exe' : ''}: ${versionResult.error.message}`);
    process.exit(versionResult.error.code === 'ENOENT' ? 127 : 1);
  }
  if (versionResult.status !== 0) process.exit(versionResult.status ?? 1);
  if (!/^wasm-bindgen 0\.2\.128(?:\s|$)/.test(versionResult.stdout.trim())) {
    console.error(`Expected wasm-bindgen 0.2.128, found: ${versionResult.stdout.trim() || 'unknown version'}`);
    process.exit(1);
  }

}

const answerSourceHash = isCheck ? undefined : sourceDigest();

function cargoBuildArgs(crate) {
  const args = ['--locked', '--manifest-path', join('crates', crate.name, 'Cargo.toml'), '--lib', '--target', 'wasm32-unknown-unknown'];
  if (crate.features) args.push('--features', crate.features);
  return args;
}

function wasmTargetDir(crate) {
  return join(rootTargetDir, 'wasm', crate.name);
}

if (isCheck) {
  for (const crate of crates) {
    run('cargo', ['check', ...cargoBuildArgs(crate)], `Cargo (${crate.name})`);
  }
} else {
  const jobs = buildJobs();
  try {
    await runPool(crates.map(crate => async () => {
      const targetDir = wasmTargetDir(crate);
      await runAsync('cargo', ['build', '--release', ...cargoBuildArgs(crate), '--target-dir', targetDir], `Cargo (${crate.name})`);
      await runAsync('wasm-bindgen', [
        join(targetDir, 'wasm32-unknown-unknown', 'release', `${crate.artifact ?? crate.module}.wasm`),
        '--target', 'web', '--typescript', '--out-dir', crate.pkg, '--out-name', crate.module,
      ], `wasm-bindgen (${crate.name})`);
    }), jobs);
  } catch (error) {
    for (const child of running) child.kill('SIGTERM');
    console.error(error.message);
    process.exit(error.exitCode ?? 1);
  }
}

if (answerSourceHash) recordReceipt(answerSourceHash);

if (!isCheck) {
  const destination = join(repoRoot, 'public/chat/wasm');
  mkdirSync(destination, { recursive: true });
  for (const name of ['metldr_panel.js', 'metldr_panel_bg.wasm']) {
    copyFileSync(join(repoRoot, 'crates/metldr-panel/pkg', name), join(destination, name));
  }
}

run('cargo', [
  isCheck ? 'check' : 'build', '--locked', '--release',
  '--manifest-path', 'crates/metldr-welcome/Cargo.toml', '--bin', 'welcome-guide',
  '--target-dir', rootTargetDir,
], 'welcome generator');
