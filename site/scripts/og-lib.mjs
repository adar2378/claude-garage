// Shared pieces for the social cards: Geist text turned into SVG paths (librsvg cannot load web fonts,
// so the glyph outlines are drawn straight from the font files), the drawn logo, and the dark TUI window.
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import opentype from 'opentype.js';
import sharp from 'sharp';

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const fontFile = (pkg, file) => readFileSync(resolve(dirname(require.resolve(`${pkg}/package.json`)), 'files', file));
const load = (buf) => opentype.parse(buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength));

export const fonts = {
  sans: load(fontFile('@fontsource/geist', 'geist-latin-600-normal.woff')),
  mono: load(fontFile('@fontsource/geist-mono', 'geist-mono-latin-500-normal.woff')),
};

export const W = 1200, H = 630, MARGIN = 110, WIN_W = 980;
export const INK = '#14151A', MUTED = '#5B5E69', PAGE = '#FAFAF8';
export const ACCENT = process.env.BRAND_ACCENT ?? '#E85002';
const render = resolve(here, '../../docs/brand/renders/wall-needs-input.svg');

// Lays glyphs out by hand (advance + kerning + tracking): opentype.js gives NaN for letterSpacing around spaces and for some kerning pairs.
// letterSpacing is in em.
function layout(text, size, font, letterSpacing) {
  const glyphs = font.stringToGlyphs(text);
  const scale = size / font.unitsPerEm;
  const out = [];
  let x = 0;
  glyphs.forEach((g, i) => {
    out.push({ g, x });
    x += (g.advanceWidth ?? 0) * scale + letterSpacing * size;
    if (glyphs[i + 1]) {
      const k = font.getKerningValue(g, glyphs[i + 1]);
      if (Number.isFinite(k)) x += k * scale; // some pairs come back NaN
    }
  });
  return { out, width: x - (glyphs.length ? letterSpacing * size : 0) };
}

// A text run as one <path>. Path data is written by hand: opentype's toPathData emitted NaN for some glyph starts.
export function textPath(text, x, y, size, { font = fonts.sans, fill = INK, letterSpacing = 0 } = {}) {
  const { out } = layout(text, size, font, letterSpacing);
  const n = (v) => +v.toFixed(2);
  const d = out.map(({ g, x: gx }) => g.getPath(x + gx, y, size).commands
    .map((c) => c.type + [c.x1, c.y1, c.x2, c.y2, c.x, c.y].filter((v) => v !== undefined).map(n).join(' ')).join('')).join('');
  return `<path d="${d}" fill="${fill}"/>`;
}
export const measure = (text, size, { font = fonts.sans, letterSpacing = 0 } = {}) => layout(text, size, font, letterSpacing).width;

// Greedy wrap. Returns lines, or null when it needs more than maxLines.
function wrap(text, size, maxW, maxLines, opts) {
  const lines = [];
  let cur = '';
  for (const word of text.split(/\s+/)) {
    const next = cur ? `${cur} ${word}` : word;
    if (cur && measure(next, size, opts) > maxW) { lines.push(cur); cur = word; } else cur = next;
  }
  if (cur) lines.push(cur);
  return lines.length <= maxLines && lines.every((l) => measure(l, size, opts) <= maxW) ? lines : null;
}

/** Largest size from `max` down to `min` at which the title fits maxLines lines; ellipsis if even `min` does not. */
export function fitTitle(title, { max = 68, min = 42, maxW = WIN_W, maxLines = 3, letterSpacing = -0.03 } = {}) {
  const opts = { letterSpacing };
  for (let size = max; size >= min; size -= 2) {
    const lines = wrap(title, size, maxW, maxLines, opts);
    if (lines) return { size, lines };
  }
  const words = title.split(/\s+/);
  while (words.length > 1) {
    words.pop();
    const lines = wrap(`${words.join(' ')}…`, min, maxW, maxLines, opts);
    if (lines) return { size: min, lines };
  }
  return { size: min, lines: [title] };
}

// The drawn logo (same geometry as src/assets/logo.svg) at (x, y), 76x30.
export function logoMark(x, y) {
  return `<g transform="translate(${x} ${y})"><rect width="76" height="30" rx="10" fill="${INK}"/>
<g fill="none" stroke="${ACCENT}" stroke-width="2.2" stroke-linecap="round"><path d="M15 8.5Q9.5 15 15 21.5"/><circle cx="27" cy="15" r="4.4"/><circle cx="49" cy="15" r="4.4"/><path d="M61 8.5Q66.5 15 61 21.5"/></g>
<circle cx="38" cy="18.6" r="1.7" fill="${ACCENT}"/></g>`;
}
export const brandRow = () => `${logoMark(MARGIN, 64)}${textPath('claude-garage', MARGIN + 92, 86, 24, { letterSpacing: -0.02 })}`;

// Dark terminal window with the real wall render, top edge at `top`, cut off by the card's bottom edge.
const wallPng = async () => sharp(readFileSync(render), { density: 150 }).resize({ width: WIN_W - 2 }).png().toBuffer();
let wallCache;
export async function windowLayers(top) {
  const BAR = 30;
  wallCache ??= await wallPng();
  const wallH = (await sharp(wallCache).metadata()).height;
  const winH = BAR + wallH;
  const bar = Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${WIN_W}" height="${BAR}">
<rect width="${WIN_W}" height="${BAR}" fill="#1d1f27"/>
<circle cx="18" cy="15" r="5" fill="#3a3d4a"/><circle cx="36" cy="15" r="5" fill="#3a3d4a"/><circle cx="54" cy="15" r="5" fill="#3a3d4a"/>
${textPath('claude-garage', 76, 19.5, 12, { font: fonts.mono, fill: '#9094a3' })}
</svg>`);
  const mask = Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${WIN_W}" height="${winH}"><rect width="${WIN_W}" height="${winH + 40}" rx="18" fill="#fff"/></svg>`);
  const full = await sharp({ create: { width: WIN_W, height: winH, channels: 4, background: '#16171d' } })
    .composite([{ input: bar, left: 0, top: 0 }, { input: wallCache, left: 1, top: BAR }, { input: mask, blend: 'dest-in' }])
    .png().toBuffer();
  const visible = Math.min(winH, H - top);
  const win = await sharp(full).extract({ left: 0, top: 0, width: WIN_W, height: visible }).png().toBuffer();
  const shadow = await sharp(Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${WIN_W + 160}" height="${H - top + 160}"><rect x="80" y="80" width="${WIN_W}" height="${H - top + 100}" rx="18" fill="#14162a" fill-opacity=".35"/></svg>`))
    .blur(28).png().toBuffer();
  return [{ input: shadow, left: MARGIN - 80, top: top - 60 }, { input: win, left: MARGIN, top }];
}

export async function card(svgBody, windowTop, out) {
  const layers = windowTop == null ? [] : await windowLayers(windowTop);
  const text = Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}">${svgBody}</svg>`);
  await sharp({ create: { width: W, height: H, channels: 4, background: PAGE } })
    .composite([...layers, { input: text, left: 0, top: 0 }])
    .png({ compressionLevel: 9 })
    .toFile(out);
}
