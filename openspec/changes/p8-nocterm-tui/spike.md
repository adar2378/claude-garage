# Spike: nocterm wall — findings (2026-08-30)

**Verdict: GO on nocterm, as a vendored fork.** All three gates pass. The
rendering/widget/PTY bones held up; the input pipeline needed four fixes —
three fixed in this spike, one open. Pin a git SHA and carry patches in
`vendor/nocterm` (Vide, nocterm's own flagship, also depends on git, not pub).

Run it yourself: `./tools/setup-sessions.sh` (or see below), then
`bin/wall` inside a terminal. Keys: `1-9` focus · `Enter` engage ·
`Ctrl+Q` back · `Shift+PageUp/Down` local scroll · `q` quit.

## Gate 1 — rendering performance: PASS (condition: fps15)

3×3 grid, 200×55 host terminal, 5 tiles scrolling colored output at ~40
lines/s each + 2 spinner tiles (heavier than 9 real Claude sessions).

| frame cap | CPU (1 core) | input latency under load |
|---|---|---|
| fps30 (default) | ~22% | **>1100 ms, keys coalesce — unusable** |
| fps15 (`SchedulerBinding.instance.targetFrameDuration = FrameRate.fps15`) | ~13% | **8–9 ms** |

The default 30fps saturates the single Dart isolate; stdin reads starve
behind PTY/render work. fps15 (which the UX design wanted anyway as the tile
refresh cap) fully resolves it. RSS ~70 MB. Build-phase headroom ideas:
dirty-tile-only rebuilds, per-tile output coalescing.

## Gate 2 — verbatim key passthrough: PASS (with our encoder + patches)

Proof: in a **real Claude Code composer** inside a tile, typed text landed
and Alt+Left/Alt+Right word-jumped (inner cursor 13→8→2→8) — the exact bug
the web UI has. Trust-dialog arrow navigation also worked. Byte-echo tile
confirmed `h`, `^A`, `^C`, `^[[1;3C`, `^[f`, `^[[D` arrive byte-exact.

What it took (all in `bin/wall.dart` + `vendor/nocterm`):

1. **Own encoder, always.** `TerminalXterm`'s built-in key translation drops
   modifiers (Alt+Right → plain Right — the web-UI bug, reproduced in the
   framework). Our `onKeyEvent` consumes everything and re-encodes
   (logicalKey + modifiers + character → raw bytes). Parser events carry
   full modifier state, so this is lossless for Claude Code's needs.
2. **`stty -ixon` at startup.** nocterm's "raw mode" is only
   echoMode/lineMode=false; IXON stays on and the tty driver eats
   Ctrl+Q/Ctrl+S. (Upstream issue candidate.)
3. **Vendored patch: character batching.** nocterm converts multi-event
   stdin chunks of "printable" chars into a synthetic paste (Ctrl+V +
   ClipboardManager), and Enter's `\n` counted as printable — coalesced
   keystrokes silently became pastes, swallowing Enter, corrupting the
   garage key layer, and overwriting the user's system clipboard via OSC 52.
   Patch: `\n`/`\r` never printable; runs < 4 chars stay individual keys.
   (Upstream PR candidate.)
4. **Paste recovery.** Real bracketed paste arrives as synthetic Ctrl+V;
   engaged handler reads `ClipboardManager.paste()` and forwards as
   `ESC[200~ … ESC[201~`. Genuine Ctrl+V keypress is ambiguous with paste
   at framework level (upstream design issue; acceptable for now).

**Known open issue (RESOLVED — p8 task group 8):** Shift+Tab (`ESC[Z`)
parsed correctly in isolation but was dropped in the live app. Root cause
found and fixed in the fork: `InputParser._parseCSISequence` matched the
3-byte CSI finals (`ESC[Z`, arrows, home/end) only when the parser buffer
was *exactly* 3 bytes. Under load stdin reads coalesce, so `ESC[Z` shared
a read with other bytes and fell to the fallbacks — either the unknown-CSI
consumer ate it (`Z` is a valid CSI final byte) or the last-byte
completeness check left the whole read stuck until the 100ms staleness
clear dropped everything (observed live: `1b 5b 5a 09` in one read →
zero events at any handler). Vendored patch ("PATCHED (garage):
shift+tab") matches those sequences by prefix (`>= 3`), per ECMA-48 —
a final byte directly after `ESC [` terminates the sequence. Regression
test: `test/shifttab_regression_test.dart` (fails 5/8 pre-patch).
(Upstream PR candidate.)

**Chord collision:** Ctrl+G is hardcoded as nocterm's debug-mode toggle,
intercepted before dispatch. Spike uses Ctrl+Q as the disengage chord; final
chord needs either a vendored one-liner to disable the debug key or an
upstream "make it configurable" PR. (UX mockup says ctrl+g — revisit.)

## Gate 3 — scrollback seam: mechanism present, semantics ours (as designed)

`TerminalXterm` has a 10k-line buffer + scrollOffset, and Shift+PageUp
scrolls it — but the offset is relative to the buffer end, so live output
drags the view along; no freeze. The planned design (local read-only history
view, frozen anchor, "back to live" affordance, capture-pane seeding) is our
own render path over the same buffer. Confirmed feasible, not free.

## Extra findings for the build phase

- Tiles must survive inner-session death: killing/recreating a tmux session
  leaves the tile's `tmux attach` PTY dead — needs watch + reattach
  (PtyController has `restart()`).
- `TMUX` env var must be stripped from tile PTYs (nested-client refusal).
- Startup frames are expensive; input in the first ~2s coalesces (mitigated
  by the batching patch).
- Claude Code's alt-screen TUI renders legibly in a tile (box-drawing,
  dialogs, composer) at 1/3-width.

## Files

- `bin/wall.dart` — spike app (grid, 3-layer key routing, encoder, paste recovery)
- `vendor/nocterm/` — nocterm 0.9.0 + batching patch (`terminal_binding.dart`, marked `PATCHED (garage spike)`)
- `tools/stress.sh` / `spinner.sh` / `keyecho.sh` — load + verification programs
- `sandbox/ws-alpha`, `ws-beta` — sample workspaces

Session setup used:

```bash
for i in 1 2 3 4 5; do tmux new-session -d -s garage-spike-$i -c sandbox/ws-alpha tools/stress.sh; done
for i in 6 7;       do tmux new-session -d -s garage-spike-$i -c sandbox/ws-beta  tools/spinner.sh; done
tmux new-session -d -s garage-spike-8 -c sandbox/ws-beta tools/keyecho.sh
tmux new-session -d -s garage-spike-9 -c sandbox/ws-beta   # ran `claude` here
```

> Note (p8 merge): this spike's `vendor/` (a duplicate of the framework fork)
> is gitignored — the maintained fork lives at `tui/vendor/nocterm` with all
> patches registered in `tui/vendor/NOCTERM_VERSION`. To re-run the spike app
> or its tests, copy that fork back to `spikes/nocterm-wall/vendor/nocterm`.

## Framework fork (historical)

The frozen garage patches (1–9 over nocterm 0.9.0, registry in
`GARAGE_PATCHES.md` at the fork's root) live at
https://github.com/adar2378/nocterm, branch `garage`, pinned at
`2ba41b4fa75dd52840d79ab7fa2707d3ef1d3f11`. The Dart TUI client (`tui/`,
which vendored that fork) was removed at p9 (`p9-ratatui-port`) after the
Rust wall passed the full parity gate — the fork is history only and is
unused by the build.
