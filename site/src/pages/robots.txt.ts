import type { APIRoute } from 'astro';
import { NOINDEX } from '../lib/site';

export const GET: APIRoute = ({ site }) => {
  const base = (site ?? new URL('https://claude-garage.xyz')).origin;
  const body = NOINDEX
    ? 'User-agent: *\nDisallow: /\n'
    : `User-agent: *\nAllow: /\nDisallow: /pagefind/\n\nSitemap: ${base}/sitemap-index.xml\n`;
  return new Response(body, { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
