// Below-the-fold wall screenshots as cropped WebP, rendered from the synced brand SVGs (public/renders).
// The hero window stays SVG. Each crop is sized at 2x its display width and written to public/crops/<name>.<hash>.webp,
// so the file name changes when the pixels change and can be cached forever. The manifest goes to src/generated/crops.json.
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const here = dirname(fileURLToPath(import.meta.url));
const renders = resolve(here, '../public/renders');
const out = resolve(here, '../public/crops');
const CANVAS = [1440, 880];

// name, source render, crop box [x, y, w, h] in render units, display width in CSS px
const CROPS = [
  ['s0', 'wall-calm', [250, 0, 600, 340], 560],
  ['s1', 'wall-calm', [0, 0, 1440, 880], 560],
  ['s2', 'wall-needs-input', [0, 0, 1440, 880], 560],
  ['s3', 'wall-engaged', [255, 520, 600, 330], 560],
  ['s4', 'wall-plugins', [0, 0, 600, 290], 560],
  ['p0', 'wall-plugins', [0, 98, 250, 160], 200],
  ['p1', 'wall-plugins', [268, 16, 570, 110], 320],
  ['p2', 'wall-plugins', [1040, 857, 400, 23], 320],
  ['p3', 'wall-plugins', [0, 857, 300, 23], 280],
  ['peek', 'wall-needs-input', [0, 0, 1440, 560], 880],
];

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
mkdirSync(resolve(here, '../src/generated'), { recursive: true });
const manifest = {};
// Story and closing crops ship three widths (srcset) so phones fetch about half the bytes.
const RESPONSIVE = /^(s\d|peek)$/;
async function render(src, [x, y, w, h], outW) {
  const k = outW / w;
  const svg = readFileSync(resolve(renders, `${src}.svg`));
  const full = await sharp(svg, { density: 72 * k }).resize(Math.round(CANVAS[0] * k)).png().toBuffer();
  return sharp(full)
    .extract({ left: Math.round(x * k), top: Math.round(y * k), width: Math.round(w * k), height: Math.round(h * k) })
    .webp({ quality: 80, effort: 6 })
    .toBuffer();
}
for (const [name, src, box, display] of CROPS) {
  const widths = RESPONSIVE.test(name) ? [0.5, 0.75, 1].map((f) => Math.round(display * 2 * f)) : [display * 2];
  const variants = [];
  for (const outW of widths) {
    const buf = await render(src, box, outW);
    const meta = await sharp(buf).metadata();
    const hash = createHash('sha256').update(buf).digest('hex').slice(0, 8);
    const file = `${name}-${meta.width}.${hash}.webp`;
    writeFileSync(resolve(out, file), buf);
    variants.push({ src: `/crops/${file}`, w: meta.width, h: meta.height });
    console.log(`crops: ${file} ${meta.width}x${meta.height} ${(buf.length / 1024).toFixed(1)} KiB`);
  }
  const big = variants[variants.length - 1];
  manifest[name] = { src: big.src, w: big.w, h: big.h, srcset: variants.map((v) => `${v.src} ${v.w}w`).join(', ') };
}
writeFileSync(resolve(here, '../src/generated/crops.json'), JSON.stringify(manifest, null, 2) + '\n');
