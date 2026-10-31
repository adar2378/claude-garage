import { defineRouteMiddleware } from '@astrojs/starlight/route-data';

// Adds per-page TechArticle JSON-LD and the shared OG image.
export const onRequest = defineRouteMiddleware((context) => {
  const { head, entry, entryMeta } = context.locals.starlightRoute;
  const site = context.site ?? new URL('https://claude-garage.xyz');
  const url = new URL(context.url.pathname, site).href;
  const image = new URL('/og-default.png', site).href;
  const description = entry.data.description ?? 'claude-garage documentation.';
  const jsonLd = {
    '@context': 'https://schema.org',
    '@type': 'TechArticle',
    headline: entry.data.title,
    description,
    url,
    mainEntityOfPage: url,
    image,
    inLanguage: 'en',
    isPartOf: { '@type': 'WebSite', name: 'claude-garage', url: site.href },
    author: { '@type': 'Organization', name: 'claude-garage', url: 'https://github.com/adar2378/claude-garage' },
  };
  head.push(
    { tag: 'script', attrs: { type: 'application/ld+json' }, content: JSON.stringify(jsonLd) },
    { tag: 'meta', attrs: { property: 'og:image', content: image } },
    { tag: 'meta', attrs: { property: 'og:image:width', content: '1200' } },
    { tag: 'meta', attrs: { property: 'og:image:height', content: '630' } },
    { tag: 'meta', attrs: { name: 'twitter:image', content: image } },
    { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
  );
  void entryMeta;
});
