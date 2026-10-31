import { defineCollection } from 'astro:content';
import { docsLoader } from '@astrojs/starlight/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

// All docs live under /docs/. Files stay in src/content/docs/<section>/ and the
// loader prefixes every id with "docs/", so guides/foo.md serves at /docs/guides/foo/.
const toId = ({ entry }: { entry: string }) => {
  const clean = entry.replace(/\.[^.]+$/, '').replace(/\/index$/, '').replace(/^index$/, '');
  return clean ? `docs/${clean.toLowerCase()}` : 'docs';
};

export const collections = {
  docs: defineCollection({ loader: docsLoader({ generateId: toId }), schema: docsSchema() }),
};
