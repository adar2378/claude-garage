# Verification: p17-tui-only

## Phase A (port to the TUI), 2026-10-03

**Unit:** `npm test` 129/129 pass. `cd wall && cargo test --lib --bins` 507 pass. `cargo clippy --all-targets` clean.

**Live e2e:** `wall/test/e2e/run_p17.sh` against the release binary (`npm run build:tui`). All 46 checks pass.
- Fully scratch: daemon on :4797, scratch `GARAGE_DIR` and `GARAGE_CLAUDE_HOME`, zsh in place of claude, scratch git repo. The user's daemon (:4747), tmux sessions (7 before and after) and `~/.claude/settings.json` were untouched.
- Every `x x` was gated on the notice naming the exact test session.

| Case | Result |
|---|---|
| Plain session `x x` | closed, no overlay |
| Worktree `x x` → `m` | overlay "garage/wtm → main"; merged, branch deleted, worktree removed, notice "merged garage/wtm into main" |
| `d` then `d` | first `d` only arms ("press d again…", branch kept); second discards |
| Dirty worktree → `m` | "uncommitted changes" inline, overlay stays; `k` keeps branch and dir |
| Merge conflict → `m` | "merge conflict — merge aborted…" inline; no `MERGE_HEAD`, repo clean; `Esc` keeps branch |
| `I` twice | "hooks installed · statusline feed installed…", then "hooks already installed · …"; scratch settings.json has the hook |

**Fixes made during review:** merge request timeout raised to 120 s; conflict message no longer tells the user to "fix conflicts and then commit" after the abort; word-wrapping for overlay errors; "(or Esc)" on the keep line.
