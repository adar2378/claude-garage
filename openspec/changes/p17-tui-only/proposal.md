## Why

claude-garage ships two surfaces, but the Rust TUI is now the product (README, showreel, hero). The web wall doubles the surface to maintain and confuses the pitch. Two features still exist only in the browser, hooks install and the worktree finish flow, so the TUI points users at a web wall we want to delete.

## What Changes

**Phase A: port to the TUI (repo stays shippable after this phase)**
- `I` installs both halves of the integration: the statusline (as today) and Claude Code hooks (`POST /api/hooks/install`). One strip notice summarizes both, including "already installed". No new key; no hooks-installed detector (no endpoint exists for it).
- Closing a worktree session (`x x`, live or restorable) opens a finish overlay holding the `{path, branch, repoDir}` record: `m` merge into the branch checked out in `repoDir` (shown in the prompt), `d d` discard, `k`/`Esc` keep. A 409 (dirty worktree, merge conflict) shows inline; the overlay stays open for retry or keep.
- Daemon fix: a failed merge runs `git merge --abort` so `repoDir` is never left mid-merge.

**Phase B: remove the web wall**
- **BREAKING:** `npx claude-garage` with no subcommand runs the TUI. `tui` stays as an alias. Released as 0.5.0 with a CHANGELOG entry.
- Delete `ui/` (React app, tests), the `ui` workspace, `ui/dist` in `files`, `prepack`, `ui/test` in `npm test`, the `dev` script and `concurrently`.
- Delete the WebSocket terminal bridge (`daemon/src/term.js`, `ws`, `node-pty`, the `postinstall` chmod), static serving (`@fastify/static`, `GARAGE_SERVE_UI` in the daemon and the launcher), and the `:5173` Vite origins in `security.js`.
- Notifications: drop `-open <web URL>` from `terminal-notifier`. A plain notification remains.
- Remove TUI strings and comments that point at the web wall.
- **Kept on purpose:** `diff.js`, `editor.js`, `/api/hooks/snippet`, `/api/statusline/snippet`, `PATCH /api/workspaces/:name`. They are the daemon half of "diff review in the TUI" on the roadmap, or harmless manual paths.

## Capabilities

### New Capabilities
- `tui-worktree-finish`: the TUI's merge / discard / keep overlay shown after closing a worktree session.
- `tui-hooks-install`: `I` installs hooks alongside the statusline, with one combined notice.

### Modified Capabilities
- `hooks-install`: the browser banner requirement is removed; install is driven from the TUI.
- `worktree-sessions`: a failed merge aborts it, leaving `repoDir` clean.
- `packaging`: the no-subcommand entrypoint runs the TUI; the daemon no longer serves a UI or `/term` WebSockets; the "UI served from daemon's own origin" requirement is removed.
- `session-status`: macOS notifications no longer open the web wall on click.

### Removed Capabilities
Web-only specs, removed in full: `attention-badge`, `connection-resilience`, `diff-review-ui`, `grid-controls`, `grid-views`, `input-mode-indicator`, `pit-pet`, `pit-wall-ui`, `theming`.

Partly removed (browser requirements only): `terminal-bridge` (keeps "session survives client disconnect" and the `tmux attach` escape hatch), `editor-escape` (keeps the daemon endpoint). `workspace-diff` stays untouched (daemon diff API is kept).

## Impact

- **Code:** `wall/src` (runtime, store, new overlay module, client), `daemon/src` (`worktrees.js`, `index.js`, `security.js`, `notify.js`, delete `term.js`), `bin/garage.js`, `package.json`, `package-lock.json`, delete `ui/`.
- **Dependencies removed:** `@fastify/static`, `node-pty`, `ws`, `concurrently`.
- **Users:** anyone opening `http://127.0.0.1:4747` loses the browser wall. Diff review is unavailable until the TUI port on the roadmap.
- **Docs:** README `I` row, CHANGELOG, `RELEASE-CHECKLIST.md`, `lefthook.yml` (untracked, references `prepack`).
- **Verification:** worktree merge/discard and `x x` on live sessions touch real git and tmux, so they run in the main session only, with consent.
