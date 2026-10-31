import type { APIRoute } from 'astro';
import { getCollection } from 'astro:content';
import { docsInOrder, urlOf } from '../lib/docs';

export const GET: APIRoute = async ({ site }) => {
  const base = (site ?? new URL('https://claude-garage.xyz')).origin;
  const docs = await docsInOrder(await getCollection('docs'));
  const posts = (await getCollection('blog', (p) => !p.data.draft)).sort((a, b) => b.data.date.getTime() - a.data.date.getTime());
  const compares = (await getCollection('compare')).sort((a, b) => a.data.order - b.data.order);
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
    '## Blog',
    '',
    `- [Blog index](${base}/blog/): Notes on running many Claude Code sessions, tmux and plugins.`,
    ...posts.map((p) => `- [${p.data.title}](${base}/blog/${p.id}/): ${p.data.description}`),
    '',
    '## Compare',
    '',
    `- [Compare index](${base}/compare/): How claude-garage compares with other ways to run Claude Code sessions.`,
    ...compares.map((p) => `- [${p.data.title}](${base}/compare/${p.id}/): ${p.data.description}`),
    '',
  ];
  return new Response(lines.join('\n'), { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
