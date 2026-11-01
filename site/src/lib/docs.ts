import type { CollectionEntry } from 'astro:content';

export const urlOf = (id: string) => `/${id}/`.replace('//', '/');

const SECTIONS = ['docs/getting-started', 'docs/guides', 'docs/reference', 'docs/plugins', 'docs/platforms', 'docs/changelog'];

// Reading order: overview, getting started, guides, reference, plugins, platforms, changelog.
export async function docsInOrder(all: CollectionEntry<'docs'>[]) {
  const rank = (id: string) => {
    if (id === 'docs') return -1;
    const i = SECTIONS.findIndex((s) => id === s || id.startsWith(s + '/'));
    return i < 0 ? 99 : i;
  };
  return [...all]
    .filter((d) => d.id !== '404')
    .sort((a, b) => {
      const ra = rank(a.id), rb = rank(b.id);
      if (ra !== rb) return ra - rb;
      const oa = a.data.sidebar?.order ?? 50, ob = b.data.sidebar?.order ?? 50;
      return oa - ob || a.id.localeCompare(b.id);
    });
}
