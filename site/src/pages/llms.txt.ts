import type { APIRoute } from 'astro';
import { getCollection } from 'astro:content';
import { docsInOrder, urlOf } from '../lib/docs';

export const GET: APIRoute = async ({ site }) => {
  const base = (site ?? new URL('https://claude-garage.xyz')).origin;
  const docs = await docsInOrder(await getCollection('docs'));
  const posts = (await getCollection('blog', (p) => !p.data.draft)).sort((a, b) => b.data.date.getTime() - a.data.date.getTime());
  const compareAll = (await getCollection('compare')).sort((a, b) => a.data.order - b.data.order);
  const compares = [...compareAll.filter((p) => p.data.kind === 'tool'), ...compareAll.filter((p) => p.data.kind !== 'tool')];
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
    '## Try it',
    '',
    `- [Try it in your browser](${base}/try/): A keyboard-driven simulation of the real wall with eight short lessons. Nothing to install.`,
    '',
    '## About',
    '',
    `- [About claude-garage and its author](${base}/about/): Who built claude-garage (Saiful Islam Adar, CTO and co-founder of Dev Monks), why, and the principles behind it.`,
    '',
    '## Blog',
    '',
    `- [Blog index](${base}/blog/): Notes on running many Claude Code sessions, tmux and plugins.`,
    ...posts.map((p) => `- [${p.data.title}](${base}/blog/${p.id}/): ${p.data.description}`),
    '',
    '## Compare',
    '',
    `- [Compare index](${base}/compare/): How claude-garage compares with other ways to run Claude Code sessions.`,
    `- [Best tools to run multiple Claude Code sessions](${base}/compare/alternatives/): A fair guide to claude-garage, herdr, cmux, Claude Squad, Emdash and Claude Code built-in features, with sources and when each fits. Checked 2026-11-01.`,
    ...compares.map((p) => `- [${p.data.title}](${base}/compare/${p.id}/): ${p.data.description}`),
    '',
  ];
  return new Response(lines.join('\n'), { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
