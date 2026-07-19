# Claude design system — extracted 2026-07-19

Source: `/Applications/Claude.app/Contents/Resources/app.asar` (Claude Desktop, Anthropic's shipping design tokens, verbatim) + `claude` CLI 2.1.214 live rendering. Values are HSL triplets as shipped (`hsl(H S% L%)`), hex approximations for reference.

## Dark theme (`.darkTheme`)

| Token | HSL | ≈ Hex | Garage mapping |
|---|---|---|---|
| bg-000 (raised) | 60 2.1% 18.4% | `#30302e` | panel/sel |
| bg-100 | 60 2.7% 14.5% | `#262624` | panel |
| bg-200 | 30 3.3% 11.8% | `#1f1e1d` | bg alt |
| bg-300 (base) | 60 2.6% 7.6% | `#141413` | **bg** (matches CLI strings `#141413`) |
| bg-400/500 | 0 0% 0% | `#000000` | overlay backdrop |
| text-000/100 | 48 33.3% 97.1% | `#faf9f5` | ink strong |
| text-200/300 | 50 9% 73.7% | `#c2c0b6` | ink |
| text-400/500 | 48 4.8% 59.2% | `#9c9a91` | dim |
| border-* | 51 16.5% 84.5% @ low alpha | `#dcd9c5`/15% | line |
| brand / accent-brand | 15 63.1% 59.6% | `#d97757` | **amber→terracotta** (the Claude orange) |
| accent-100/200 (blue) | 210 70.9% 51.6% | `#2c84db` | blue |
| danger-100 | 0 67% 59.6% | `#dd5353` | red |
| success-100 | 97 75% 32.9% | `#519313` | green |

## Light theme (`:root`)

| Token | HSL | ≈ Hex |
|---|---|---|
| bg-000 | 0 0% 100% | `#ffffff` |
| bg-100 | 48 33.3% 97.1% | `#faf9f5` (ivory) |
| bg-200 | 53 28.6% 94.5% | `#f5f3e9` |
| bg-300 | 48 25% 92.2% | `#f0eee2` |
| bg-400/500 | 50 20.7% 88.6% | `#e8e5d5` |
| text-000/100 | 60 2.6% 7.6% | `#141413` |
| text-200/300 | 60 2.5% 23.3% | `#3d3d3a` |
| text-400/500 | 51 3.1% 43.7% | `#73726c` |
| border-* | 30 3.3% 11.8% @ low alpha | `#1f1e1d`/15% |
| brand-000/100 | 15 54.2% 51.2% | `#c96442` |
| accent-000 (blue) | 210 73.7% 40.2% | `#1a6fc4` |
| danger-100 | 0 56.2% 45.4% | `#b53232` |
| success-100 | 103 72.3% 26.9% | `#2f7613` |

CLI cross-check: in a 256-color tmux, Claude Code's accent renders as color 174 (`d78787`) — the quantized terracotta; secondary text 246, warnings 220. Consistent with brand `#d97757`.

## Typography

- **Anthropic Sans** (variable ttf) — UI text. **Anthropic Serif** — headings/serif accents.
- Mono stack (used verbatim by Claude Desktop): `ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas`.

⚠️ **Licensing**: Anthropic Sans/Serif are proprietary fonts bundled inside Claude's own apps. They must NOT be committed to this repo or shipped in the npm package. Implementation rule: declare `"Anthropic Sans"` first in the font stack so it picks up when locally installed, with clean fallbacks (`-apple-system … sans-serif`); mono UI keeps the ui-monospace stack (visually equivalent to what Claude Code shows in a default macOS terminal).

## Implementation sketch (p5)

- Extend `@theme` in `ui/src/index.css` to define both palettes; theme setting (`dark` / `light` / `system`) in the settings popover; `data-theme` on `<main>`; xterm terminal theme object switches with it (bg/fg per palette).
- Keep glyph semantics: needs-input dot moves from garage-amber to Claude terracotta `#d97757`.
