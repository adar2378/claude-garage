import { defineRouteMiddleware } from '@astrojs/starlight/route-data';
import { AUTHOR, PUBLISHER, SOFTWARE, WEBSITE, abs, breadcrumbs, sharedNodes } from './lib/site';
import { fileDates } from './lib/gitdates';
import { ogAlt, ogCard } from './lib/og';

const SECTIONS: Record<string, string> = {
  'getting-started': 'Getting started',
  guides: 'Guides',
  reference: 'Reference',
  plugins: 'Plugins',
};

// Adds per-page TechArticle (CollectionPage for /docs/) + BreadcrumbList JSON-LD and the page's own OG card.
// The full Organization and Person nodes go in every graph; website and software are referenced by @id.
export const onRequest = defineRouteMiddleware((context) => {
  const { head, entry } = context.locals.starlightRoute;
  const path = context.url.pathname.endsWith('/') ? context.url.pathname : `${context.url.pathname}/`;
  const url = abs(path);
  const image = abs(ogCard('docs', entry.id));
  const imageAlt = ogAlt('docs', entry.data.title);
  const description = entry.data.description ?? 'claude-garage documentation.';

  const file = (entry as { filePath?: string }).filePath ?? `src/content/${entry.id}.md`;
  const dates = fileDates(file);

  const isDocsHome = entry.id === 'docs';

  // docs/guides/install -> Home > Docs > Guides > Install claude-garage
  const parts = entry.id.split('/').slice(1);
  const crumbs: [string, string][] = [['Home', '/']];
  if (parts.length > 0) {
    crumbs.push(['Docs', '/docs/']);
    const section = SECTIONS[parts[0]];
    if (section && parts.length === 1) {
      // a section index page is the section itself
      crumbs.push([section, path]);
    } else if (section) {
      crumbs.push([section, `/docs/${parts[0]}/`]);
      crumbs.push([entry.data.title, path]);
    } else {
      crumbs.push([entry.data.title, path]);
    }
  } else {
    crumbs.push(['Docs', path]);
  }

  const jsonLd = {
    '@context': 'https://schema.org',
    '@graph': [
    ...sharedNodes(),
      {
        '@type': isDocsHome ? 'CollectionPage' : 'TechArticle',
        '@id': `${url}#${isDocsHome ? 'page' : 'article'}`,
        ...(isDocsHome ? { name: entry.data.title } : { headline: entry.data.title }),
        description,
        url,
        mainEntityOfPage: url,
        image,
        inLanguage: 'en',
        datePublished: dates.published,
        dateModified: dates.modified,
        isPartOf: WEBSITE,
        about: SOFTWARE,
        author: AUTHOR,
        publisher: PUBLISHER,
      },
      breadcrumbs(crumbs),
    ],
  };
  head.push(
    { tag: 'script', attrs: { type: 'application/ld+json' }, content: JSON.stringify(jsonLd) },
    { tag: 'meta', attrs: { property: 'og:image', content: image } },
    { tag: 'meta', attrs: { property: 'og:image:width', content: '1200' } },
    { tag: 'meta', attrs: { property: 'og:image:height', content: '630' } },
    { tag: 'meta', attrs: { property: 'og:image:alt', content: imageAlt } },
    { tag: 'meta', attrs: { name: 'twitter:image', content: image } },
    { tag: 'meta', attrs: { name: 'twitter:image:alt', content: imageAlt } },
    { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
  );
});
