// Crawls dist/**/*.html and checks that every internal href/src points at a file in dist.
// Also flags links that point at a redirect source (dist/_redirects) and fragments with no matching id.
// Usage: node scripts/check-links.mjs   (after a build). Exits 1 on any broken link.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const dist = resolve(dirname(fileURLToPath(import.meta.url)), '../dist');
const walk = (d) => readdirSync(d).flatMap((f) => (statSync(join(d, f)).isDirectory() ? walk(join(d, f)) : [join(d, f)]));
const pages = walk(dist).filter((f) => f.endsWith('.html') && !f.includes('/pagefind/'));

const redirects = existsSync(join(dist, '_redirects'))
  ? new Set(readFileSync(join(dist, '_redirects'), 'utf8').split('\n').filter((l) => l.startsWith('/')).map((l) => l.split(/\s+/)[0].replace(/\/$/, '')))
  : new Set();

const resolveFile = (path) => {
  const p = join(dist, decodeURIComponent(path));
  if (existsSync(p) && statSync(p).isFile()) return p;
  for (const c of [join(p, 'index.html'), `${p}.html`]) if (existsSync(c)) return c;
  return null;
};
const idCache = new Map();
const idsOf = (file) => {
  if (!idCache.has(file)) idCache.set(file, new Set([...readFileSync(file, 'utf8').matchAll(/\s(?:id|name)="([^"]+)"/g)].map((m) => m[1])));
  return idCache.get(file);
};

const problems = [];
let checked = 0;
for (const page of pages) {
  const rel = '/' + page.slice(dist.length + 1).replace(/index\.html$/, '');
  const html = readFileSync(page, 'utf8');
  for (const m of html.matchAll(/\s(?:href|src)="([^"]*)"/g)) {
    let href = m[1].replace(/&amp;/g, '&');
    if (!href || /^(https?:|mailto:|tel:|data:|javascript:|\/\/)/.test(href)) {
      if (!/^https?:\/\/claude-garage\.xyz/.test(href)) continue;
      href = href.replace(/^https?:\/\/claude-garage\.xyz/, '') || '/';
    }
    checked++;
    const [pathAndQuery, frag] = href.split('#');
    const path = pathAndQuery.split('?')[0];
    const target = path ? (path.startsWith('/') ? path : join(rel, path)) : rel;
    if (redirects.has(target.replace(/\/$/, ''))) {
      problems.push(`${rel}  ->  ${href}  (points at a redirect source)`);
      continue;
    }
    const file = resolveFile(target);
    if (!file) { problems.push(`${rel}  ->  ${href}  (404)`); continue; }
    if (frag && file.endsWith('.html') && !idsOf(file).has(decodeURIComponent(frag))) problems.push(`${rel}  ->  ${href}  (no #${frag} on target)`);
  }
}
console.log(`check-links: ${pages.length} pages, ${checked} internal references`);
if (problems.length) {
  const uniq = [...new Set(problems)];
  console.log(uniq.join('\n'));
  console.log(`check-links: ${uniq.length} problem(s)`);
  process.exit(1);
}
console.log('check-links: no broken internal links');
