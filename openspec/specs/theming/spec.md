# theming

## Purpose

Viewer-selectable palettes over one token set — the original garage dark plus the two Claude Desktop palettes (CLAUDE-THEME.md) — with terminals following, and Google Sans Code as the bundled default font.

## Requirements

### Requirement: Theme setting with four options
The settings popover SHALL offer a theme choice of `garage` (default), `claude dark`, `claude light`, and `system`, persisted per browser alongside the other viewer settings. `system` SHALL resolve to `claude dark` or `claude light` from the OS `prefers-color-scheme` and SHALL track OS changes live. Switching themes SHALL re-skin the running UI without a page reload, and popout windows SHALL follow the main window's choice via the existing cross-window settings sync.

#### Scenario: Live switch without reload
- **WHEN** the user selects `claude light` in the settings popover while terminals are streaming
- **THEN** the chrome and every terminal re-render in the light palette immediately, with no reload and no terminal reconnect

#### Scenario: System follows the OS
- **WHEN** the theme is set to `system` and the OS switches from dark to light appearance
- **THEN** the UI moves from the claude-dark to the claude-light palette without user action

### Requirement: One token set, palette overrides
Themes SHALL be implemented as overrides of the existing `--color-garage-*` custom properties (stamped via `data-theme` on the document element), so every component — including the dockview skin — re-skins without per-component changes. The accent slot SHALL keep its semantic role in every palette (garage amber / Claude terracotta): needs-input signalling always renders in the accent color.

#### Scenario: Dockview chrome follows the palette
- **WHEN** the theme changes
- **THEN** the grid separators, drag-over highlights, and tab strips render in the new palette without dockview-specific configuration

### Requirement: Terminals follow the theme
Because xterm.js paints a canvas that CSS variables cannot reach, each palette SHALL carry a matching terminal theme object (background, foreground, cursor, selection; the light palette also overrides the ANSI colors, whose defaults assume a dark background). Theme changes SHALL apply to already-open terminals in place, preserving scrollback.

#### Scenario: Light terminals get readable ANSI colors
- **WHEN** the claude-light theme is active and a session prints ANSI-colored output
- **THEN** the colors render from the light-tuned ANSI set, not xterm defaults designed for dark backgrounds

### Requirement: Google Sans Code as the default font
The UI and terminals SHALL default to Google Sans Code, bundled with the package (OFL-licensed), with a system-mono fallback stack (`ui-monospace`, SF Mono, Menlo, …) so nothing breaks offline or before the font loads. Terminals SHALL refit their cell metrics once fonts finish loading so glyph widths are measured against the real font.

#### Scenario: Fallback before the font loads
- **WHEN** the page renders before the bundled font finishes loading
- **THEN** text renders in the system mono fallback and terminals re-measure/refit once `document.fonts` settles
