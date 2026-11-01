import { defineCollection, z } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsLoader } from '@astrojs/starlight/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

// All docs live under /docs/. Files stay in src/content/docs/<section>/ and the
// loader prefixes every id with "docs/", so guides/foo.md serves at /docs/guides/foo/.
const toId = ({ entry }: { entry: string }) => {
  const clean = entry.replace(/\.[^.]+$/, '').replace(/\/index$/, '').replace(/^index$/, '');
  return clean ? `docs/${clean.toLowerCase()}` : 'docs';
};

// Blog posts. A post with draft: true is left out of production builds (index, feed, sitemap).
const blog = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/blog' }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    date: z.coerce.date(),
    updated: z.coerce.date().optional(),
    author: z.string().default('Saiful Islam'),
    tags: z.array(z.string()).default([]),
    draft: z.boolean().default(false),
  }),
});

// Comparison pages. The table, lists and FAQ live in frontmatter so the page and its JSON-LD share one source.
const compare = defineCollection({
  loader: glob({ pattern: '**/*.md', base: './src/content/compare' }),
  schema: z.object({
    title: z.string(),
    description: z.string(),
    short: z.string(),
    order: z.number().default(50),
    otherLabel: z.string(),
    intro: z.string(),
    who: z.array(z.string()),
    rows: z.array(z.object({ topic: z.string(), garage: z.string(), other: z.string() })),
    notNeeded: z.array(z.string()),
    faq: z.array(z.object({ q: z.string(), a: z.string() })),
  }),
});

export const collections = {
  docs: defineCollection({ loader: docsLoader({ generateId: toId }), schema: docsSchema() }),
  blog,
  compare,
};
