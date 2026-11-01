// Builds the social cards (1200x630, light brand look, Geist text drawn as paths):
//   public/og-default.png    the homepage and any page without its own card
//   public/og/<slug>.png     one per blog post, compare page and docs page (names: see src/lib/og.ts)
// Output is gitignored. Needs no network and no browser.
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { mkdirSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { parse } from 'yaml';
import { ACCENT, MARGIN, MUTED, H, brandRow, card, fitTitle, fonts, measure, textPath } from './og-lib.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const pub = resolve(here, '../public');
const content = resolve(here, '../src/content');
mkdirSync(join(pub, 'og'), { recursive: true });

// Homepage card: the two-line headline with the one orange word.
{
  const sans = fonts.sans, ls = -0.04;
  const body = `${brandRow()}
${textPath('Every Claude session.', MARGIN, 190, 74, { font: sans, letterSpacing: ls })}
${textPath('One ', MARGIN, 272, 74, { font: sans, letterSpacing: ls })}
${textPath('calm', MARGIN + measure('One ', 74, { letterSpacing: ls }), 272, 74, { font: sans, letterSpacing: ls, fill: ACCENT })}
${textPath('wall.', MARGIN + measure('One calm ', 74, { letterSpacing: ls }), 272, 74, { font: sans, letterSpacing: ls })}`;
  await card(body, 330, join(pub, 'og-default.png'));
}

// Per-page cards.
const files = (dir) => readdirSync(dir, { withFileTypes: true, recursive: true }).filter((e) => e.isFile() && e.name.endsWith('.md')).map((e) => join(e.parentPath, e.name));
const front = (file) => parse(readFileSync(file, 'utf8').match(/^---\n([\s\S]*?)\n---/)?.[1] ?? '') ?? {};
// Same id rules as src/content.config.ts for docs.
const docId = (rel) => {
  const clean = rel.replace(/\.[^.]+$/, '').replace(/\/index$/, '').replace(/^index$/, '');
  return clean ? `docs/${clean.toLowerCase()}` : 'docs';
};

const jobs = [];
for (const f of files(join(content, 'blog'))) jobs.push({ kind: 'blog', slug: `blog-${f.slice(join(content, 'blog').length + 1, -3)}`, title: front(f).title });
for (const f of files(join(content, 'compare'))) jobs.push({ kind: 'compare', slug: `compare-${f.slice(join(content, 'compare').length + 1, -3)}`, title: front(f).title });
for (const f of files(join(content, 'docs'))) jobs.push({ kind: 'docs', slug: docId(f.slice(join(content, 'docs').length + 1)).replace(/\//g, '-'), title: front(f).title });

rmSync(join(pub, 'og'), { recursive: true, force: true });
mkdirSync(join(pub, 'og'), { recursive: true });
for (const { kind, slug, title } of jobs) {
  if (!title) { console.warn(`og-image: no title for ${slug}, skipped`); continue; }
  const { size, lines } = fitTitle(title);
  const lead = Math.round(size * 1.12);
  const first = 196 + Math.round(size * 0.8);
  const last = first + lead * (lines.length - 1);
  const t = lines.map((l, i) => textPath(l, MARGIN, first + i * lead, size, { letterSpacing: -0.03 })).join('\n');
  const body = `${brandRow()}
${textPath(`claude-garage · ${kind}`, MARGIN, 160, 17, { font: fonts.mono, fill: MUTED, letterSpacing: 0.02 })}
${t}`;
  const top = last + Math.round(size * 0.3) + 40;
  await card(body, H - top >= 110 ? top : null, join(pub, 'og', `${slug}.png`));
}
console.log(`og-image: wrote og-default.png and ${jobs.length} cards in public/og/`);
