# Verification: p11-context-meters

Scope of this pass: tasks 3.0 (launcher staleness guard) and 3.1 (e2e suite +
regression sweep). Waves 1–2 (daemon telemetry, wall meters) were built and
unit-tested by an earlier wave; this file covers the closing verification
only. `npm test` (daemon + ui, node:test) is 109/109 green throughout;
`cargo test --release` in `wall/` is 367/367 green throughout.

## 3.0 — Launcher staleness guard (`bin/garage.js`)

Added `wallSourcesNewerThan()` (compares the mtime of every file under
`wall/src`, plus `wall/Cargo.toml`/`Cargo.lock`, against the resolved dist
binary) and `cargoBuildTuiBinary()` (the cargo-build-and-copy-into-dist step,
extracted so the existing "no prebuilt binary" path and this new path share
one implementation). `resolveTuiBinary()` runs the check only when the dist
binary already exists AND `wall/Cargo.toml` exists (a source checkout) —
never for the `GARAGE_TUI_BIN` override branch, which returns before this
code is ever reached, and never for an installed package (no `wall/`
sources to compare against).

Proof (`bin/garage.js tui`, scratch `GARAGE_DIR`/`GARAGE_PORT`, real cargo on
PATH throughout except where noted):

1. **Binary newer than sources (steady state)** — no message printed, no
   rebuild, binary mtime unchanged. (Implicit in every other e2e run below —
   dozens of `tui` launches against the checked-in dist binary produced zero
   spurious rebuilds.)
2. **Source newer than binary, cargo present** — `touch wall/src/runtime.rs`,
   then launched: printed exactly
   `wall sources newer than the built binary — rebuilding`, ran `cargo build
   --release` (visible `Compiling garage-wall … Finished`), copied the
   result into `wall/dist/garage-wall-darwin-arm64`, and continued into
   `no daemon on port … — starting one`. Binary mtime moved from
   `1788179777` to `1788179890` (newer than the touched source's
   `1788179878`) — confirmed via `stat -f %m`.
3. **Source newer than binary, no cargo on PATH** — same touch, but `PATH`
   stripped of every `.cargo` entry: printed exactly
   `warning: wall/src is newer than the built TUI binary (…) and no cargo on
   PATH to rebuild it — it may be stale`, then **continued** into daemon
   startup (did not exit). Binary mtime unchanged, confirming no rebuild was
   attempted.
4. **`GARAGE_TUI_BIN` override, source newer than the real dist binary** — a
   fake executable script pointed to via `GARAGE_TUI_BIN`; launched with
   `wall/src/runtime.rs` still touched-newer: no rebuild message, no cargo
   invocation, the real dist binary's mtime unchanged, and the fake script
   ran verbatim (printed `FAKE TUI RAN`) — the override is honored exactly
   as written, the guard never runs for it.

`npm test` stayed green after this change (109/109) — no test exercises
`resolveTuiBinary` directly (only indirectly, through run_p82.sh's
stale-daemon-gate stage, which also passed — see the sweep table below).

## 3.1 — `wall/test/e2e/run_p11.sh`

New harness, 35 checks, scratch port 4800 + scratch `GARAGE_DIR` +
**scratch `GARAGE_CLAUDE_HOME`** (both env vars exported to the daemon and
to every `tui` launch, matching the task's "for both daemon and wall"
instruction — the wall does not read `GARAGE_CLAUDE_HOME` today, but the
launcher does honor `GARAGE_DIR`, so both ride along uniformly with every
other e2e harness in this repo). Fully self-cleaning (verified back-to-back
runs, no leftover `p11e2e-*`/`garage/p11e2e-*` tmux sessions or scratch
files); never touches the real daemon on 4747 or `~/.garage`/`~/.claude`.

Coverage:

- **Statusline ingest → tile meter, dim vs red (SGR-verified)**: a fake
  `POST /api/statusline/claude` (token pulled from `/api/hooks/snippet`,
  same per-install secret the wrapper uses) with `context_window.used_percentage`
  42 and 88, resolved via **cwd fallback** (no `session_id` needed — the
  payload carries only `cwd`, matching a spawned session's dir). Posted
  *before* the TUI's first launch so the wall's one-shot startup fetch
  already reflects it (sidesteps any poll-interval wait). Verified two
  ways: the exact meter text (`▰▰▱▱ 42%`, `▰▰▰▰ 88%`) **and** the literal
  SGR foreground-color escape sequence immediately preceding it in
  `tmux capture-pane -e` output — `\x1b[38;5;7m` (dim/gray) for 42%,
  `\x1b[38;5;1m` (red) for 88%. These codes were determined empirically
  against this exact binary (see "SGR color codes" below) rather than
  assumed from the ratatui `Color` enum name.
- **No-data session shows no meter**: a fourth session with no statusline
  post and no transcript fixture; its tile's own title-bar line (found by
  grepping its unique label, not by grepping `%` globally — the strip's own
  usage chip contains `%` too and would false-negative a blanket check) is
  asserted to contain no `%` at all.
- **Strip usage chip**: the same statusline POST's `rate_limits` renders as
  `5h 91% · wk 33%` in the strip, present regardless of which workspace is
  focused.
- **Transcript fallback**: `GARAGE_CLAUDE_HOME` scratch dir; a `claudeSessionId`
  is injected directly into the scratch `GARAGE_DIR/state.json` (documented
  below — there is no API for this), a JSONL fixture is written under
  `projects/<slug>/<claudeSessionId>.jsonl` with a synthetic assistant
  `usage` block (100000 tokens / 200000 default window = 50%), and the
  daemon's cache is primed via `GET /api/sessions` polling *before* the TUI
  launches. The meter (`▰▰▱▱ 50%`, dim) appears with **zero** statusline
  posts ever made for that session; the priming curl loop also asserts
  `source: "transcript"` in the JSON directly.
- **Chaining wrapper byte parity**: a fake pre-existing `statusLine` command
  (`"cat"`) written straight into the scratch `settings.json`, then
  `POST /api/statusline/install` (asserted `chained: true`). The generated
  wrapper script is then run directly (not through the daemon) with a
  sample JSON stdin; its stdout is asserted byte-identical to running `cat`
  alone on the same stdin, and to the stdin itself — proving the
  fire-and-forget background `curl` never leaks into or corrupts the
  chained command's output.
- **`I` install-from-wall**: pressing `I` in the TUI fires
  `POST /api/statusline/install` and shows the exact success notice text
  (`statusline feed installed — meters go live as agents work`).

### Injecting `claudeSessionId` for the fallback test

There is no API to set a session's `claudeSessionId` — production only ever
learns one from the poller observing a real `claude agents --json` match
(never true for these zsh-backed e2e sessions). The harness writes it
directly into the scratch `GARAGE_DIR/state.json`'s `sessions[<id>].claudeSessionId`
via a small Python script, matching the same field `upsertSessionMeta`
would have written. This is safe here specifically because: (a) it targets
a fully scratch, per-run `state.json`; (b) the poller only ever *adds*
`claudeSessionId`/`workspace`/`label` for sessions it can pid-match to a
real `claude agents --json` entry — it never *clears* or fights an existing
value for a session it can't match, so the injected value is never
clobbered by a background poller tick.

### Transcript fixture path flake (macOS-only, environmental — not a garage bug)

While building this scenario, `tmux`'s `pane_current_path` was observed to
intermittently report the **raw** (`/var/folders/...`) vs. **fully
`realpath`-resolved** (`/private/var/folders/...`) form of a scratch dir
under `$TMPDIR` across *successive* queries from the same long-lived daemon
process — reproduced with a minimal debug harness that exposed
`transcript.js`'s internal cache directly (`getCacheEntry`), confirming
`computeContext` itself is correct in isolation (returns the right
percentage every time, given the right `dir`) but the `dir` string
`sessions.js` actually passes to it flickered between the two forms across
requests milliseconds apart. This is macOS `tmux`/`proc_pidinfo` path-cache
behavior on a symlinked `$TMPDIR` (`/var` → `/private/var`), not a garage
bug — `daemon/src/tmux.js` and `daemon/src/transcript.js` are unmodified
and behave correctly for whatever `dir` string they're actually given.
`write_transcript_fixture()` in `run_p11.sh` writes the fixture under
**both** the raw and `realpath`-resolved slugs (when they differ) so the
test never depends on which form a given request happens to see — a
test-robustness measure only, not a workaround for a product defect.

### SGR color codes (empirical)

Built the current dist binary, ran it against a scratch daemon with a
41%/88% context posted, and read `tmux capture-pane -e` directly:
`colors::DIM` (`Color::Gray`) renders as `\x1b[38;5;7m`; `colors::CTX_HOT`
(`Color::Red`) renders as `\x1b[38;5;1m`. Both were confirmed by locating the
exact byte sequence immediately preceding the meter's leading space in the
raw SGR stream, matching `tile.rs`'s `Span::styled(format!(" {}", …),
Style::default().fg(context_meter_color(pct)))`. `run_p11.sh`'s
`meter_color_ok()` asserts these literal sequences.

### JSON payload construction (bash gotcha, harness-only)

A hand-escaped JSON literal with **nested** braces (e.g.
`"{\"rate_limits\":{\"five_hour\":{...},\"seven_day\":{...}}}"`) inside a
bash double-quoted argument, itself nested inside a `$(...)` command
substitution, was observed to be silently shredded into multiple words —
`statusline_post` was invoked six times with JSON *fragments* instead of
once with the whole payload, each fragment posted as its own (mostly
invalid) request. A flat single-level JSON literal (`put_ws`/`spawn_body`'s
`{"name":"...","dir":"..."}`) was unaffected — only the nested-brace,
multi-comma shape triggered it. Root cause not fully chased down (bash
brace-expansion-adjacent word splitting inside escaped-quote strings, best
guess); the fix — building the payload with `printf '{"cwd":"%s",...}' "$VAR"`
into a plain variable first, then passing that variable to `check`/`curl` —
sidesteps it entirely and matches the discipline `run_p10.sh`'s own `hook()`
helper already documents ("a plain variable sidesteps it entirely"). Applied
to both statusline POSTs in `run_p11.sh`; no other script in this repo
builds JSON with nested braces this way, so nothing else needed the same
fix.

### The "focused ≠ gridded" footgun (harness-only)

`run_p11.sh`'s first draft gated a workspace-focus wait on
`outer_has '<label>'`, which is **always already true** before the focus
keypress even lands — the rail lists every session in every workspace
regardless of focus. This raced the very next assertion against a stale
screen. Fixed with a `gridded()` helper that requires the tile's own
top-left border corner immediately adjacent to its glyph (`"╭ ○ <label>"`),
which only appears on the actual rendered tile — this is precisely the
footgun `run_p10.sh`'s own comments already call out ("a bare `outer_has
'<label>'` never proves a tile's CONTENT is actually gridded"); this pass
just tripped over it independently before finding that note.

## Regression sweep (fresh `npm run build:tui` binary)

Rebuilt `wall/dist/garage-wall-darwin-arm64` from the current `wall/src`
(`cargo build --release`, 367/367 `cargo test --release` green immediately
before), then ran every e2e suite against it, each in its own scratch
port/dir, cleaned between runs:

| Suite | Result | Checks |
|---|---|---|
| `run_e2e.sh` | PASS | 35/35 |
| `run_p81.sh` | PASS | 37/37 |
| `run_p82.sh` | PASS | 30/30 |
| `run_p83.sh` | PASS | 24/24 |
| `run_p84.sh` | PASS | 25/25 |
| `run_p10.sh` | PASS | 67/67 |
| `run_p11.sh` | PASS | 35/35 |

`npm test` (daemon + ui): 109/109 pass throughout.

### Observed pre-existing flake in `run_p81.sh` (not a p11 regression)

This machine ran under sustained heavy load throughout this session (other
IDE/browser/build processes; `uptime` load averages 4.5–7.3 on what's
otherwise a normal box) while wave-3 verification ran. Under that load,
`run_p81.sh`'s step 6 ("R restores every restorable in the workspace" —
which re-kills a just-restored session *and* adds a brand-new,
never-live, meta-only restorable entry in the same beat) intermittently
failed its very next check (`wait_for 25 outer_has 'press Enter…'`) in
roughly half of ~8 repeated attempts, always at the same step, and always
resolved on retry. At every observed failure, `GET /api/sessions` queried
directly showed the *correct* `restorable: true` state for both sessions
well within a second — the daemon's own data was never wrong. A stripped-down
reproduction (scratch daemon + tmux only, no TUI attached, same exact
kill/add-meta/kill sequence) never failed across several attempts and
showed the SSE `sessions` event firing correctly every time, meaning the
daemon-side poller/SSE mechanism is not obviously at fault either — the
gap is somewhere between the daemon's SSE push and the *TUI's* refetch/
render landing on screen, specifically under this sequence and under load.
This is pre-existing (`daemon/src/poller.js`, `daemon/src/events.js`,
`wall/src/runtime.rs`'s SSE client, and `run_p81.sh` are all untouched by
p11) and unrelated to context-telemetry; not fixed here per this change's
scope (report precisely, don't fix `wall/src`/`daemon/src`). The sweep
table above records a clean run once achieved; recommend a follow-up look
at the wall's SSE reconnect/refetch path under load if this resurfaces.
