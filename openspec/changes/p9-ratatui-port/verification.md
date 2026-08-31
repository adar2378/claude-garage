# p9-ratatui-port — task 6 parity-gate verification (2026-08-31)

The full p8 e2e surface run against the Rust wall, the spike's open caveat
(real Claude Code composer) closed, and the performance/visual comparison
recorded. **All six suites green — 178/178 checks — plus the real-claude
smoke.** Two real defects were found and fixed by the gate (one in `wall/`,
one in the harness's SGR parser); details in the failure→fix log.

## Environment

| | |
|---|---|
| Host | macOS 26.6.2 (Darwin 25.6.0), Apple M1 Pro (arm64) |
| tmux | 3.7b |
| node | v22.22.3 |
| rustc / cargo | 1.94.1 |
| claude (6.2 smoke) | 2.1.251 (Claude Code) |
| Binary under test | `tui/dist/garage-wall-darwin-arm64`, rebuilt fresh via `npm run build:tui`; sha1 `88924f25a22d20398d6a5ebc0dcf471d61265bd7`; 2,451,824 bytes |
| Dart comparison binary | `tui/dist/garage-tui-darwin-arm64` (p8 dist); 9,048,624 bytes |
| Unit suites | `cargo test --release`: **253 pass** (252 lib + 1 PTY-teardown integration); `npm test`: **47 pass** |

Safety: every run used a scratch port (4793–4798) + scratch `GARAGE_DIR`;
the live daemon on 4747 (pid 45410) and `~/.garage` were never read or
written; only `e2e-*`, `garage/e2e-*`, `p8x-*`, `garage/p8x-*`, `p9gate-*`,
`garage/p9gate-*` tmux sessions were created/killed and none leaked.

## 6.1 — full e2e surface against the Rust binary (`GARAGE_TUI_BIN`)

All suites invoked as
`GARAGE_TUI_BIN=$REPO/tui/dist/garage-wall-darwin-arm64 bash tui/test/e2e/<suite>`;
every suite listed below was run to completion against the FINAL binary
(sha1 above — `run_e2e.sh`/`run_p81.sh` were rerun after the cursor fix so
all six results describe the same build).

| Suite | Checks | Result | Notes |
|---|---|---|---|
| `run_e2e.sh` | 35 | **PASS** | incl. latency <50 ms at 200×55 and 250×70 (5 ms medians), tty restore, q-quit |
| `run_p81.sh` | 38 | PASS | lifecycle: PTY sizing, maximize, restore round-trips, x/X arming, worktree keep |
| `run_p82.sh` | 30 | PASS | launcher stale-daemon swap ×2, engaged-cursor contract, frozen view, vim alt-screen — via `node bin/garage.js tui` (exercises the new `GARAGE_TUI_BIN` launcher hook end-to-end) |
| `run_p83.sh` | 24 | PASS | ▸ focus marker, [ ] cycling, rail clicks, X-X unregister, unregistered-group resurface |
| `run_p84.sh` | 25 | PASS | X-X keep / X-K kill-all, empty-workspace arm wording |
| `run_click_smoke.sh` | 26 | PASS | tile/rail/badge/queue/overlay clicks, typing hint |
| **Total** | **178** | **PASS** | |

`run_click_probe_smoke.sh` excluded by design: it compiles and drives
`tui/test/e2e/click_probe.dart` — a probe of the **nocterm Dart framework's**
mouse decoding, with no daemon and no app binary under test. Nothing in it
can point at the Rust wall; it goes away with `tui/` in task 7.

### Harness changes made for the gate (no assertion weakened)

- `bin/garage.js` `resolveTuiBinary()`: `GARAGE_TUI_BIN` added as the
  highest-priority override — documented as a test hook, used only when set;
  set-but-not-executable is a hard error (a test hook must never silently
  fall through to a different binary). Required because `run_p82.sh`
  launches through `node bin/garage.js tui`. `npm test` stays 47/47.
- `run_e2e.sh`: given the same scratch-port + scratch-`GARAGE_DIR` safety
  plumbing the p8.1–p8.4 harnesses already had (`GARAGE_E2E_PORT`, default
  4794; daemon started with `GARAGE_PORT`/`GARAGE_DIR`; TUI launched with
  `-e GARAGE_TUI_PORT`). Previously it hardcoded 4747 + `~/.garage`, which
  the gate's safety contract forbids while a live daemon runs there. Checks
  themselves are untouched.

### Failure → fix log

1. **`run_p82.sh` "unengaged tile shows NO cursor" FAIL — harness parser
   false positive.** The embedded `revcells.py` SGR parser split
   `ESC[38;5;7m` (indexed-color foreground 7 — ratatui's `Gray`, which the
   Rust wall legitimately emits for dim text and borders) into independent
   codes and read the `7` as SGR 7 inverse video, flagging the entire screen
   as inverse. Per ECMA-48, `38;5;N` / `38;2;R;G;B` (and 48/58) are one
   extended-color parameter. Fix: the parser now consumes extended-color
   sequences whole. The assertion (exactly 0 / exactly 1 inverse cell) is
   unchanged and still detects real `ESC[7m` (verified on synthetic input,
   including a truecolor sequence with a `7` channel). The Dart wall had
   passed only because its truecolor emissions never contained a bare 7.
2. **`run_p82.sh` "engaged live tile shows EXACTLY ONE inverse-video cell"
   FAIL — real `wall/` bug.** tui-term 0.3.4 paints its REVERSED overlay
   only when the cursor cell **has contents**; on an empty cell — the common
   case, the cursor resting just past the prompt/composer text — it draws a
   `█` glyph with a plain gray foreground instead, i.e. no inverse cell at
   all. Fix in `wall/src/ui/tile.rs`: the engaged cursor is configured with
   `symbol(" ")` + `style(REVERSED)` so both tui-term paths render exactly
   one inverse cell. Unit test added
   (`engaged_cursor_is_one_inverse_cell_even_on_an_empty_cell`: empty-cell
   path, contents path, unengaged = zero). Rebuilt; `run_p82.sh` rerun fully:
   30/30.
3. *(Transient, not a fix)* one flaky failure of the PTY integration test
   `teardown_never_injects_bytes_into_attached_panes` during a
   `cargo test --release` run that overlapped harness tmux activity; passed
   on standalone rerun and in every later full run. tmux-global test, no
   code change.

## 6.2 — real Claude Code composer smoke (the spike's open caveat) — PASS

Real `claude` 2.1.251 (no `GARAGE_CLAUDE_CMD`), scratch daemon on 4793 with
scratch `GARAGE_DIR`, fresh scratch workspace dir (forces the trust dialog),
Rust wall at 200×55 in a scratch outer tmux session. **No prompt was ever
submitted** — Enter was pressed only in the trust dialog; the composer was
cleared before exit.

| Check | Result | Evidence |
|---|---|---|
| Trust dialog answered through the engaged tile with Down+Enter | PASS | dialog defaults to `❯ No, exit`; Down moved `❯` to `Yes, I trust this folder` (arrow routed byte-exact), Enter accepted |
| Composer renders in the tile | PASS | `❯` prompt + `⏵⏵ auto mode on (shift+tab to cycle)` mode line |
| Typed text lands | PASS | `alpha beta gamma` visible on the inner composer line |
| Engaged cursor = one SGR-7 inverse cell tracking the composer | PASS | `capture-pane -e` parse: exactly one cell — (50,31) empty composer → (50,47) after 16 chars (+16 cols) → (50,42) overlaying `g` after word-jumps |
| Alt+Left/Right word-jump via inner `cursor_x` | PASS | 18 →(Alt+Left×3) 13, 8, 2 (starts of gamma/beta/alpha); →(Alt+Right×2) 8, 13 |
| Shift+Tab cycles Claude's mode line | PASS | auto mode → (default, no line) → accept edits on → plan mode on |
| a-jump after a hook Notification lands engaged | PASS | Ctrl+G to garage, `Notification` hook → `blocked` badge; `a` → chip `keys → p9gate-a/main`, engaged cursor present |
| Clean exit before teardown | PASS | Ctrl+C cleared the composer, Ctrl+C ×2 exited claude; inner session ended; wall showed the restorable placeholder; q-quit; scratch daemon stopped; live daemon/`~/.garage` untouched |

## 6.3 — numbers and visual pass

### Latency / CPU / RSS — standard load (5 stress + 2 spinner), keylog method, 5 samples

Same rig, same day, both binaries (Dart measured back-to-back for a fair
same-host baseline; spike and p8 rows quoted from their records).

| Binary | Size | Latency samples (ms) | Median | %CPU (3× 1 s) | RSS |
|---|---|---|---|---|---|
| **Rust wall** | 200×55 | 5, 8, 5, 5, 6 | **5 ms** | 1.5 / 1.8 / 2.2 | **6.4 MB** |
| **Rust wall** | 250×70 | 5, 5, 7, 6, 5 | **5 ms** | 2.7 / 2.9 / 2.9 | **7.6 MB** |
| Dart wall (same rig) | 200×55 | 229¹, 6, 6, 5, 6 | 6 ms | 8.6 / 8.7 / 9.4 | 61.8 MB |
| Dart wall (same rig) | 250×70 | 5, 5, 5, 5, 5 | 5 ms | 15.2 / 17.0 / 17.2 | 60.6 MB |
| spike (SPIKE.md) | 200×55 | 5, 6, 6, 6, 6 | 6 ms | 1.7 / 1.9 / 1.7 | 5.9 MB |
| spike (SPIKE.md) | 250×70 | 5, 6, 6, 6, 6 | 6 ms | 2.7 / 2.6 / 2.4 | — |
| p8 record (Dart) | 200×55 | 6, 5, 6, 6, 5 | 6 ms | 9.7 / 10.4 / 11.0 | ~70 MB (spike) |
| p8 record (Dart) | 250×70 | 6, 6, 6, 5, 5 | 6 ms | 10.1 / 11.6 / 11.8 | — |

¹ Dart's known cold-start first-keypress coalescing (p8 verification noted
96 ms once; the median is unaffected).

In-suite (`run_e2e.sh`'s own 5-stress load, final binary): 5 ms medians at
both sizes; CPU 1.8% at 200×55, 2.8–3.2% at 250×70.

**Binary size:** Rust 2,451,824 B (2.4 MB) vs Dart 9,048,624 B (9.0 MB) —
3.7× smaller. (The 1.1 MB spike binary was the reduced spike feature set.)

Summary vs the Dart wall: same latency, ~5× less CPU at both sizes, ~8×
less RSS, 3.7× smaller binary. Vs the spike: equal within noise (the full
app costs nothing measurable over the spike skeleton).

### Side-by-side visual pass at 200×55 — PASS, three cosmetic drifts reported

Same scripted state rendered by both binaries sequentially (scratch daemon:
two workspaces — `p9gate-amb` with one needs-input session carrying a
message, focused `p9gate-vis` with three sessions — grid, rail with amber
ordering + ▸ marker, strip with blocked badge), `capture-pane` plain + `-e`,
diffed with elapsed timers normalized.

Identical: grid geometry byte-exact (border rows/columns at identical
positions, tile interiors byte-identical), the entire strip line byte-exact
(tabs, `●` amber dot, `● 1 blocked` badge, `keys → garage` chip), rail
content (glyphs, ordering, ▸ marker, timer placement), and styling
structure — per-role SGR counts match 1:1; **amber is byte-identical
truecolor `38;2;255;179;64` in both** (the reserved hue survives exactly).
Neutral tones differ only in encoding: Dart truecolor (e.g.
`38;2;98;104;117`) vs Rust ANSI-256 (`38;5;8`) — visually equivalent
palette entries, same cells styled.

Drift found (all cosmetic, none functional, reported per the gate):

1. **Tile title decoration**: Dart `╭─  ✓ label  ──…`, Rust `╭ ✓ label ──…`
   — ratatui places the title one cell after the corner with single-space
   padding; nocterm inset it by one border segment with double-space padding.
2. **Rail vertical offset**: the rail block starts on screen row 1 in Rust
   vs row 2 in Dart (Dart leaves the first rail row blank); rows 1–53 are
   otherwise identical modulo the offset.
3. **Rail indent**: rail rows are indented one column less in Rust.

Plus the documented wording drift the gate permits: frozen-view affordance
`↓ live · +N` (Dart) vs `↓ live · +N lines` (Rust).

Verdict: layout parity holds; the drifts above are the complete list.

## Deviations

- `run_click_probe_smoke.sh` excluded (Dart-framework-scoped; rationale in
  6.1).
- `run_e2e.sh` gained scratch-port/scratch-`GARAGE_DIR` plumbing (safety
  requirement, zero check changes); `run_p82.sh`'s SGR parser was corrected
  per ECMA-48 (assertion strictness unchanged) — both detailed above.
- Dart RSS/CPU re-measured on this rig for a same-day baseline; they came
  in slightly better than the p8-recorded numbers (61–63 MB vs ~70 MB,
  8.6–9.4% vs 9.7–11.0% at 200×55) — the comparison uses the same-rig rows.
- 6.2 exit path: Ctrl+D on the empty composer did not exit claude 2.1.251
  (single press); Ctrl+C ×2 exited cleanly.
