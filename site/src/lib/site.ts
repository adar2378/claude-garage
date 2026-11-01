import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// One entity graph for the whole site. Google does not follow @id across pages, so every page's @graph
// carries the full Person and Organization nodes (see sharedNodes). Same origin and trailing-slash form everywhere.
export const ORIGIN = (process.env.SITE_URL ?? 'https://claude-garage.xyz').replace(/\/$/, '');
export const REPO = 'https://github.com/adar2378/claude-garage';
export const AUTHOR_URL = 'https://github.com/adar2378';
export const AUTHOR_NAME = 'Saiful Islam';
export const AUTHOR_ALT_NAME = 'Adar';
/** Visible byline form. */
export const AUTHOR_BYLINE = 'Saiful Islam (Adar)';
export const AUTHOR_PAGE = '/about/';
export const DEVMONKS_URL = 'https://www.devmonks.co/';
export const PERSONAL_URL = 'https://saifulislamadar.com/';
export const NPM_URL = 'https://www.npmjs.com/package/claude-garage';
export const NOINDEX = process.env.SITE_NOINDEX === '1';

export const abs = (path: string) => `${ORIGIN}${path}`;
export const ID = {
  org: abs('/#organization'),
  person: abs('/#person'),
  website: abs('/#website'),
  software: abs('/#software'),
  blog: abs('/blog/#blog'),
  source: abs('/#source'),
} as const;
export const ref = (id: string) => ({ '@id': id });

// Refs to the entities, used alongside sharedNodes().
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
  warnPrerelease(v);
  return v;
}

let warned = false;
function warnPrerelease(v: string) {
  if (warned || NOINDEX || !/-/.test(v)) return;
  warned = true;
  console.warn(`\n[site] WARNING: softwareVersion is "${v}" (pre-release, from ../Cargo.toml) and SITE_NOINDEX is not set. Public JSON-LD would advertise a version that is not on npm. Bump Cargo.toml before a public deploy.\n`);
}

export const SOFTWARE_DESCRIPTION =
  'claude-garage is a terminal wall for Claude Code. See every session at once, jump to the one that needs your answer, and keep sessions alive in tmux. One binary for macOS and Linux, with a Windows preview.';

/** The author. Same node, same @id, on every page. */
export function personNode() {
  return {
    '@type': 'Person',
    '@id': ID.person,
    name: AUTHOR_NAME,
    alternateName: AUTHOR_ALT_NAME,
    jobTitle: 'Chief Technology Officer',
    image: 'https://saifulislamadar.com/prints/portrait.webp',
    worksFor: {
      '@type': 'Organization',
      name: 'Dev Monks',
      url: DEVMONKS_URL,
      sameAs: ['https://www.linkedin.com/company/dev-monks/'],
    },
    url: abs(AUTHOR_PAGE),
    sameAs: [
      PERSONAL_URL,
      'https://www.linkedin.com/in/saiful-islam-adar/',
      AUTHOR_URL,
      'https://x.com/adar2378',
      'https://pub.dev/publishers/theadar.xyz/packages',
    ],
  };
}

export function organizationNode() {
  return {
    '@type': 'Organization',
    '@id': ID.org,
    name: 'claude-garage',
    url: abs('/'),
    logo: { '@type': 'ImageObject', url: abs('/logo-512.png'), width: 512, height: 512 },
    sameAs: [REPO],
    founder: ref(ID.person),
  };
}

/** The Organization and Person nodes in full. Put these in every page's @graph. */
export function sharedNodes() {
  return [organizationNode(), personNode()];
}

/** Landing-page graph: the shared nodes plus the website, the app and its source. */
export function entityGraph() {
  return [
    ...sharedNodes(),
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
      softwareRequirements: 'tmux 3.2 or newer; Claude Code',
      softwareVersion: appVersion(),
      license: 'https://spdx.org/licenses/MIT.html',
      downloadUrl: `${REPO}/releases/latest`,
      installUrl: abs('/docs/guides/install/'),
      sameAs: [REPO, NPM_URL],
      featureList: [
        'Every Claude Code session on one terminal wall, grouped by project',
        'The session waiting for your answer turns amber',
        'One key jumps to the session that needs you',
        'Sessions run in tmux and survive closing the terminal',
        'Restore stopped sessions after a reboot with their conversation',
        'Optional git worktree per session',
        'Plugins for keys, widgets, panes and commands',
      ],
      isAccessibleForFree: true,
      offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
      author: ref(ID.person),
      publisher: ref(ID.org),
    },
    {
      '@type': 'SoftwareSourceCode',
      '@id': ID.source,
      name: 'claude-garage source code',
      codeRepository: REPO,
      programmingLanguage: 'Rust',
      license: 'https://spdx.org/licenses/MIT.html',
      author: ref(ID.person),
      targetProduct: ref(ID.software),
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
