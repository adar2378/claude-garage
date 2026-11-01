// Renders the brand mark (public/favicon.svg, the same mark as src/assets/logo.svg) into the raster icons.
// Outputs are committed: logo-512.png (Organization logo), apple-touch-icon.png (180), favicon.ico (32),
// icon-192.png (web manifest) and icon-maskable-512.png (manifest, purpose maskable: full-bleed ink with the mark in the safe zone).
// Run `npm run icons` after changing the mark.
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readFileSync, writeFileSync } from 'node:fs';
import sharp from 'sharp';

const here = dirname(fileURLToPath(import.meta.url));
const pub = resolve(here, '../public');
const svg = readFileSync(resolve(pub, 'favicon.svg'));
const png = (size) => sharp(svg, { density: Math.ceil((72 * size) / 32) * 2 }).resize(size, size).png().toBuffer();

writeFileSync(resolve(pub, 'logo-512.png'), await png(512));
// iOS paints transparent corners black and rounds them itself, so the touch icon is a solid square.
writeFileSync(resolve(pub, 'apple-touch-icon.png'), await sharp(await png(180)).flatten({ background: '#14151A' }).png().toBuffer());

writeFileSync(resolve(pub, 'icon-192.png'), await png(192));
// Maskable: the OS crops to its own shape, so no rounded corners; the mark sits well inside the central 80%.
const inner = readFileSync(resolve(pub, 'favicon.svg'), 'utf8').match(/<svg[^>]*>([\s\S]*)<\/svg>/)[1].replace(/<rect[^>]*\/>/, '');
const maskable = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><rect width="512" height="512" fill="#14151A"/><g transform="translate(64 64) scale(12)">${inner}</g></svg>`;
writeFileSync(resolve(pub, 'icon-maskable-512.png'), await sharp(Buffer.from(maskable), { density: 72 }).resize(512, 512).png().toBuffer());

// favicon.ico: an ICO container holding one 32x32 PNG image.
const img = await png(32);
const dir = Buffer.alloc(22);
dir.writeUInt16LE(0, 0); // reserved
dir.writeUInt16LE(1, 2); // type: icon
dir.writeUInt16LE(1, 4); // one image
dir.writeUInt8(32, 6); // width
dir.writeUInt8(32, 7); // height
dir.writeUInt16LE(1, 10); // planes
dir.writeUInt16LE(32, 12); // bits per pixel
dir.writeUInt32LE(img.length, 14);
dir.writeUInt32LE(22, 18); // data offset
writeFileSync(resolve(pub, 'favicon.ico'), Buffer.concat([dir, img]));
console.log('icons: logo-512.png, apple-touch-icon.png, favicon.ico, icon-192.png, icon-maskable-512.png');
