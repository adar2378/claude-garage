import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// One entity graph for the whole site. The full nodes live on the landing page (see entityGraph);
// every other page points at them by @id. Same origin and trailing-slash form everywhere.
export const ORIGIN = (process.env.SITE_URL ?? 'https://claude-garage.xyz').replace(/\/$/, '');
export const REPO = 'https://github.com/adar2378/claude-garage';
export const AUTHOR_URL = 'https://github.com/adar2378';
export const AUTHOR_NAME = 'Saiful Islam Adar';
export const AUTHOR_PAGE = '/about/';
export const DEVMONKS_URL = 'https://devmonks.co';
export const NOINDEX = process.env.SITE_NOINDEX === '1';

export const abs = (path: string) => `${ORIGIN}${path}`;
export const ID = {
  org: abs('/#organization'),
  person: abs('/#person'),
  website: abs('/#website'),
  software: abs('/#software'),
  blog: abs('/blog/#blog'),
} as const;
export const ref = (id: string) => ({ '@id': id });

// Refs for pages that are not the landing page.
export const AUTHOR = ref(ID.person);
export const PUBLISHER = ref(ID.org);
export const WEBSITE = ref(ID.website);
export const SOFTWARE = ref(ID.software);

/** Version from the Cargo workspace, read at build time (the build runs from site/). */
export function appVersion(): string {
  const toml = readFileSync(resolve(process.cwd(), '../Cargo.toml'), 'utf8');
  const section = toml.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/)?.[1] ?? '';
  const v = section.match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];
  if (!v) throw new Error('site: workspace version not found in ../Cargo.toml');
  return v;
}

export const SOFTWARE_DESCRIPTION =
  'claude-garage is a terminal wall for Claude Code. See every session at once, jump to the one that needs your answer, and keep sessions alive in tmux. One binary for macOS and Linux, with a Windows preview.';

/** The author. Rendered on the landing page and on /about/ (same node, same @id). */
export function personNode() {
  return {
    '@type': 'Person',
    '@id': ID.person,
    name: AUTHOR_NAME,
    jobTitle: 'CTO and co-founder',
    worksFor: { '@type': 'Organization', name: 'Dev Monks', url: DEVMONKS_URL },
    url: abs(AUTHOR_PAGE),
    sameAs: [AUTHOR_URL, DEVMONKS_URL],
  };
}

/** The full entity nodes. Rendered once, on the landing page. */
export function entityGraph() {
  return [
    {
      '@type': 'Organization',
      '@id': ID.org,
      name: 'claude-garage',
      url: abs('/'),
      logo: { '@type': 'ImageObject', url: abs('/logo-512.png'), width: 512, height: 512 },
      sameAs: [REPO],
      founder: ref(ID.person),
    },
    personNode(),
    {
      '@type': 'WebSite',
      '@id': ID.website,
      url: abs('/'),
      name: 'claude-garage',
      inLanguage: 'en',
      publisher: ref(ID.org),
    },
    {
      '@type': 'SoftwareApplication',
      '@id': ID.software,
      name: 'claude-garage',
      description: SOFTWARE_DESCRIPTION,
      url: abs('/'),
      image: abs('/og-default.png'),
      applicationCategory: 'DeveloperApplication',
      operatingSystem: 'macOS, Linux, Windows (preview)',
      softwareVersion: appVersion(),
      license: 'https://spdx.org/licenses/MIT.html',
      downloadUrl: `${REPO}/releases/latest`,
      installUrl: abs('/docs/guides/install/'),
      sameAs: [REPO],
      isAccessibleForFree: true,
      offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
      author: ref(ID.person),
      publisher: ref(ID.org),
    },
  ];
}

/** Full ISO datetime with timezone. Dates from front matter are UTC midnight. */
export const isoDate = (d: Date) => d.toISOString();
/** Date-only form, for <time datetime> and visible dates. */
export const dayOf = (d: Date) => d.toISOString().slice(0, 10);
export const longDate = (d: Date) =>
  d.toLocaleDateString('en-US', { year: 'numeric', month: 'long', day: 'numeric', timeZone: 'UTC' });

export function breadcrumbs(items: [name: string, path: string][]) {
  return {
    '@type': 'BreadcrumbList',
    itemListElement: items.map(([name, path], i) => ({ '@type': 'ListItem', position: i + 1, name, item: abs(path) })),
  };
}
