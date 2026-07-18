# P2 e2e verification — 2026-07-19

Environment: same as P1. Sandbox workspace made a real git repo with the exact IDEA.md gate diff: 2 modified (`util.py`, `config.json`), 1 untracked (`mathx.py`), 1 deleted (`notes.txt`).

## 3.1 The IDEA.md P2 gate — review a real 4-file diff, zero iTerm

- Changes pane rendered all 4 files with correct glyphs (M/M/?/D), stats (+3/−2, +4/−3, +2/−0, +0/−1) and parse-diff hunks with line numbers.
- `r` → full-screen review mode: file rail, `0/4 viewed`, continuous diff, keybinding legend.
- `v` ×4 → each file marked viewed with auto-advance → **4/4 viewed**, all ✓. Screenshot: `p2-review-mode-4of4.png`.
- Entire flow in the browser; iTerm never opened. ✅

## Viewed-state persistence + invalidation

- Full page reload → re-entered review → still **4/4 viewed** (localStorage) ✅
- `util.py` modified on disk → after refetch, **3/4 viewed** with only util.py reset to unviewed (content-hash invalidation); other three kept ✓ ✅

## 3.2 Freshness via Stop hook

beta (respawned with tokened hooks) given a trivial prompt → Stop hook → `done` → SSE event → **changes pane refetched itself**: util.py stats updated `+4/−3` → `+7/−3` with zero manual action ✅ (also re-proves the token-authenticated hook path with a real session).

## 3.3 Editor escape hatch

`o` on util.py in review mode → daemon `POST /api/open-editor` 200 → VS Code process launched, file opened at first-hunk line ✅. (Daemon agent had separately verified: traversal `../../..` and absolute-path escapes → 400; unknown workspace → 404; read-only diff proven via identical `git status` before/after repeated calls.)

## 3.4 Keybinding coexistence

With DOM focus inside a terminal: `r` did NOT open review mode, `Esc` did NOT act on chrome — both keys passed through to the pty (typed `r` visible then cleared by Esc in claude's input); terminal retained focus throughout ✅. P1 suppression rule holds for all new keys.

## Verdict

**P2 gate: PASS.** The review loop is closed: glance beside the terminal, deep-review full-screen with viewed-tracking that survives reloads and invalidates on change, freshness driven by the session's own Stop events, and a one-keystroke VS Code escape hatch.
