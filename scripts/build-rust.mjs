import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { recordReceipt, sourceDigest } from './answer-contract.mjs';

const repoRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
// Ordered longest first so the parallel pool below finishes as early as it can.
const crates = [
  // The service worker loads one module, the companion relay with the core (whose search compares
  // int8 vectors with wasm SIMD, 16 lanes at once); the chat page loads the Rune variant.
  { name: 'metldr-panel', module: 'metldr_panel', pkg: 'crates/metldr-panel/pkg', simd: true, features: 'worker' },
  { name: 'metldr-panel', variant: 'rune', module: 'metldr_panel', pkg: 'target/wasm/metldr-panel-rune/pkg', features: 'rune-ui' },
  { name: 'metldr-welcome', module: 'welcome_runtime', pkg: 'public/welcome/pkg' },
  { name: 'metldr-pdf', module: 'metldr_pdf', pkg: 'crates/metldr-pdf/pkg', pdfium: true },
  { name: 'metldr-answer-codec', artifact: 'metldr_answer_codec', module: 'answer_codec', pkg: 'crates/metldr-answer-codec/pkg', features: 'guest' },
];
const isCheck = process.argv.includes('--check');
const isWindows = process.platform === 'win32';
const rootTargetDir = process.env.CARGO_TARGET_DIR || 'target';
const defaultBuildJobs = 2;
// sccache is opt-in: contributors without it keep the plain compiler. When it is installed,
// every build uses it, whatever started the build: cargo fingerprints the wrapper, so a build
// with it and one without would each throw the other's work away.
const cargoEnv = { ...process.env };
if (!cargoEnv.RUSTC_WRAPPER && !isWindows) {
  const found = spawnSync('sccache', ['--version'], { stdio: 'ignore' });
  if (found.status === 0) cargoEnv.RUSTC_WRAPPER = 'sccache';
}
// `METLDR_PROFILE=fast` builds with the fast profile for edit and check loops; anything else
// builds `release`, which packaged and measured builds use.
const profile = process.env.METLDR_PROFILE === 'fast' ? 'fast' : 'release';

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

function runAsync(command, args, label, capture = false, env = cargoEnv) {
  const executable = isWindows ? `${command}.exe` : command;
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(executable, args, {
      cwd: repoRoot,
      env,
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

// PDFium for the browser, linked into metldr-pdf. The archive is fetched once and used only
// when its SHA-256 matches, so the package never ships a binary nobody pinned.
const PDFIUM = {
  url: 'https://github.com/run-llama/pdfium-binaries/releases/download/chromium%2F8058/pdfium-wasi-wasm.tgz',
  sha256: 'e8a5b7851f3bbbd325dca9e9f5029fc56be1df0be40dd710051521cff1028f35',
  dir: join(homedir(), '.cache', 'metldr-pdfium', 'chromium_8058'),
};

const simdEnv = { ...cargoEnv, CARGO_ENCODED_RUSTFLAGS: '-Ctarget-feature=+simd128' };
const buildError = message => Object.assign(new Error(message), { exitCode: 1 });

async function pdfiumEnv() {
  const lib = join(PDFIUM.dir, 'lib');
  if (!existsSync(join(lib, 'libpdfium.a'))) {
    const response = await fetch(PDFIUM.url);
    if (!response.ok) throw buildError(`PDFium download failed: ${response.status}`);
    const archive = Buffer.from(await response.arrayBuffer());
    const found = createHash('sha256').update(archive).digest('hex');
    if (found !== PDFIUM.sha256) {
      throw buildError(`PDFium archive hash ${found} is not the pinned ${PDFIUM.sha256}`);
    }
    const staging = `${PDFIUM.dir}.tmp`;
    rmSync(staging, { recursive: true, force: true });
    mkdirSync(staging, { recursive: true });
    writeFileSync(join(staging, 'pdfium.tgz'), archive);
    await runAsync('tar', ['-xzf', join(staging, 'pdfium.tgz'), '-C', staging], 'PDFium extract');
    rmSync(join(staging, 'pdfium.tgz'));
    rmSync(PDFIUM.dir, { recursive: true, force: true });
    renameSync(staging, PDFIUM.dir);
  }
  return { ...cargoEnv, PDFIUM_LIB_PATH: lib, PDFIUM_INCLUDE_PATH: join(PDFIUM.dir, 'include') };
}

function cargoBuildArgs(crate) {
  const args = ['--locked', '--manifest-path', join('crates', crate.name, 'Cargo.toml'), '--lib', '--target', 'wasm32-unknown-unknown'];
  if (crate.features) args.push('--features', crate.features);
  return args;
}

function wasmTargetDir(crate) {
  return join(rootTargetDir, 'wasm', crate.variant ? `${crate.name}-${crate.variant}` : crate.name);
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
      const args = ['check', ...cargoBuildArgs(crate)];
      const env = crate.pdfium ? await pdfiumEnv() : crate.simd ? simdEnv : cargoEnv;
      await runAsync('cargo', args, `Cargo (${crate.name})`, false, env);
      if (failure) throw failure;
    }
  } else {
    await runPool(crates.map(crate => async () => {
      const targetDir = wasmTargetDir(crate);
      const args = ['build', '--profile', profile, ...cargoBuildArgs(crate), '--target-dir', targetDir];
      const env = crate.pdfium ? await pdfiumEnv() : crate.simd ? simdEnv : cargoEnv;
      await runAsync('cargo', args, `Cargo (${crate.name})`, false, env);
      if (failure) return;
      const wasm = join(targetDir, 'wasm32-unknown-unknown', profile, `${crate.artifact ?? crate.module}.wasm`);
      // An unchanged wasm already has its bindings: skip wasm-bindgen when the wasm's hash
      // matches the one its last bindings were made from.
      mkdirSync(crate.pkg, { recursive: true });
      const stamp = join(crate.pkg, '.bindgen-source');
      const hash = createHash('sha256').update(readFileSync(wasm)).digest('hex');
      if (readFileSync(stamp, { encoding: 'utf8', flag: 'a+' }) === hash) return;
      await runAsync('wasm-bindgen', [
        wasm, '--target', 'web', '--typescript', '--out-dir', crate.pkg, '--out-name', crate.module,
      ], `wasm-bindgen (${crate.name})`);
      if (failure) return;
      writeFileSync(stamp, hash);
    }), buildJobs());
  }
  if (failure) throw failure;
  if (answerSourceHash) recordReceipt(answerSourceHash);
  await runAsync('cargo', [
    isCheck ? 'check' : 'build', '--locked', '--profile', profile,
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
