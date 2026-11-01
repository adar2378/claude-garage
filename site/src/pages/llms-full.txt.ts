import type { APIRoute } from 'astro';
import { getCollection } from 'astro:content';
import { docsInOrder, urlOf } from '../lib/docs';

export const GET: APIRoute = async ({ site }) => {
  const base = (site ?? new URL('https://claude-garage.xyz')).origin;
  const docs = await docsInOrder(await getCollection('docs'));
  const parts = docs.map((d) => {
    const body = (d.body ?? '').replace(/<!--[\s\S]*?-->\n*/g, '').trim();
    return `# ${d.data.title}\n\nSource: ${base}${urlOf(d.id)}\n\n${body}\n`;
  });
  return new Response(`# claude-garage documentation\n\n${parts.join('\n---\n\n')}`, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
};
