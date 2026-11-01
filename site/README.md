# claude-garage site

Astro + Starlight, static output, deployed to Cloudflare Workers static assets (`wrangler.jsonc`).

- `/` landing page (`src/pages/index.astro`, hosts `src/components/Landing.astro`, currently a stub)
- `/docs/...` Starlight docs. Hand-written pages and `guides/` live in `src/content/docs/`; the loader prefixes every id with `docs/`.
- `/llms.txt`, `/llms-full.txt`, `/robots.txt`, `/sitemap-index.xml`, `/og-default.png` are generated at build.

## Develop

Needs Node 22.23.4 or newer (Astro 7 requirement). `.node-version` pins 24.

```bash
cd site
npm ci
npm run dev       # syncs repo docs, then starts Astro on :4321
npm run build     # sync + OG image + astro build -> site/dist
npm run check     # astro check
```

`scripts/sync-docs.mjs` runs before dev and build. It copies `docs/plugins/*.md`, `docs/PLATFORMS.md` and `CHANGELOG.md` from the repo, adds frontmatter, rewrites links, and generates `reference/keys.md` from `BINDINGS` in `wall/src/ui/help.rs`. Generated files are gitignored. Edit the repo sources, never the copies. Because it reads `../docs`, the build needs the full repo checkout, not just `site/`.

`scripts/og-image.mjs` composes `public/og-default.png` (1200x630) from `docs/brand/renders/wall-needs-input.svg`.

## Configuration

| Variable | Default | Use |
|---|---|---|
| `SITE_URL` | `https://claude-garage.xyz` | Canonical URLs, sitemap, OG tags, llms.txt. TODO confirm the domain. |
| `SITE_NOINDEX` | unset | Set to `1` for a preview build: `noindex` meta on every page, `robots.txt` says `Disallow: /`, no sitemap, and an `X-Robots-Tag: noindex` header. |
| `BRAND_ACCENT` | `#E85002` | OG image accent bar only. CSS accent lives in `src/styles/theme.css` (`--brand`). |

## Deploy (Cloudflare Workers static assets)

Run every command from `site/`.

- **Preview** (must not be indexed): `SITE_NOINDEX=1 npm run build && npx wrangler deploy`
- **Production** (indexable): `npm run build && npx wrangler deploy`, with `SITE_NOINDEX` unset.

The custom domain (`claude-garage.xyz`) is attached on release day, not before. Until then the site lives on its `workers.dev` address, built with `SITE_NOINDEX=1`. On release day: attach the domain to the Worker, build without `SITE_NOINDEX`, deploy, then submit the sitemap.

`npm run build` also writes the sha256 of the one inline script on `/try/` into `dist/_headers` (`scripts/postbuild.mjs`), so deploy the `dist` it produced. `npm run check:links` crawls `dist` for broken internal links. Page dates in JSON-LD and the sitemap come from git history, so build from a full clone, not a shallow one. `public/_redirects` redirects the old install URL to `/docs/guides/install/`. Check `/.well-known/security.txt` after the first deploy: it must return 200.

`npm run icons` regenerates `logo-512.png`, `apple-touch-icon.png` and `favicon.ico` from `public/favicon.svg`.

## Deploy to Cloudflare Pages (older notes)

1. Push the branch to GitHub.
2. Cloudflare dashboard, Workers & Pages, Create, Pages, Connect to Git. Pick the `claude-garage` repo.
3. Build settings:
   - Production branch: `main` (or the branch you ship from)
   - Framework preset: Astro (or None)
   - Root directory: `site`
   - Build command: `npm ci && npm run build`
   - Build output directory: `dist`
4. Environment variables (Production and Preview):
   - `NODE_VERSION` = `24` (must be 22.23.4 or newer)
   - `SITE_URL` = `https://<your domain>`
5. Save and deploy. `public/_headers` is picked up automatically (security headers and asset caching).

Watch out: the build reads `../docs`, `../CHANGELOG.md` and `../wall`, so leave the root directory as `site` and do not enable a "sparse" checkout of `site/` only.

### Custom domain: Namecheap to Cloudflare

Fastest path is to move DNS to Cloudflare:

1. Cloudflare dashboard, Add a site, enter the domain, choose the Free plan. Cloudflare shows two nameservers (for example `xxx.ns.cloudflare.com`).
2. Namecheap, Domain List, Manage, Nameservers, choose "Custom DNS", paste the two Cloudflare nameservers, save. Propagation takes minutes to a few hours. Cloudflare emails when the zone is active.
3. Pages project, Custom domains, Set up a domain. Enter the apex (`claude-garage.xyz`). Cloudflare creates the CNAME automatically because the zone is on Cloudflare. Repeat for `www`, then add a redirect rule from `www` to the apex (Rules, Redirect Rules).
4. SSL/TLS: set the mode to Full (strict). Turn on Always Use HTTPS.
5. Set `SITE_URL` to the final domain and redeploy so canonical URLs and the sitemap match.

If you would rather keep DNS at Namecheap, add a `CNAME` record for `www` to `<project>.pages.dev` and a URL redirect from the apex to `www`. Apex CNAME is not possible at Namecheap, so the Cloudflare nameserver route is recommended.

### After the first deploy

- Submit `https://<domain>/sitemap-index.xml` in Google Search Console and Bing Webmaster Tools.
- Check the preview with an OG debugger and `curl -I` for the headers.

## Theme

`src/styles/theme.css` holds the CSS variables. One accent (`--brand`, #E85002), with hover, pressed and high-contrast link shades derived from it. Amber `#FFB020` (`--needs-input`) is only for "needs input" mentions. Fonts are Geist and Geist Mono, self-hosted through `@fontsource-variable`, so the CSP allows only `self`.

## TODO

- Confirm the domain.
- Replace the stub `Landing.astro` with the real design.
- Replace `public/favicon.svg` and `src/assets/logo.svg` with the brand mark.
- `reference/commands.md` is hand-written; generate it once the CLI exposes a command list.
- The OG image text uses a system sans-serif (no web font in librsvg). Swap for Geist TTF if wanted.
