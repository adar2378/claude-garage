// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import sitemap from '@astrojs/sitemap';
import { lastmodFor } from './src/lib/lastmod.ts';

// TODO confirm domain. Override per environment: SITE_URL=https://example.com npm run build
const SITE = (process.env.SITE_URL ?? 'https://claude-garage.xyz').replace(/\/$/, '');
// SITE_NOINDEX=1 builds a preview that search engines must skip: noindex on every page, robots.txt disallows all, no sitemap.
const NOINDEX = process.env.SITE_NOINDEX === '1';
const DESCRIPTION =
  'claude-garage is a terminal wall for Claude Code. See every session at once, jump to the one that needs you, and keep sessions alive in tmux.';

export default defineConfig({
  site: SITE,
  output: 'static',
  build: { inlineStylesheets: 'never' },
  trailingSlash: 'ignore',
  integrations: [
    starlight({
      title: 'claude-garage',
      description: DESCRIPTION,
      logo: { src: './src/assets/logo.svg', alt: '', replacesTitle: false },
      favicon: '/favicon.svg',
      lastUpdated: true,
      head: [
        { tag: 'link', attrs: { rel: 'icon', href: '/favicon.ico', sizes: '32x32' } },
        { tag: 'link', attrs: { rel: 'apple-touch-icon', href: '/apple-touch-icon.png' } },
        { tag: 'link', attrs: { rel: 'manifest', href: '/site.webmanifest' } },
        ...(NOINDEX ? [{ tag: /** @type {'meta'} */ ('meta'), attrs: { name: 'robots', content: 'noindex' } }] : []),
      ],
      disable404Route: true,
      editLink: { baseUrl: 'https://github.com/adar2378/claude-garage/edit/main/site/' },
      customCss: [
        '@fontsource-variable/geist/index.css',
        '@fontsource-variable/geist-mono/index.css',
        './src/styles/theme.css',
      ],
      components: { Head: './src/components/Head.astro', SocialIcons: './src/components/SocialIcons.astro' },
      routeMiddleware: './src/routeData.ts',
      sidebar: [
        { label: 'Getting started', items: [
          { slug: 'docs/guides/install' },
          { slug: 'docs/getting-started/first-run' },
        ] },
        { label: 'Guides', items: [
          { slug: 'docs/guides/first-wall' },
          { slug: 'docs/guides/many-sessions' },
          { slug: 'docs/guides/alerts' },
          { slug: 'docs/guides/worktrees' },
          { slug: 'docs/guides/nothing-lost' },
          { slug: 'docs/guides/plugins' },
          { slug: 'docs/guides/from-terminal-tabs' },
          { slug: 'docs/guides/first-plugin' },
          { slug: 'docs/guides/configuration' },
          { slug: 'docs/guides/contributing' },
        ] },
        { label: 'Plugins', items: [{ autogenerate: { directory: 'docs/plugins' } }] },
        { label: 'Reference', items: [
          { slug: 'docs/reference/keys' },
          { slug: 'docs/reference/commands' },
        ] },
        { slug: 'docs/platforms', label: 'Platforms' },
        { slug: 'docs/changelog' },
        { label: 'Blog', link: '/blog/' },
        { label: 'Compare', link: '/compare/' },
      ],
    }),
    ...(NOINDEX
      ? []
      : [
          sitemap({
            filter: (page) => !/\/404\/?$/.test(page),
            serialize(item) {
              const lastmod = lastmodFor(new URL(item.url).pathname);
              return lastmod ? { ...item, lastmod } : item;
            },
          }),
        ]),
  ],
});
