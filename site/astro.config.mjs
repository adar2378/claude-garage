// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import sitemap from '@astrojs/sitemap';

// TODO confirm domain. Override per environment: SITE_URL=https://example.com npm run build
const SITE = (process.env.SITE_URL ?? 'https://claude-garage.xyz').replace(/\/$/, '');
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
          { slug: 'docs/getting-started/install' },
          { slug: 'docs/getting-started/first-run' },
        ] },
        { label: 'Guides', items: [{ autogenerate: { directory: 'docs/guides' } }] },
        { label: 'Plugins', items: [{ autogenerate: { directory: 'docs/plugins' } }] },
        { label: 'Reference', items: [
          { slug: 'docs/reference/keys' },
          { slug: 'docs/reference/commands' },
        ] },
        { slug: 'docs/platforms' },
        { slug: 'docs/changelog' },
        { label: 'Blog', link: '/blog/' },
        { label: 'Compare', link: '/compare/' },
      ],
    }),
    sitemap({ filter: (page) => !/\/404\/?$/.test(page) }),
  ],
});
