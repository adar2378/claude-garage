import type { APIRoute } from 'astro';
import { getCollection } from 'astro:content';

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

export const GET: APIRoute = async ({ site }) => {
  const base = (site ?? new URL('https://claude-garage.xyz')).origin;
  const posts = (await getCollection('blog', (p) => !p.data.draft)).sort((a, b) => b.data.date.getTime() - a.data.date.getTime());
  const items = posts.map((p) => {
    const link = `${base}/blog/${p.id}/`;
    return `<item><title>${esc(p.data.title)}</title><link>${link}</link><guid isPermaLink="true">${link}</guid><pubDate>${p.data.date.toUTCString()}</pubDate><description>${esc(p.data.description)}</description><author>${esc(p.data.author)}</author></item>`;
  });
  const last = posts[0]?.data.date ?? new Date('2026-11-01');
  const xml = `<?xml version="1.0" encoding="UTF-8"?>\n<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom"><channel><title>claude-garage blog</title><link>${base}/blog/</link><description>Notes on running many Claude Code sessions, tmux, and claude-garage plugins.</description><language>en</language><lastBuildDate>${last.toUTCString()}</lastBuildDate><atom:link href="${base}/blog/rss.xml" rel="self" type="application/rss+xml"/>${items.join('')}</channel></rss>\n`;
  return new Response(xml, { headers: { 'Content-Type': 'application/rss+xml; charset=utf-8' } });
};
