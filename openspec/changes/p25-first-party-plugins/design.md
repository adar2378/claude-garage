> **Detailed design is written per plugin as each is built.** Decisions made earlier are below; the usage bar, pit pet and turn markers are designed in the sections that follow them. The other plugins add their own sections.

## Decisions made

**1. The acceptance test.** Every "plugin" row in the proposal's map runs as a plugin through the public plugin API only, with no private hooks. If a plugin needs something the API lacks, the gap is added to the plugin host (p24), not worked around.

**2. Core owns data and lifecycle; plugins own opinions and presentation.** Example: the daemon keeps closed-session records (data), the closed-chats plugin decides how to show them.

**3. Core items stay core.** Draft and screenshot recovery needs to read every keystroke sent to a session; without a sandbox that is too much trust to hand a plugin. Selection, copy and sidebar layout are terminal fundamentals.

**4. First-party plugins live in the repo** (`plugins/<name>/`), ship with garage and are enabled by default where the feature existed before (pit pet). They use the same manifest and install path as third-party plugins.

**5. Freeze rule.** Plugin API v1 is frozen only when every plugin here works on macOS and Linux. Freezing earlier would freeze guesses.

## Open for the phase start
- Which plugins are enabled by default.
- Whether turn markers need a new scrollback slot or can use widgets.
- Source of truth for the usage figures (statusline only, or transcript fallback).

## Resolved (2026-10-31, user: all defaults)

- On by default: usage bar, closed chats, quick send, close/delete buttons, pet. Off by default: jump palette, turn markers.
- Two plugins in one slot: both render, in install order; a config setting can reorder them.

## Widget plugins: usage-bar, pit-pet, turn-markers

Three plugins that only draw. They use the public API (widgets, events, `GET` of core routes, `ui/action`) and nothing private. Each is a Rust crate `plugins/<id>/` with a `[lib]` (logic, unit tested) and a `[[bin]]` named `<id>` (what `stage-plugins.sh` copies and the manifest runs as `{plugin_dir}/bin/<id>`). Manifests are the source of the compiled bundled allowlist (`crates/daemon/build.rs` hashes every `plugins/*/garage-plugin.toml` except `hello`), so adding the three needed no daemon change. Defaults come from `bundled::DEFAULT_ON`: usage-bar on, pit-pet on, turn-markers off.

Shared pieces added to `crates/plugin-sdk` (kept small):
- `state_dir()`, `read_state_in` / `write_state_in`: JSON files in the plugin's own storage directory, written by temp file plus rename. A damaged file reads as "no state" (storage is a cache the plugin can rebuild).
- `Api::get(path)` for core `GET` routes.
- `StateLock` (pit-pet, turn-markers): a `create_new` lock file with a stale timeout, because the host runs up to 4 runs of one plugin at once and they all read-modify-write the same file.
- `fake::FakeDaemon` (`#[doc(hidden)]`): a scripted loopback daemon for plugin tests.

### usage-bar (2.5)

**What it shows.** Exactly what the core chip showed: one flow item in `strip.right`, priority 50, one segment `" 5h N% · wk M% "` styled `fg: dim`; a missing window is left out; none at all draws nothing. The percentage is rounded and clamped like the wall did (`23.5` shows `24%`).

**Data.** `GET /api/usage` (the daemon keeps the last statusline report in memory). Triggers: `daemon.started` (the daemon forgets widgets when it restarts), `session.turn` (the statusline reports after every turn) and a 60 s `timer` (the wall's old poll interval; no `usage.changed` event exists, see gap G6).

**Persistence.** After a successful read the plugin writes `usage.json` (`{five_hour, seven_day}`, each `{pct, resets_at}`) to its state dir. When the daemon has no figures (it just restarted, no statusline report yet) the plugin draws the stored ones, dropping any window whose `resets_at` has passed. A window the daemon knows replaces the stored one; the other keeps its stored value. Nothing known: the widget is cleared. Reset times are RFC 3339 strings or numeric epochs (seconds or milliseconds); the small parser is unit tested because the plugin has no time crate.

**Replacing the core chip.** The core provider (`usage_chip_provider`) and its tests are removed, so there is no duplicate. The strip snapshots `strip_with_usage_chip_and_notice` and `strip_narrow_drops_right_side_content` now render the plugin's golden widget body (`plugins/usage-bar/golden/*.json`) through the real wire-to-slot path. The plugin crate asserts it produces those same files. The `.snap` files did not change: pixel identity held, so no snapshot was updated.

### pit-pet (2.9)

**Moved as is.** `wall/src/ui/pet/engine.rs` (species, moods, sprites, the motion sim, the chatter scheduler and its tests) moves to `plugins/pit-pet/src/engine.rs` unchanged apart from `serde` derives so the sim state can be stored. The wall loses `ui/pet/`, the `pet` cargo feature, `PetHost`, `PetRender`, `pet_slot_item`, the `P` binding, `pet.pet`, `pet.cycle_species`, `WallState.pet`, `set_pet`, the `pet` field of `wall.json`, and the usage poll that only fed pet chatter.

**Who does what.** The plugin owns the simulation. The TUI cycles frames from its own clock and draws them; no process ticks while the pet animates.

**The frame set.** One animated filler item in `strip.right`: up to 24 frames, `frame_ms = 300`. Frame `i` is one simulated tick: the sprite text (dim for sleep and box, normal otherwise), a reserved bang column, and the sprite's column `x`. The wire gains one optional field for this, `frame_x` (gap G1): a per-frame filler column parallel to `frames`, because a walking pet changes column and an item has one `placement.x`. Each frame is clickable: calm frames with `pit-pet:pet`, alert frames with the core `jump.blocked` (the wall resolves the click of the frame on screen). The chatter line is the item's `placement.caption`.

**Why a rolling window.** The TUI picks the frame by `floor(now / frame_ms) mod len`, so a set is a loop on the wall clock, not a sequence that starts when it arrives. The plugin therefore places simulated tick `k` (time `t0 + k * 300`) at index `floor((t0 + k * 300) / 300) mod 24`. As long as a new set arrives within `24 * 300 = 7.2 s` the pet moves continuously; the 6 s `timer` event (the timer minimum is 5 s) guarantees that. Every push carries `ttl_secs = 15`, so a pet that stops being renewed disappears. A late run replays the start of the old window (the pet steps back a few columns) and carries on; it never shows garbage.

**Stored state (`pet.json`).** Species, the sim, the chatter limiter, the rng, and the last window as a trace of `(tick time, sim, rng)`. A run does not continue from the end of the old window (which is up to 7 s in the future); it takes the trace entry for the current tick and simulates forward from there with the fresh inputs. That is what makes an event-triggered run (a session needs input) start from where the pet really is.

**Inputs.** `GET /api/sessions`: needs-input count, any working, session count (restorable sessions count, as before). Usage percentage for the chatter's "usage is high" line: `GET /api/usage`. Strip room: `ui.slots["strip.right"].filler` from the run context (x1 - x0 columns; the old code subtracted the two gutter columns the same way). Local hour: `date +%H`, at most once a minute, as before.

**Triggers.** `timer` every 6 s (keeps the window rolling), `daemon.started`, `session.created`, `session.closed`, `session.status_changed` (mood changes within a few hundred ms; at most one push per 300 ms, the user's own key and a daemon restart are never debounced), and two actions: `pit-pet:cycle` (default key `P`, off, Arthur, Papito, Segan, off; resets the sim; the arrival text is the new pet's caption) and `pit-pet:pet` (the click).

**Click.** An alert frame jumps to the blocked session by itself (`ui.navigate`, G4); the plugin is not run. `pit-pet:pet` runs for a calm frame and pets: the sim celebrates for 1.2 s, a petting line becomes the caption, and the window is pushed again.

**Transients.** A caption lasts 4 s (chatter) or 2.5 s (petting). A run that sets one sleeps until it expires, takes the lock, and pushes the same frames without it, but only if no newer push happened meanwhile (a `rev` counter in the state). `ttl_secs` is no use: it removes the whole item, not the caption. The celebrate after petting or after the blocked count falls to zero needs no timer: it is part of the simulated frames.

**Differences from the in-wall pet** (all listed as gaps in tasks 4.1):
- The "box" mood (daemon down) cannot be shown: a plugin cannot run when the daemon is gone, and the widget is frozen in the TUI on its last frame (G3).
- The needs-input `!` uses the `needs-input` token, so it is amber while a session waits for you, as before (G2).
- The species choice moves from `wall.json` to the plugin's state. The plugin reads the old `pet` value from `wall.json` once, on its first run, so existing users keep their pet.
- The `P` arrival notice ("Arthur the shop cat is on the wall") becomes the pet's caption; "the strip is quiet again" is a plugin notice (toast), as before (G5).
- `GARAGE_PET_ASCII=1` cannot reach a plugin (its environment is cleared): the pup's ASCII faces are on when a file named `ascii` exists in the plugin's state dir. `GARAGE_PET_CHATTER_MS` (an E2E-only override) is dropped (G10).

**Pet snapshot.** `strip_with_the_pet_in_the_filler` is rebuilt from golden widget bodies (`plugins/pit-pet/golden/*.json`) via the real path, and the plugin crate asserts it generates them. The text and every column are the same; only the lit `!` cell's colour differs (see above).

### turn-markers (2.6)

**What it shows.** A one-column marker in the left gutter of a tile at the scrollback row where each agent turn ended. Off by default.

**Data and anchoring.** The daemon anchors each turn. When it handles the Stop hook it asks the backend for the session's `#{history_size}` and `#{cursor_y}` (tmux `display-message`; the host backend answers the same from its vt100 screen through a new control frame) and puts `row = history_size + cursor_y` (absolute: 0 is the oldest retained line) and `history_limit` in the `session.turn` event, next to `at` and `transcript_offset`. A backend that cannot say leaves them `null`; the plugin then draws nothing for that turn.

**Plugin.** Keeps the last 32 turns per session in `turns.json` and drops a session's list on `session.closed`. A push is `tile.gutter` items `{row, kind: "marker"}`, newest turns first across all sessions, until the body would pass 11.5 KiB (limit 16 KiB), so a busy wall loses its oldest markers rather than the push. `daemon.started` pushes everything again, because widgets do not survive a daemon restart.

**Wall.** Draws a marker at `row - top` of the view (live tail: the polled history size; frozen view: its own top). The history poll also reads `#{history_limit}`; a marker whose row is below `history_size - history_limit` is not drawn. The first-draw placement of an earlier version (`turn` field, "first seen within 10 s") is removed.

**Known limit.** The row is absolute in tmux's numbering. Once a pane's history is full, tmux trims its oldest lines and every row shifts by the number trimmed; tmux keeps no counter of that, so an old marker can sit a few lines low (or be dropped) in a pane with a long history. Markers on a pane that has not reached its limit are exact.
