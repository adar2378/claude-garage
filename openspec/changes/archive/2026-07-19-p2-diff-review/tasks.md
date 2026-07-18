# Tasks: p2-diff-review

Groups 1 (daemon) and 2 (UI) are parallelizable; group 3 integrates and verifies e2e.

## 1. Diff + editor APIs (daemon)

- [x] 1.1 `GET /api/diff/:workspace` — porcelain-v2 enumeration + single `git diff` split by file, untracked via `--no-index /dev/null`, rename + binary handling, per-file stats, size budgets with `truncated` flags; read-only (never mutates index); 404 unknown workspace; 200 empty for non-git dirs
- [x] 1.2 `POST /api/open-editor` — `{workspace, file?, line?}`; `path.resolve` + prefix traversal guard (400); 404 unknown workspace; `code` / `$GARAGE_EDITOR_CMD` spawn; 501 when editor CLI missing; behind Origin allowlist
- [x] 1.3 Gate: curl against the real repo — diff lists a modified + an untracked file with stats and hunks; traversal attempt (`../../etc/hosts`) → 400; non-git workspace → 200 `[]`; index untouched after repeated calls (`git status` unchanged)

## 2. Changes pane + review mode (UI)

- [x] 2.1 Changes pane (third column, 360px, collapsible): changed-file list with +/− stats, scrollable unified diff rendered via parse-diff + Tailwind tokens; `Tab` toggles list⇄diff emphasis; `j`/`k` step files; refetch on SSE `done` for focused workspace + manual refresh
- [x] 2.2 Review mode: `r` opens full-screen overlay (terminals stay mounted underneath) — file rail with viewed checkmarks, continuous full-width diff, `j`/`k` navigate, `v` marks viewed + auto-advances to next unviewed, `Esc` exits; entering blurs any focused terminal and refetches
- [x] 2.3 Viewed-state: localStorage keyed workspace+path+content-hash; reset when a file's diff changes; wiped when file leaves the diff
- [x] 2.4 Editor affordances: per-workspace open-root control; `o` opens current file (`--goto` with line of first hunk) from pane and review mode; surface 501 inline
- [x] 2.5 Gate: `npm run build -w ui` clean; with a real dirty repo — pane shows files+stats+diff, review mode navigates and tracks viewed across reload

## 3. Integration + e2e verification (the P2 phase gate)

- [x] 3.1 Full IDEA.md P2 gate (Playwright): create a real ≥4-file diff in a workspace (modify 2, add 1 untracked, delete 1); review it start-to-finish in review mode — j/k through all files, v-mark each viewed with auto-advance, zero iTerm usage
- [x] 3.2 Freshness: a session's Stop (done) in the focused workspace refreshes the pane without manual action
- [x] 3.3 `o` opens VS Code at the right file (verify `code` spawn; assert daemon side + VS Code window observed or spawn logged)
- [x] 3.4 Keybinding coexistence: j/k/r/v/Tab suppressed while typing into a terminal; Esc in a focused terminal goes to Claude, not review-exit
- [x] 3.5 Record in `verification.md`; only then is P2 done
