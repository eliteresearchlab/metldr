// Verifies the decoded SFNT fonts that the internal Rune host loads. Every check reads tracked
// files only: the manifest, the font files, the WOFF2 sources pinned in scripts/panel-assets.json
// and the OFL notices. Digests come from the font-conversion evidence and are copied into the
// manifest, so this check does not depend on the untracked evidence directory.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const fontsDir = join(repoRoot, 'crates/metldr-rune-host/assets/fonts');
const FONT_COUNT = 13;

const sha256 = (buffer) => createHash('sha256').update(buffer).digest('hex');
const failures = [];
const fail = (message) => failures.push(message);

const manifest = JSON.parse(readFileSync(join(fontsDir, 'manifest.json'), 'utf8'));
const pinned = new Map(JSON.parse(readFileSync(join(repoRoot, 'scripts/panel-assets.json'), 'utf8'))
  .map((asset) => [asset.name, asset]));

if (manifest.format !== 'sfnt') fail(`manifest format is ${manifest.format}, expected sfnt`);
if (manifest.fonts.length !== FONT_COUNT) fail(`manifest lists ${manifest.fonts.length} fonts, expected ${FONT_COUNT}`);

for (const font of manifest.fonts) {
  if (basename(font.name) !== font.name) fail(`font name is not a basename: ${font.name}`);
  const source = pinned.get(font.source);
  if (!source) {
    fail(`WOFF2 source ${font.source} is not pinned in scripts/panel-assets.json`);
  } else if (source.sha256 !== font.sourceSha256) {
    fail(`${font.name}: source digest differs from scripts/panel-assets.json`);
  }
  let bytes;
  try {
    bytes = readFileSync(join(fontsDir, font.name));
  } catch {
    fail(`${font.name} is missing`);
    continue;
  }
  if (bytes.length !== font.bytes) fail(`${font.name}: ${bytes.length} bytes, manifest says ${font.bytes}`);
  if (sha256(bytes) !== font.sha256) fail(`${font.name}: decoded digest differs from the manifest`);
  const signature = bytes.subarray(0, 4).toString('hex');
  const sfnt = ['00010000', '74727565', '4f54544f'].includes(signature);
  if (!sfnt) fail(`${font.name}: signature ${signature} is not SFNT`);
}

for (const notice of manifest.licenses) {
  const pin = pinned.get(notice);
  if (!pin) {
    fail(`license ${notice} is not pinned in scripts/panel-assets.json`);
    continue;
  }
  const copy = readFileSync(join(fontsDir, notice));
  if (sha256(copy) !== pin.sha256) fail(`${notice}: copy differs from the pinned notice`);
}

if (failures.length > 0) {
  for (const failure of failures) console.error(`check-rune-fonts: ${failure}`);
  process.exit(1);
}
console.log(`check-rune-fonts: ${manifest.fonts.length} SFNT fonts and ${manifest.licenses.length} notices verified`);
