# Design: p4-layout-focus-workspace-ux

## Context

First post-spec change, driven by real usage. The architecture makes the layout features unusually cheap: terminals are thin viewers over tmux, so destroying and reattaching a terminal anywhere (another pane, another window) costs one WebSocket handshake and tmux repaints the screen — the migration problem that makes VS Code's terminal-docking hard does not exist here. The workspace-picker feature has one genuine constraint to design around: browsers deliberately never expose absolute filesystem paths (the File System Access API returns opaque handles), so a native picker must be daemon-side.

## Goals / Non-Goals

**Goals:**
- VS Code-feel drag-docking and pop-out windows without breaking the zero-config default (focus a workspace → sensible layout, no arranging required).
- A focus-dimming mode that spotlights the working surface while guaranteeing attention signals (needs-input) always render at full brightness.
- One-click workspace add via native folder picker; auto-name; rename later; path-derived visual nesting.
- A home for UI settings that future toggles can join.

**Non-Goals:**
- Free-floating in-page panels; tabbed dock groups; server-side layout storage; pickers on non-macOS (manual input stays); renaming a workspace's *directory* (re-register instead).

## Decisions

**D-popout — separate browser window per session, placeholder in the grid.**
A cell title-bar control (`⇱`) opens `window.open("/?solo=<id>", ...)`; App detects the `solo` query param and renders a single full-viewport `SessionTerminal` with a minimal header. While popped out, the main grid renders a placeholder cell ("viewing in separate window — reclaim") instead of a live terminal. Rationale: tmux sizes a session to its *smallest* attached client, so a small pop-out would shrink the main-grid view of the same session; exclusive viewing avoids the fight entirely (same acceptance P0 made for iTerm attaches, now enforced by the UI for its own windows). Cross-window bookkeeping via `localStorage` (`garage-popouts: {id: heartbeatTs}`) + `storage` events: the popout heartbeats every 5s and clears its entry on `beforeunload`/`pagehide`; the main window treats entries stale after 15s as closed (covers force-killed windows). Alternatives considered: keeping both attached (rejected — sizing fight); `BroadcastChannel` (equivalent; localStorage chosen because the heartbeat needs persistence across main-window reloads anyway).

**D-dock — dockview, per-workspace layout in localStorage, remount-on-move accepted.**
`dockview` (react wrapper) provides exactly the requested interaction: drag a cell, VS Code-style drop targets above/below/left/right, resizable splitters. Alternatives: `react-mosaic` (lighter but tiling-only, weaker drag affordances), hand-rolled flex splitters (weeks, not days). Panels are keyed by session id; moving a panel remounts the terminal → the WS reconnects and tmux repaints in well under a second — accepted for v1 (a portal-based DOM-preserving scheme is the escalation if the flicker ever matters). Layout serialization per workspace in `localStorage` (`garage-layout:<workspace>`), reconciled on every session-set change: new session → added as a bottom split of the largest panel; dead/popped-out session → panel removed; layout invalid/empty → default vertical stack. A `reset layout` control in the workspace header restores the default. The changes pane and rail stay outside the dock region (three-column shell unchanged); docking governs only the terminal grid.

**D-dim — column-level dimming via per-zone classes, never `opacity` on containers with exempt children.**
A `focusDim` setting gates a `focus-dim` class on `<main>`. The dimming *unit* is a column: workspace rail | terminal grid | changes pane. Only one column — the `activeColumn` — renders fully bright; the other two dim. Because CSS cannot un-dim a child inside a dimmed parent, the mechanism stays per-zone (each rail row, each grid cell, and the changes pane's content as one zone, all tagged `data-dim-zone`), but brightness is now driven by column membership: a component adds `.dim-focused` to *every* zone it owns when its column is active, not just to whichever row/cell happens to be "focused" within it. CSS: `.focus-dim [data-dim-zone]:not(.dim-exempt):not(.dim-focused) { opacity: .45; transition: opacity 120ms }`.

The needs-input exemption survives this rework unchanged in spirit: any rail row or grid cell whose session is `needs-input` gets `dim-exempt`, so it stays fully bright even inside a dimmed column — the amber dot is never dimmed, preserving the product's core attention signal regardless of which column the user is looking at. The changes pane has no such exemption (nothing inside it carries a needs-input signal); it's one all-or-nothing zone.

`activeColumn` is tracked by last interaction, not by which cell/row is nominally focused: a pointer `mousedown` inside a column's own wrapper (capture-phase, so it fires ahead of any inner `stopPropagation()`) sets that column active; keybindings that unambiguously target a column do the same as a side effect (`[`/`]`/`a`/`1`-`9` → grid; `Tab`/`j`/`k`, when the pane is visible → pane). Focus-target resolution (first match wins): help overlay open → it alone is bright, no column dimming applies (its backdrop already covers the page); review mode open → same; otherwise → the `activeColumn` as tracked above. The app header is never a zone — always full brightness regardless of `activeColumn`. Off by default.

**D-settings — localStorage, header gear, tiny store.**
`lib/settings.js`: defaults `{focusDim: false}`, read/write to `localStorage garage-settings`, subscribers via a `useSettings()` hook (storage-event aware so popout windows follow the main window's setting). A gear button beside "+ add workspace" opens a popover with toggles. Settings are per-browser by design — they're viewer preferences, not daemon truth; nothing crosses to `~/.garage`. Future settings (theme, decay timeout display, notification muting) join the same object.

**D-picker — daemon-side `osascript choose folder`; browser cannot do this.**
`POST /api/pick-directory` (Origin-allowlisted): on darwin runs `osascript -e 'POSIX path of (choose folder with prompt "Add workspace")'` → `{dir}` on choice, `{cancelled:true}` on user-cancel (osascript exit 1 with "User canceled"), 501 on other platforms. The add-workspace flow becomes: click `+ add workspace` → picker opens natively → on return, an inline row shows the derived name (basename → lowercase, non-`[a-z0-9]` runs collapsed to `-`, trimmed; collision with an existing workspace appends `-2`, `-3`, …) pre-filled and editable → confirm calls the existing `PUT /api/workspaces`. Manual path entry remains as a "type a path instead" fallback link (non-darwin, SSH-forwarded browsers, or picker failure). The dialog appears app-modal on the daemon's host — correct, since garage is same-machine by definition.

**D-rename — `PATCH /api/workspaces/:name` renames everything that embeds the name.**
Body `{name: <new>}` (NAME_RE-validated, 409 if taken). Steps, in order: (1) registry key moved; (2) for each live `garage/<old>/<label>` tmux session: `tmux rename-session` to `garage/<new>/<label>` — attached clients (grid ptys, iTerm) survive a rename untouched since they're attached to the session entity, not the name; (3) resume-metadata keys in the `sessions` map rewritten. The UI refetches; new `/term/:id` connections use the new ids. Failure mid-way (a rename-session errors) is reported with the completed subset — acceptable for a local single-user tool; no rollback machinery.

**D-nesting — derived, presentational, flat registry untouched.**
The rail builds a containment tree at render: workspace B nests under A iff `B.dir` is inside `A.dir` (`path` prefix with separator guard, deepest-parent wins for multi-level). Indented rendering, sessions still belong to their own workspace, spawn/status/diff semantics unchanged. `1–9` indexes the *flattened rendered order* (what you see is what you press). No cycles are possible (strict containment), and re-registering a directory elsewhere re-derives naturally. Alternative rejected: storing parent links in the registry — duplicates what the paths already say and can drift.

## Risks / Trade-offs

- [dockview bundle + API surface for a v1 need] → it's confined to `TerminalGrid`; react-mosaic remains the documented fallback if dockview fights us in practice.
- [Remount flicker when dragging a terminal] → sub-second, tmux repaints; portal-preservation documented as the escalation path.
- [Stale popout heartbeats after a crashed window] → 15s TTL reclaims the cell; worst case is a briefly-placeholder cell that self-heals.
- [`osascript` dialog can be missed behind windows] → `activate` the dialog via `tell application "System Events"`? Deferred; the prompt string and Dock bounce are usually enough. Cancel path handled explicitly.
- [Rename while a session is mid-restore or mid-attach] → rename only touches live sessions; a restorable session's meta key is rewritten too; the race window is one poll tick and self-corrects on refetch.
- [Dimming legibility] → 0.45 opacity chosen to keep dimmed text readable-but-recessive; tunable constant in one place.

## Open Questions

- Should the pop-out window auto-adopt focus-dim state? (Current answer: yes via storage-event sync — revisit if it feels wrong.)
- Default split direction when a session joins a customized layout (bottom of largest panel vs end of a row) — pick during implementation with real layouts.
- Whether `1–9` should skip nested children (top-level only) if flattened indexing feels unstable as workspaces nest — decide from dogfooding.
