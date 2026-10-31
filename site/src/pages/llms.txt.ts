import type { APIRoute } from 'astro';
import { getCollection } from 'astro:content';
import { docsInOrder, urlOf } from '../lib/docs';

export const GET: APIRoute = async ({ site }) => {
  const base = (site ?? new URL('https://claude-garage.xyz')).origin;
  const docs = await docsInOrder(await getCollection('docs'));
  const lines = [
    '# claude-garage',
    '',
    '> A terminal wall for Claude Code. See every session at once, jump to the one that needs your answer, and keep sessions alive in tmux.',
    '',
    `Full documentation in one file: ${base}/llms-full.txt`,
    '',
    '## Docs',
    '',
    ...docs.map((d) => `- [${d.data.title}](${base}${urlOf(d.id)}): ${d.data.description ?? ''}`.trimEnd()),
    '',
  ];
  return new Response(lines.join('\n'), { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
