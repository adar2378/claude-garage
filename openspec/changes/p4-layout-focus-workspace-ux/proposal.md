# Proposal: p4-layout-focus-workspace-ux

## Why

The spec'd product (P0–P3) is complete; P4 is the first driven-by-usage change. Three requests from dogfooding: the fixed vertical stack doesn't fit every screen (VS Code-style drag-docking and pop-out windows do); everything on screen competes at equal brightness (a focus-dimming mode should spotlight what you're working on — with attention signals exempt); and adding a workspace by typing an absolute path is the clumsiest interaction in the app (a native directory picker with auto-naming and path-based grouping removes it).

## What Changes

- **Pop-out terminal windows**: a cell control opens any session in its own browser window (second-monitor use). The main grid shows a "popped out" placeholder while the window lives — deliberately, to avoid tmux's smallest-client sizing fight — and reclaims the cell when it closes.
- **Drag-to-dock grid**: cells become draggable with VS Code-style drop targets (above/below/left/right) and resizable splitters, via a docking layout library. Per-workspace layout persists client-side; new sessions join the layout automatically; a reset-layout control restores the default stack.
- **Focus dimming mode** (off by default): when enabled, the focused surface renders at full brightness and everything else dims. Granularity is fine (rail rows, cells, panes) so `needs-input` rows/cells are **never dimmed** — attention routing pierces the effect by design.
- **Settings**: a header gear opens a small settings popover (first setting: focus dimming toggle). Preferences are per-browser (localStorage), not daemon state.
- **Directory-picker workspace add**: the add-workspace flow calls the daemon, which opens a native macOS folder picker (`osascript choose folder`) and returns the absolute path — sidestepping the browser File System Access API's refusal to expose real paths. Name auto-derives from the folder's basename (kebab-cased to fit `[a-z0-9-]+`), editable inline before saving and renameable after.
- **Workspace rename**: `PATCH /api/workspaces/:name` renames the registry entry and, for live sessions, renames the tmux sessions (`garage/<old>/<label>` → `garage/<new>/<label>`) and their resume metadata so nothing orphans.
- **Nested workspace grouping**: purely presentational — a workspace whose directory sits inside another workspace's directory renders indented under it in the rail. The registry stays a flat map; containment is derived at render time.

Out of scope: free-floating panels inside the page (fights the pit-wall concept), tabs within dock groups, cross-browser layout sync, Linux/Windows pickers (manual path input remains the fallback everywhere).

## Capabilities

### New Capabilities
- `flexible-layout`: pop-out windows (placeholder + reclaim semantics) and the drag-dock grid (drop targets, splitters, persistence, reset, auto-join/leave).
- `focus-dimming`: the dimming model, its focus-target resolution, the attention-exemption rule, and the settings surface that gates it.
- `workspace-picker`: the native-picker endpoint, auto-naming, rename endpoint semantics (registry + tmux + metadata), and derived nesting.

### Modified Capabilities
- `pit-wall-ui`: the "New-session affordance per workspace" requirement's add-workspace flow changes (picker-first with manual fallback); keybinding `1–9` indexing is defined over the flattened rendered order when nesting is present.

## Impact

- UI: new dependency for docking (dockview); settings module; popout route; tree-building in the rail. Largest UI change since P1.
- Daemon: two small endpoints (`/api/pick-directory`, `PATCH /api/workspaces/:name`) — the picker one is darwin-gated (501 elsewhere), both behind the Origin allowlist. Rename touches registry, live tmux sessions, and the sessions metadata map.
- No changes to session lifecycle, status, diff, restore, or packaging semantics.
