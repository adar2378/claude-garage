// Runs after `astro build`.
// 1. Fills __INLINE_SCRIPT_HASHES__ in dist/_headers with the sha256 of every inline script on /try/,
//    computed from the built bytes, so the /try/* CSP stays strict without 'unsafe-inline'.
// 2. SITE_NOINDEX=1: adds an X-Robots-Tag header to every response (covers non-HTML files too).
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const dist = resolve(dirname(fileURLToPath(import.meta.url)), '../dist');
const headersFile = resolve(dist, '_headers');
let headers = readFileSync(headersFile, 'utf8');

const html = readFileSync(resolve(dist, 'try/index.html'), 'utf8');
const hashes = [];
for (const m of html.matchAll(/<script(?![^>]*\bsrc=)([^>]*)>([\s\S]*?)<\/script>/g)) {
  if (/type\s*=\s*["']?application\/(ld\+)?json/i.test(m[1])) continue; // data blocks are not executed
  hashes.push(`'sha256-${createHash('sha256').update(m[2], 'utf8').digest('base64')}'`);
}
if (!headers.includes('__INLINE_SCRIPT_HASHES__')) throw new Error('postbuild: placeholder missing in dist/_headers');
headers = headers.replaceAll('__INLINE_SCRIPT_HASHES__', hashes.join(' '));
if (headers.includes("script-src 'self'  ")) headers = headers.replaceAll("script-src 'self'  ", "script-src 'self' ");
console.log(`postbuild: ${hashes.length} inline script hash(es) for /try/`);

if (process.env.SITE_NOINDEX === '1') {
  headers = headers.replace(/^\/\*\n/m, '/*\n  X-Robots-Tag: noindex\n');
  console.log('postbuild: X-Robots-Tag noindex added');
}
writeFileSync(headersFile, headers);
