// Builds public/og-default.png (1200x630): the real wall render under the product name.
// Text uses a generic sans-serif stack because librsvg cannot read web fonts.
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readFileSync, mkdirSync } from 'node:fs';
import sharp from 'sharp';

const here = dirname(fileURLToPath(import.meta.url));
const render = resolve(here, '../../docs/brand/renders/wall-needs-input.svg');
const out = resolve(here, '../public/og-default.png');
const ACCENT = process.env.BRAND_ACCENT ?? '#E85002';

const W = 1200, H = 630;
const wall = await sharp(readFileSync(render), { density: 150 }).resize({ width: 1000 }).png().toBuffer();
const wallFaded = await sharp(wall)
  .composite([{
    input: Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="611"><defs><linearGradient id="f" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0f1014" stop-opacity="1"/><stop offset="0.35" stop-color="#0f1014" stop-opacity="0"/></linearGradient></defs><rect width="1000" height="611" fill="url(#f)"/></svg>`),
    blend: 'over',
  }])
  .png().toBuffer();

const text = Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}">
<rect x="80" y="70" width="64" height="8" rx="4" fill="${ACCENT}"/>
<text x="80" y="150" font-family="Helvetica, Arial, sans-serif" font-size="68" font-weight="700" fill="#f4f4f6" letter-spacing="-2">claude-garage</text>
<text x="80" y="204" font-family="Helvetica, Arial, sans-serif" font-size="30" fill="#b4b4bc">A terminal wall for Claude Code</text>
</svg>`);

mkdirSync(dirname(out), { recursive: true });
await sharp({ create: { width: W, height: H, channels: 4, background: '#0f1014' } })
  .composite([{ input: wallFaded, left: 100, top: 250 }, { input: text, left: 0, top: 0 }])
  .png({ compressionLevel: 9 })
  .toFile(out);
console.log('og-image: wrote public/og-default.png');
