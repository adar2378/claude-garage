import { existsSync, readdirSync } from 'node:fs';
import { latest } from './gitdates';

const mds = (dir: string) => readdirSync(dir).filter((f) => f.endsWith('.md')).map((f) => `${dir}/${f}`);

/** Sitemap lastmod for a site path, from git. Returns undefined when no source file is known. */
export function lastmodFor(pathname: string): string | undefined {
  const p = pathname.replace(/\/+$/, '') || '/';
  if (p === '/') return latest(['src/pages/index.astro', 'src/components/Landing.astro', 'src/styles/landing.css', 'src/scripts/landing.js']);
  if (p === '/try') {
    return latest(['src/pages/try.astro', 'src/components/TrySim.astro', 'src/components/TryLessons.astro', 'src/scripts/try.js', 'src/scripts/try-data.js', 'src/styles/try.css']);
  }
  if (p === '/blog') return latest(['src/pages/blog/index.astro', ...mds('src/content/blog')]);
  if (p === '/compare') return latest(['src/pages/compare/index.astro', ...mds('src/content/compare')]);
  const m = p.match(/^\/(blog|compare)\/([^/]+)$/);
  if (m) {
    const f = `src/content/${m[1]}/${m[2]}.md`;
    return existsSync(f) ? latest([f]) : undefined;
  }
  if (p === '/docs' || p.startsWith('/docs/')) {
    const rest = p.slice('/docs'.length).replace(/^\//, '');
    const base = rest ? `src/content/docs/${rest}` : 'src/content/docs/index';
    // Synced pages are copied in before the build, so they exist on disk too.
    const f = [`${base}.md`, `${base}/index.md`].find((c) => existsSync(c));
    if (f) return latest([f]);
  }
  return undefined;
}
