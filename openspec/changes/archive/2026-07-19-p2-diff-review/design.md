# Design: p2-diff-review

## Context

P0 (`archive/2026-07-19-p0-terminal-bridge`) proved the tmux ⇄ node-pty ⇄ WebSocket ⇄ xterm.js pipeline; P1 (`archive/2026-07-19-p1-pit-wall`) built the workspace rail, multi-terminal grid, `StatusStore`/SSE push, the workspace registry (`~/.garage/state.json`, name→dir only), and the chrome-keybinding model (single window `keydown` listener, live only when DOM focus is outside a terminal's `.xterm` content). P2 is the review loop IDEA.md and the proposal describe: a glance-level changes pane beside the grid, a full-screen review mode, and a `code` escape hatch — so that reviewing a session's work never requires leaving the browser for iTerm or VS Code. Everything here is additive: no P1 capability changes shape, only consumes it (workspace registry for directory resolution, SSE `status` events for refetch triggers, the keybinding suppression rule for the new keys).

## Goals / Non-Goals

**Goals:**
- Define the exact, read-only git invocations behind `GET /api/diff/:workspace` — tracked and untracked changes, renames, binary handling, and a size budget — without ever mutating the repo (including its index).
- Choose a diff-rendering approach for the changes pane and review mode that fits the existing Tailwind-token dark-TUI aesthetic without a heavy new DOM/CSS dependency.
- Define refetch triggers (freshness) that reuse P1's existing signals (SSE `done` transitions) rather than adding filesystem watchers.
- Define where "viewed" checkmark state lives and how it invalidates when a file's diff changes.
- Define `POST /api/open-editor` and its path-traversal trust boundary, consistent with the registry-scoped resolution pattern `sessions.js` already uses.
- Extend the P1 keybinding model with the new keys (`Tab`, `j`/`k`, `r`/`Esc`, `v`, `o`) under the same suppression rule, resolving the one new nuance (Esc while review mode is open).
- Fit the changes pane and review mode into the existing grid layout without disrupting the P1 terminal grid's mount/unmount cost model.

**Non-Goals:**
- Inline comments back to Claude, side-by-side diff view, syntax highlighting (shiki) — all explicitly deferred polish, not part of this change.
- Filesystem watchers of any kind.
- Reboot restore, `npx` packaging, `garage deck` tmux-native layout (P3 / deferred per IDEA.md).
- Any change to session/status semantics — the diff pane is a read-only consumer of `StatusStore` transitions, not a new state source.
- Full git worktree/submodule support (see Risks).

## Decisions

**D-diff-cmd — `GET /api/diff/:workspace` git invocations.**

Resolution mirrors `sessions.js`'s existing pattern: `getWorkspace(workspace)` from the registry; unregistered → 404. No separate directory-existence check is added — see below for why.

1. `git -C <dir> rev-parse --is-inside-work-tree` first. A non-zero exit covers **both** "not a git repo" and "registered dir no longer exists on disk" with one branch — deliberately not special-cased, since both cases mean the same observable thing (nothing to diff): respond `200 {workspace, truncated: false, files: []}`.
2. `git -C <dir> status --porcelain=v2 -z` enumerates every changed path — ordinary changes (`1`), renames/copies (`2`, with similarity score and both paths), untracked (`?`). `-z` (NUL-separated) avoids filename-escaping ambiguity.
3. Tracked changes: `git -C <dir> diff -M --numstat HEAD -- .` for stats and `git -C <dir> diff -M --no-color HEAD -- .` for patches — **one process each**, not one per file; the daemon splits the single patch blob into per-file chunks on the `diff --git a/... b/...` header (cheap string split). Staged and unstaged are deliberately combined (`HEAD`, not `--cached` separately) — a glance pane needs "what changed since last commit," not git's staged/unstaged distinction; that finer view is exactly what the `o` escape hatch to VS Code's SCM panel is for. Fallback when the repo has no commits yet (`git rev-parse HEAD` fails): treat every status-reported tracked path the same as untracked (step 4) — there is no tree to diff against.
4. Untracked files (status `?` entries): `git diff --no-color --no-index -- /dev/null <path>` once per file. This is read-only and touches nothing — explicitly **not** `git add -N` (intent-to-add still writes an index entry, which is a mutation a read-only endpoint must never perform, however small). Bounded by the size budget (step 6): once the response budget is hit, remaining untracked files appear stat-only (`patch: null`, additions = line count via a cheap newline count, no further subprocess spawned).
5. Binary files: both diff invocations emit `Binary files a/... and b/... differ` (or `/dev/null and b/...` for untracked) identically — one detection path for both. On match: `{binary: true, patch: null}`, never base64 or otherwise dump binary bytes into the JSON response.
6. Size budget: cap each file's `patch` at ~30 KB (truncate, mark that file `truncated: true`); cap the total response at ~300 KB (stop attaching further patches once hit — remaining files still get their stat line, `patch: null`, `truncated: true`); top-level `truncated: true` if anything was capped. Both numbers are defaults to tune from real usage (see Open Questions), not load-bearing constants.
7. Rename detection: `-M` (git's default ~50% similarity) on both invocations; porcelain v2's `2` entries carry the rename pairing with a score, attached to the file entry as `renamedFrom` rather than re-derived from `diff --git` header text.
8. Response shape: `{workspace, truncated, files: [{path, status, binary, renamedFrom, additions, deletions, patch, truncated}]}`.

Alternatives considered: `git add -N` for untracked content — rejected outright per the requirement (it mutates the index, however reversible; a "read-only" guarantee shouldn't have an asterisk). Per-file `git diff` subprocesses for tracked files — rejected in favor of one process + client-side (daemon-side) splitting, fewer `execFile` calls for the common case of several changed tracked files in one commit's worth of work.

**D-diff-render — parse-diff + custom React, shiki deferred.**

Recommend `parse-diff` (small, ~a few KB, just structures a unified diff into hunks/lines) + hand-written React components styled with the existing `garage-*` Tailwind tokens (`garage-green`/`garage-red` for added/removed lines — both already defined in `ui/src/index.css`, `garage-dim`/`garage-faint` for context and line numbers, the mono font already loaded app-wide). Line coloring only, no syntax highlighting.

Rejected: **diff2html** — ships its own opinionated DOM structure and CSS, meaning re-skinning it to the dark garage aesthetic fights the library rather than using it, and it bundles its own diff parser (redundant next to `parse-diff`). **parse-diff + shiki** — shiki's syntax highlighting is real value eventually, but its WASM grammar/theme assets are a meaningfully heavier bundle than a glance-level diff pane justifies for a tool whose stated review model is "glance beside the terminal, deep-review in VS Code via `o`." Since VS Code already provides full syntax-highlighted diffing one keystroke away, shipping shiki now duplicates that value at real bundle-size cost for marginal benefit in the glance/review-mode use case. Explicitly deferred as later polish (revisit once P2 ships and someone actually misses the highlighting).

The full-width review mode reuses the identical file-diff renderer component — only the surrounding layout (full-screen overlay + file rail vs. inline pane) differs; no second rendering path.

**D-freshness — refetch on `done` transition, manual refresh, and review-mode entry; no fs-watchers.**

Three triggers, no others:
1. SSE `status` event where the transitioning session belongs to the currently focused workspace and `to === "done"` — the changes pane subscribes the same way `App.jsx` already does for the rail (`es.addEventListener("status", ...)`), filters to focused-workspace sessions, and calls the diff fetch.
2. A manual refresh control in the changes-pane header (covers a session mid-`working` where the user wants to peek early, or a `done`→`idle` decay window they missed).
3. Entering review mode (`r`) always refetches, unconditionally — guarantees review mode never shows a diff staler than "right now," even if an SSE event was missed during a reconnect gap (P1's D-push already documents `EventSource` reconnects resync `/api/sessions`, not the diff; review mode entry is the diff-side equivalent).

No filesystem watchers: a chokidar-style watch per registered workspace directory adds a persistent OS-level resource per workspace, plus noise from editor autosave/temp-file-swap churn that has nothing to do with "did Claude finish a turn" — the actual moment worth refetching for. The Stop-hook-driven `done` transition (P1's `session-status` capability, already built) *is* that moment; adding a second, noisier signal for the same event is the kind of extra state the thin-daemon philosophy exists to avoid.

**D-viewed — client-side localStorage, keyed by workspace + file path + content hash.**

Key: `garage:viewed:<workspace>:<path>`; value: a hash of that file's diff patch text (a fast non-cryptographic string hash, e.g. djb2 over `patch`, computed client-side — collision risk is irrelevant here, this is a UI convenience, not a security boundary). On render, compare the current fetch's hash for that file to the stored value: match → checkmark shown; mismatch or absent → unviewed. Marking viewed (`v`) writes the current hash. Wipe: after each diff fetch, reconcile stored keys for that workspace against the new file list — delete any key whose path is no longer present (the file stopped changing, e.g. reverted or already committed elsewhere).

Not in the daemon (`~/.garage`): viewed-state is ephemeral per-user UI progress, not source-of-truth data — it doesn't need to survive a daemon restart independent of the browser, and a single-user local tool has no multi-client sync requirement to justify a server-side store. This matches the registry's own scope discipline (`registry.js` stores directory mappings only, never opinions about UX state) and P1's "daemon is almost stateless" thread. Revisit if reviewing the same workspace from two browsers/machines becomes a real workflow — at that point a small daemon-side store analogous to `registry.js` (keyed by workspace, atomic write) would be the natural extension.

**D-editor — `POST /api/open-editor`, registry-scoped path resolution.**

Body `{workspace, file?, line?}`. Resolution: `getWorkspace(workspace)` → 404 if unregistered (same pattern as `sessions.js`). If `file` is given: `const abs = path.resolve(registered.dir, file)`; reject with 400 unless `abs === registered.dir || abs.startsWith(registered.dir + path.sep)` — a prefix check on the *resolved* path, not the raw string, so `../../etc/passwd`-style traversal (and absolute-path overrides of `file`) are caught regardless of how they're spelled. Command: `code --goto <abs>:<line ?? 1>` with a file, else `code <registered.dir>`. `GARAGE_EDITOR_CMD` env var overrides the `code` binary name, mirroring `GARAGE_CLAUDE_CMD` in `sessions.js`. If the spawn fails with `ENOENT` (editor CLI missing), respond 501 `{error: "..."}` with a message naming the missing command and the override env var; the UI surfaces that string directly (banner/toast) rather than failing silently. Behind the existing Origin allowlist — no `EXEMPT_PATHS` entry (unlike `/api/hooks/claude`, this is browser-called, not CLI-called).

**D-keys — extend the P1 chrome-keybinding model.**

New bindings, all under the existing suppression rule (dead while `document.activeElement.closest(".xterm")` is truthy): `Tab` toggles file-list ⇄ diff emphasis within the changes pane; `j`/`k` step the current file, working in both the pane and review mode (distinct scope from P1's `[`/`]`, which cycles *terminals* — no key collision, no semantic overlap); `r` enters review mode; `v` marks the current file viewed and auto-advances to the next unviewed file, review-mode only; `o` opens the current file (`POST /api/open-editor`), available in both the pane and review mode.

`Esc` exits review mode, with the one nuance worth stating explicitly: terminals must swallow their own `Esc` (Claude Code's TUI uses it, e.g. to interrupt) — correct, unchanged behavior — so review-mode `Esc` only fires under the same "no terminal has DOM focus" condition as everything else. This is safe by construction, not by luck: review mode is a `position: fixed` overlay covering the whole viewport (D-layout), so its overlay intercepts all pointer events — a user cannot click into a hidden terminal to give it focus while review mode is open. Entering review mode additionally calls the existing `blurActiveTerminal()` helper (already used by header/rail click-through in `App.jsx`) so any terminal focus held from *before* entry is cleared immediately, guaranteeing `Esc` works on the very next keypress rather than requiring an extra click first.

No reserve conflicts: none of `Tab`/`j`/`k`/`r`/`Esc`/`v`/`o` collide with P1's `1`–`9`/`[`/`]`/`a`, and all are inert while typing into a terminal for the same reason P1's bindings already are.

**D-layout — changes pane as a third grid column; review mode as a non-destructive overlay.**

The P1 layout is a CSS grid (`grid-cols-[240px_1fr]` in `App.jsx`, not flex, despite the proposal's shorthand) — P2 extends it to `grid-cols-[240px_1fr_360px]`, a fixed pixel width consistent with the existing 240px rail (a `30%` proportional column would fight the two already-fixed columns at typical viewport widths; 360px approximates ~30% at a common ~1200px width without the layout math). Collapsible: a header toggle collapses the third column to a narrow strip (e.g. `32px`, matching the rail's own toggle-affordance pattern for `AddWorkspaceForm`); collapse state is plain `useState`, not persisted to localStorage — unlike D-viewed, this is disposable per-session UI state with no invalidation concern, so persisting it is unjustified complexity until proven annoying.

Review mode is `position: fixed; inset: 0` over everything, and deliberately **unmounts nothing underneath**. Terminals stay mounted and streaming because they already are, at zero incremental cost: P1's D-grid caps concurrent ptys to the focused workspace's sessions (design center ~6) regardless of review mode, so hiding them behind the overlay via CSS costs nothing new. The alternative — unmounting the grid on review-mode entry — would tear down each `SessionTerminal`'s WebSocket (per `term.js`, closing the socket kills its `tmux attach` pty) and force a fresh reattach on exit: a visible reconnect flash, a brief gap in live rendering, and work `TerminalGrid.jsx`'s existing key-based unmount logic already does for *workspace switches* (where it's correct — a different workspace's ptys should die) but not for a same-workspace view toggle, which is what review mode is. Keeping everything mounted matches the mental model: review mode is a view change, not a workspace change.

## Risks / Trade-offs

- [Huge diffs blow past reasonable render/response size] → per-file (~30 KB) and total-response (~300 KB) budgets (D-diff-cmd step 6); UI shows a "diff truncated" banner with the `o` open-in-VS-Code action right there as the natural next step.
- [Untracked binary files] → same binary-detection path as tracked binaries (`Binary files ... differ` match on both `git diff HEAD` and `git diff --no-index` output) — marked, never dumped as content, regardless of tracked/untracked origin.
- [git worktrees / submodules] → explicitly unsupported and untested. Worktrees: git's commands remain worktree-aware so basic diffing still runs correctly against that worktree's own HEAD, but no special handling exists for worktree-specific states (locked, prunable, administrative files) — not exercised, not guaranteed. Submodules: a changed submodule reference shows as an opaque gitlink/mode change in `git diff`, not a real content diff — P2 does not special-case this, it will render as whatever git's own diff output produces for that line (likely a one-line mode-change entry, not a truncation bug, but not a meaningful review either). Both are called out as known gaps, not silent failure modes.
- [`code` CLI absent] → `POST /api/open-editor` responds 501 with a message naming the missing command and the `GARAGE_EDITOR_CMD` override; the UI surfaces that message rather than failing silently (D-editor).
- [Esc-key overloading between terminal-native Esc and review-mode exit] → resolved structurally, not just by convention: review mode's full-screen overlay intercepts all pointer events (no way to click into a hidden terminal while it's open), and entry explicitly blurs any terminal focus held from before — so the existing "no terminal has DOM focus" suppression rule is sufficient and Esc is available immediately (D-keys).
- [Path traversal via `file` in `POST /api/open-editor`] → resolved path (`path.resolve`, not string concatenation) checked against the registered directory as a prefix, not the raw input string, so `..`-based and absolute-path traversal attempts are both caught (D-editor).

## Open Questions

- Exact size-budget numbers (~30 KB/file, ~300 KB/response) are proposed defaults — tune from real usage rather than fixing now, same posture P1 took with its `done`→`idle` decay timeout and poller interval.
- Changes-pane collapsed width/behavior is unpersisted for now; revisit if users find re-expanding it every reload annoying.
- Whether `git diff -M` rename detection's default ~50% threshold ever needs to be configurable — no evidence either way yet; ship the default.
