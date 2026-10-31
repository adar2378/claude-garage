// Builds public/og-default.png (1200x630) in the light brand look: warm off-white page, the headline,
// and the real needs-input wall render in a dark terminal window.
// Text uses a generic sans-serif stack because librsvg cannot read web fonts.
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readFileSync, mkdirSync } from 'node:fs';
import sharp from 'sharp';

const here = dirname(fileURLToPath(import.meta.url));
const render = resolve(here, '../../docs/brand/renders/wall-needs-input.svg');
const out = resolve(here, '../public/og-default.png');
const ACCENT = process.env.BRAND_ACCENT ?? '#E85002';
const INK = '#14151A';
const PAGE = '#FAFAF8';

const W = 1200, H = 630;
const WIN_W = 980, BAR = 30, WIN_X = (W - WIN_W) / 2, WIN_Y = 330;
const wallW = WIN_W - 2;
const wall = await sharp(readFileSync(render), { density: 150 }).resize({ width: wallW }).png().toBuffer();
const wallH = (await sharp(wall).metadata()).height;
const winH = BAR + wallH;

// Dark window: title bar with dots, the render below, rounded top corners.
const bar = Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${WIN_W}" height="${BAR}">
<rect width="${WIN_W}" height="${BAR}" fill="#1d1f27"/>
<circle cx="18" cy="15" r="5" fill="#3a3d4a"/><circle cx="36" cy="15" r="5" fill="#3a3d4a"/><circle cx="54" cy="15" r="5" fill="#3a3d4a"/>
<text x="76" y="19.5" font-family="Menlo, Courier, monospace" font-size="12" fill="#9094a3">claude-garage</text>
</svg>`);
const mask = Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${WIN_W}" height="${winH}"><rect width="${WIN_W}" height="${winH + 40}" rx="18" fill="#fff"/></svg>`);
const winFull = await sharp({ create: { width: WIN_W, height: winH, channels: 4, background: '#16171d' } })
  .composite([{ input: bar, left: 0, top: 0 }, { input: wall, left: 1, top: BAR }, { input: mask, blend: 'dest-in' }])
  .png().toBuffer();
const win = await sharp(winFull).extract({ left: 0, top: 0, width: WIN_W, height: Math.min(winH, H - WIN_Y) }).png().toBuffer();

const shadow = await sharp(Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${WIN_W + 160}" height="${H - WIN_Y + 160}"><rect x="80" y="80" width="${WIN_W}" height="${H - WIN_Y + 100}" rx="18" fill="#14162a" fill-opacity=".35"/></svg>`))
  .blur(28).png().toBuffer();

const text = Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}">
<rect x="${WIN_X}" y="64" width="76" height="30" rx="10" fill="${INK}"/>
<g fill="none" stroke="${ACCENT}" stroke-width="2.2" stroke-linecap="round" transform="translate(${WIN_X} 64)"><path d="M15 8.5Q9.5 15 15 21.5"/><circle cx="27" cy="15" r="4.4"/><circle cx="49" cy="15" r="4.4"/><path d="M61 8.5Q66.5 15 61 21.5"/></g>
<circle cx="${WIN_X + 38}" cy="${64 + 18.6}" r="1.7" fill="${ACCENT}"/>
<text x="${WIN_X + 92}" y="86" font-family="Helvetica, Arial, sans-serif" font-size="24" font-weight="700" fill="${INK}" letter-spacing="-0.5">claude-garage</text>
<text x="${WIN_X}" y="190" font-family="Helvetica, Arial, sans-serif" font-size="74" font-weight="700" fill="${INK}" letter-spacing="-3">Every Claude session.</text>
<text x="${WIN_X}" y="272" font-family="Helvetica, Arial, sans-serif" font-size="74" font-weight="700" fill="${INK}" letter-spacing="-3">One <tspan fill="${ACCENT}">calm</tspan> wall.</text>
</svg>`);

mkdirSync(dirname(out), { recursive: true });
await sharp({ create: { width: W, height: H, channels: 4, background: PAGE } })
  .composite([
    { input: shadow, left: WIN_X - 80, top: WIN_Y - 60 },
    { input: win, left: WIN_X, top: WIN_Y },
    { input: text, left: 0, top: 0 },
  ])
  .png({ compressionLevel: 9 })
  .toFile(out);
console.log('og-image: wrote public/og-default.png');
