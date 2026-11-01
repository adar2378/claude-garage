// Where each page's social card lives. scripts/og-image.mjs writes the files; keep the naming in step with it.
export const OG_DEFAULT = '/og-default.png';
export const OG_DEFAULT_ALT = 'claude-garage: every Claude session, one calm wall. A terminal wall listing Claude Code sessions, with one waiting for your answer.';

/** kind 'blog' | 'compare' with the entry id, or 'docs' with the docs entry id (already "docs/..."). */
export function ogCard(kind: 'blog' | 'compare' | 'docs', id: string) {
  const slug = kind === 'docs' ? id : `${kind}-${id}`;
  return `/og/${slug.replace(/\//g, '-')}.png`;
}

export function ogAlt(kind: 'blog' | 'compare' | 'docs', title: string) {
  const what = { blog: 'Blog post', compare: 'Comparison page', docs: 'Documentation page' }[kind];
  return `claude-garage ${what.toLowerCase()}: ${title}`;
}
