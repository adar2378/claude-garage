# Spike: ratatui wall — findings (2026-08-31)

**Verdict: GO on ratatui + tui-term + vt100 — published crates only, no
vendoring, no patches.** All three gates pass with wide margins on the first
crate choice (plan a); wezterm-term (plan b) was never needed and ghostty-vt
FFI stays an unused escalation path. The input pipeline that cost nocterm
four fixes (three vendored patches + one open issue) worked here out of the
box; the one genuinely new finding is on the scrollback/teardown side, both
solved in-spike (see gate 3 and the teardown note).

Toolchain: cargo 1.94.1. Deps (pinned in `Cargo.toml`): ratatui 0.30.2,
crossterm 0.29.0, portable-pty 0.9.0, vt100 0.16.2, tui-term 0.3.4.
Binary: 1.1 MB release build.

Run it yourself: `cargo build --release`, then `./checks/run_checks.sh`
(fully self-contained: creates scratch `rspike-1..9` sessions, runs every
gate, kills only `rspike-*` on exit — 37 checks, all PASS). Interactive:
create sessions, run `target/release/ratatui-wall` in a ≥200-col terminal.
Keys: `1-9` focus · `Enter` engage · `Ctrl+G` back · `Shift+PageUp/Down`
frozen scrollback · `q` quit.

## Gate 1 — performance: PASS

3×3 grid, 5 tiles scrolling colored output at ~40 lines/s + 2 spinner tiles
+ keyecho + shell (same `tools/` scripts as the nocterm spike). Latency is
keypress→handled via the same `GARAGE_TUI_KEYLOG` epoch-µs contract as the
Dart TUI; 5 samples per size.

| host size | latency samples (ms) | median | %CPU (3 samples) | RSS |
|---|---|---|---|---|
| 200×55 | 5, 6, 6, 6, 6 | **6 ms** | 1.7 / 1.9 / 1.7 | 5.9 MB |
| 250×70 | 5, 6, 6, 6, 6 | **6 ms** | 2.7 / 2.6 / 2.4 | — |

Target was <50 ms; measured 6 ms with zero tuning (nocterm needed the fps15
cap to get from >1100 ms down to 8–9 ms, at ~13% CPU and ~70 MB RSS).
Architecture credit: PTY parsing runs on per-tile reader threads behind a
mutex, input on its own thread, render capped at 30 fps on the main thread —
stdin never starves behind render/PTY work, which was nocterm's core problem.

**Resize done right from the start:** tile PTYs are opened via portable-pty
and resized to their tile's inner rect on startup and every `Resize` event
(`MasterPty::resize` → TIOCSWINSZ), with the vt100 parser resized under the
same code path. Verified from the tmux side (`list-clients`):
attach client = 65×16 at 200×55 host, 81×21 at 250×70 (matches tile inner).

## Gate 2 — verbatim passthrough: PASS (own encoder, zero framework patches)

Engaged keyecho tile received every sequence byte-exact: `h`, `^A` (0x01),
`^C` (0x03, wall survives — raw mode clears ISIG), `^[[1;3C` (Alt+Right —
modifier survives), `^[f` (Alt+f), `^[[Z` (Shift+Tab), `^[[D`. `Ctrl+G`
disengages and is consumed (no `^G` ever reaches the pane).

**The nocterm three-patch failure class does not exist here.** A single
`tmux send-keys -H 68 1b 5b 5a 0d 69` (h + Shift+Tab + Enter + i in ONE
stdin read) arrived as four individual correct events (`h^[[Z^Mi` in the
pane). crossterm's parser handles coalesced reads natively: no synthetic
paste conversion (nocterm patch 3), no exact-length CSI matching bug
(nocterm's Shift+Tab patch), no clipboard corruption. Also:

- **Own encoder still required, and sufficient.** Like nocterm, the
  framework doesn't re-encode events to bytes; unlike nocterm, the parsed
  events carry full modifiers reliably. `encode_key` (~90 lines in
  `src/main.rs`) covers the spec's verbatim-re-encoding list: printables,
  Ctrl+letter/@[\]^_?, Alt-ESC prefix, `CSI 1;<mod><final>` arrows/Home/End,
  `CSI <n>;<mod>~` Ins/Del/PgUp/PgDn, Enter/Tab/BackTab/Backspace/Esc, F1–F12.
- **IXON/ISIG:** crossterm raw mode clears both (cfmakeraw), no external
  `stty -ixon` bootstrap needed (nocterm patch 2 obsolete). Verified by the
  post-quit `stty -a` check: no `-ixon/-isig/-icanon/-echo` left behind.
- **Ctrl+G chord collision: none.** No debug-key intercept anywhere in the
  stack, so the UX mockup's Ctrl+G works as the disengage chord with no
  vendored change (nocterm needed Ctrl+Q + a planned patch).
- **Paste:** bracketed paste enabled and forwarded wrapped in
  `ESC[200~…201~` via crossterm's first-class `Event::Paste` (implemented,
  not gate-tested; no synthetic-paste recovery layer needed).
- Not retested here: a live Claude Code composer inside a tile (nocterm's
  gate-2 proof). Byte-exactness at the PTY covers the contract; do the
  composer smoke test early in the port.

## Gate 3 — frozen scrollback: PASS (capture-pane seeding, absolute anchor)

Engaged streaming tile, Shift+PageUp ×2: tile interior byte-identical across
a 2 s double-capture while a neighboring stress tile visibly advanced.
`#{pane_in_mode}` stayed 0 throughout (tmux copy-mode never touched).
Shift+PageDown ×2 returns to live (stream visibly resumes); typing while
frozen snaps to live and then passes the key through.

**Finding (this spike's most important non-obvious result):** a tmux attach
client lives on the alternate screen and is repainted in place — the tile's
vt100 emulator NEVER accumulates scrollback, so any emulator-buffer-based
scrollback plan (nocterm's gate-3 sketch included) silently has nothing to
scroll. The p8 design's "capture-pane seeding" is not an enhancement, it is
the only mechanism. Implemented as: freeze = one `tmux capture-pane -p -e`
snapshot anchored at an ABSOLUTE tmux history index (`#{history_size}`
coordinates), rendered through a throwaway vt100 parser so colors survive.
Live output grows history but cannot drag the view — the anchor is absolute
by construction, no per-frame re-pinning. PageUp pages the anchor up
(re-capture); PageDown decrements a page counter and returns to live at 0
(on a streaming tile the live edge runs away from any absolute anchor, so
"as many pages down as up" is the correct return path). Caveat for the port:
if tmux `history-limit` trims while frozen, deeper PageUps can land shifted
by the trimmed amount; the frozen snapshot itself can never drift.

(Side note recorded for the port: vt100 0.16 does auto-anchor its own
scrollback absolutely — `Grid::scroll_up` increments `scrollback_offset`
when scrolled back, so the nocterm "offset relative to buffer end" gap
doesn't exist in the emulator either — it's just unreachable for attach
clients because the buffer never fills.)

## VT crate decision: tui-term 0.3.4 + vt100 0.16.2 (plan a) — chosen

Passed all gates cleanly; plan b (wezterm-term + custom widget) not needed.
ghostty-vt FFI: NOT built, remains the escalation path only.

Turborepo perf-patch status (vercel/turborepo#9123) against current crates,
verified in source:

- `Cell::contents()` per-cell String allocation: **upstreamed** — vt100
  0.16.2 returns `&str` (`src/cell.rs:89`), zero allocation per cell.
- `visible_rows()` scrollback-offset bounds bugs: **upstreamed** — 0.16.2
  carries the skip/take clamping fixes (comments in `src/grid.rs:126`).
- `visible_row(n)` O(rows) iterator `.nth()` indexing: **not fixed** —
  tui-term's render path calls `Screen::cell(row, col)` per cell, each an
  O(row) walk ⇒ O(rows²·cols) per tile per frame. At wall tile sizes this is
  noise (the 1.7–2.7% CPU numbers above INCLUDE it; a maximized 250×70 tile
  is ~600k iterator steps/frame, still trivial). Flagged as the first
  optimization candidate (cache rows per frame, or a custom widget over
  `Screen::rows_formatted`) — not a blocker.

tui-term 0.3.4's `Screen` trait handles scrollback in visible coordinates
and exposes per-tile `Cursor` visibility; it sits on ratatui-core 0.1 /
ratatui-widgets 0.3 = ratatui 0.30.x. All current, all on crates.io.

## Nocterm-lessons checklist

| nocterm bug class | ratatui stack |
|---|---|
| coalesced stdin → synthetic paste, Enter swallowed (patch 3) | not present — crossterm splits coalesced reads into real events (verified with multi-key `send-keys -H`) |
| 3-byte CSI exact-length match drops Shift+Tab under load (patch 4) | not present — `^[[Z` byte-exact inside a coalesced read |
| framework key translation drops modifiers (patch 1: own encoder) | same medicine, no surgery: own encoder over crossterm events; modifiers verified surviving (`^[[1;3C`) |
| IXON eats Ctrl+Q/Ctrl+S (patch 2: external stty) | not present — raw mode clears IXON, restore verified via `stty -a` |
| Ctrl+G debug-toggle collision | not present — Ctrl+G is the disengage chord, natively |
| `TMUX` env leaks into tile PTYs (nested-client refusal) | handled (`env_remove("TMUX")`, TERM=xterm-256color) |
| tiles must track host resize (TIOCSWINSZ) | done from the start via portable-pty resize; verified via `tmux list-clients` at both sizes |
| cursor rendering | tui-term `Cursor::visibility` — only the engaged live tile shows a cursor |
| scrollback anchoring | capture-pane seeding with absolute tmux-history anchor (see gate 3 — emulator buffers are empty for attach clients) |
| alt-screen behavior of attach clients | vt100 handles the alt screen; it's also WHY capture-pane seeding is mandatory |
| startup input coalescing | not observed (first keys handled in ~6 ms) |
| tile survives inner-session death | not exercised this spike — port needs the watch+respawn loop either way |

**New bug class found here (not in the nocterm list): teardown injection.**
Killing attach clients via SIGHUP (portable-pty's `ChildKiller::kill`!) or
letting them die on master-close EOF can inject `\n`/`^D` into the ATTACHED
PANE; an idle interactive shell reads ^D as EOF and exits 0 — taking its
tmux session with it (observed repeatedly against a default-shell session;
byte-level proof via a `cat -v` recorder pane). Fix, verified across full
harness runs: `tmux detach-client -s <session>` FIRST, then close masters —
zero bytes injected, all sessions survive quit AND SIGKILL of the wall.
Port rules: never signal attach clients; detach gracefully on every exit
path; note `-s` detaches all of that session's clients, so a port where the
user may co-attach should target the client by tty instead.

## Port-effort read (p8 components)

Translates ~1:1:
- **Key-routing spec** (three layers, chip, engage/disengage, garage
  bindings): the layer state machine is this spike's `run()` loop shape;
  the tui-key-routing spec's scenarios map directly. Ctrl+G is free.
- **Encoder table**: port `encode_key` from the Dart encoder nearly
  line-for-line; crossterm events carry the same info the Dart parser did.
- **E2e harness**: `tui/test/e2e/run_e2e.sh` works nearly unchanged — same
  send-keys/capture-pane/keylog(µs) contract; this spike's
  `checks/run_checks.sh` is the proof (37 checks in that style).
- **Grid/rail/strip rendering**: ratatui `Layout` + tui-term widget; the
  strip chip is a one-line `Paragraph`.
- **PTY lifecycle**: portable-pty spawn/resize/reader-thread; same
  TMUX-strip and TERM rules.

Needs rework (not 1:1):
- **Scrollback seam**: nocterm's plan ("own render path over the emulator's
  10k buffer") is void for attach clients — build the capture-pane-seeded
  frozen view (this spike's `FrozenView` is the skeleton; add styling-exact
  `-e` handling, wrapped-line care, history-limit trim awareness).
- **Teardown/lifecycle**: detach-client discipline (above), plus the
  inner-session death watch + reattach from the nocterm findings.
- **Daemon client** (`/api/sessions`, hooks): straightforward but new code
  in Rust (std TCP or a small HTTP crate) — the Dart side doesn't port.
- **All four vendored nocterm patches + fork maintenance: deleted.** No
  vendor directory at all. That is the headline cost difference.

## Files

- `src/main.rs` — spike app (grid, 2-layer key routing, encoder,
  capture-pane frozen view, detach-client teardown, keylog)
- `Cargo.toml` — pinned published deps (no git, no vendor)
- `checks/run_checks.sh` — 37-check gate harness (run_e2e.sh style; scratch
  `rspike-*` only; safe cleanup trap)
- `tools/stress.sh` / `spinner.sh` / `keyecho.sh` — copied verbatim from
  `spikes/nocterm-wall/tools/`

Session setup used (the harness does this itself):

```bash
for i in 1 2 3 4 5; do tmux new-session -d -s rspike-$i tools/stress.sh; done
for i in 6 7;       do tmux new-session -d -s rspike-$i tools/spinner.sh; done
tmux new-session -d -s rspike-8 tools/keyecho.sh
tmux new-session -d -s rspike-9   # default shell
```
