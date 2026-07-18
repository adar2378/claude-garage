# Proposal: p2-diff-review

## Why

The pit wall (P1) tells you *which* session needs you; P2 tells you *what it did*. GAP.md's second verified gap: no tmux-native tool has any diff review (agent-deck: none; claude-squad: one scrollable blob), and the web tools that have it own your sessions. Reviewing a session's changes today means leaving the garage for iTerm/VS Code — exactly the context-switching this tool exists to kill. P2 completes the review loop: glance at changes beside the terminal, deep-review full-screen, and jump to VS Code only when you choose to edit.

## What Changes

- **Diff API**: `GET /api/diff/:workspace` — `git status` + unified `git diff` (including untracked files) for the workspace's registered directory. Read-only; never mutates the repo. Non-git directories return an empty diff, not an error.
- **Changes pane (right)**: glance-level view for the focused workspace — changed-file list with per-file +/− stats and a scrollable unified diff. `Tab` toggles file-list ⇄ diff emphasis; `j`/`k` step through changed files.
- **Diff freshness**: the pane refetches when a session in the focused workspace transitions to `done` (SSE already delivers this) plus a manual refresh control. No filesystem watching in P2.
- **Review mode** (`r` to enter, `Esc` to exit): full-screen review — left file rail with GitHub-PR-style viewed checkmarks (`v` marks viewed + auto-advances to next unviewed), continuous full-width diff, `j`/`k` between files. Viewed state is per-workspace, client-side, and resets when the underlying diff for that file changes.
- **VS Code escape hatch**: `POST /api/open-editor` — `{workspace}` opens the project root (`code <dir>`); `{workspace, file, line}` opens a file at a line (`code --goto <file>:<line>`). UI: per-workspace open button and `o` on the current file in review mode.
- **Keybindings added**: `Tab`, `j`/`k`, `r`/`Esc`, `v`, `o` — same chrome-navigation rules as P1 (suppressed while a terminal has DOM focus).

Out of scope: inline comments back to Claude (desktop-app territory, not our moat), side-by-side diff (later polish), filesystem watchers, reboot restore + packaging (P3).

## Capabilities

### New Capabilities
- `workspace-diff`: the read-only diff API — what it reports (tracked changes, untracked files, per-file stats), non-git behavior, and freshness contract.
- `diff-review-ui`: changes pane, review mode with viewed-tracking, and the review keybindings.
- `editor-escape`: the open-in-VS-Code endpoint and its UI affordances; the trust boundary (only registered workspace dirs / files inside them can be opened).

### Modified Capabilities

(none — P1 capabilities are consumed, not changed: SSE `done` events trigger diff refetch, pit-wall keybinding rules extend to the new keys without changing existing ones)

## Impact

- Daemon: diff module shelling to `git` (`-C <dir>`, read-only flags), open-editor module shelling to `code`; both behind the existing Origin allowlist. Path validation so open-editor can never open files outside a registered workspace.
- UI: right pane joins the P1 layout (rail | grid | changes); full-screen review mode overlays everything; diff parsing/rendering dependency (design decides: parse-diff + highlighting vs plain unified rendering with token colors).
- No new session/status semantics; no schema changes to existing capabilities.
- Risk surface: `code` CLI may be absent → endpoint 501s with a clear message; large diffs need render capping (design sets budget).
