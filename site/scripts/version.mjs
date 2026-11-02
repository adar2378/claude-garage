// Writes src/generated/version.json with the version people get from `npx claude-garage`.
// Source: the npm registry's `latest` tag, read at build time. The page stays static.
// Fallback: the Cargo workspace version, when the registry cannot be reached.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(here, '../src/generated/version.json');
const NOINDEX = process.env.SITE_NOINDEX === '1';

function cargoVersion() {
  const toml = readFileSync(resolve(here, '../../Cargo.toml'), 'utf8');
  const section = toml.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/)?.[1] ?? '';
  return section.match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1] ?? null;
}

async function npmVersion() {
  try {
    const res = await fetch('https://registry.npmjs.org/claude-garage/latest', { signal: AbortSignal.timeout(8000) });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return (await res.json()).version ?? null;
  } catch (e) {
    console.warn(`[version] npm registry not reachable (${e.message}).`);
    return null;
  }
}

const cargo = cargoVersion();
const npm = await npmVersion();
const version = npm ?? cargo;
if (!version) throw new Error('version: neither npm nor ../Cargo.toml gave a version');

if (!npm) console.warn(`[version] using ${cargo} from ../Cargo.toml instead.`);
if (npm && cargo && npm !== cargo && !NOINDEX) {
  console.warn(`\n[version] WARNING: npm latest is ${npm} but ../Cargo.toml says ${cargo}. The site shows ${npm}.\n`);
}

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, JSON.stringify({ version, source: npm ? 'npm' : 'cargo' }, null, 2) + '\n');
console.log(`[version] ${version} (from ${npm ? 'npm' : 'Cargo.toml'})`);
