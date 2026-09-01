# Proposal: p15-pit-pet

## Why

The pit pet (Arthur the shop cat, Papito the rubber duck, Segan the pit
pup) is the wall's one ambient, emotional signal — and it exists only in
the web wall. The TUI wall, now the primary surface since the p6–p14 arc,
has no pet at all: nothing glanceable that says "you're needed" or "all
clear" without reading a badge. Porting it also opens the door to what the
web pet never had: a voice. A pet that occasionally says "drink water" or
"proud of you" in its own character turns a status indicator into a
companion for long sessions.

## What Changes

- **One-row pit pet in the TUI strip**: an opt-in ASCII creature that
  lives in the bottom strip's filler gap (between the workspace tabs and
  the right-aligned notice/badge/chip cluster). Every mood from the web
  spec — sleep, watch, alert, box, celebrate — rendered as a single-row
  sprite per species, with species-true motion adapted to one row (the
  cat saunters and ignores strolls, the duck waddles and never bounces,
  the pup is fastest and does zoomies; the pup's "bounce" becomes a
  blinking `!`).
- **Mood is derived wall state**, same rules as the web: daemon SSE
  disconnected → box; no sessions → sleep; any `needs-input` → alert
  (runs left toward the rail with `!`); any `working` → watch; else
  sleep. Last blocked session clearing → transient celebrate.
- **`P` cycles the roster** off → Arthur → Papito → Segan → off, with a
  strip notice naming who arrived. Off by default. Choice persisted in
  `wall.json` alongside views. Listed in the `?` help overlay.
- **Click routing**: a click on the pet while alert performs the `a`
  jump; a click otherwise is petting (short celebrate + a petting reply).
- **Chatter** (new): every few minutes, rate-limited (≥3 min apart, ~4 s
  on screen), the pet says one line in its own voice through the strip's
  notice slot — hydrate, encourage, proud, stretch, late-night, usage
  heads-up, daemon-back. Some lines are earned (celebrate → proud;
  wall open 60+ min → stretch/hydrate weight up; after 23:00 local →
  late-night; usage chip ≥ 80% → pace-yourself; box → live → "you're
  back"). Chatter never fires while alert, never while a non-pet notice
  is showing, and never appears inside a tile. Off with the pet; no
  separate toggle.
- The daemon `extended-keys-format csi-u` fix identified during this
  session's terminal investigation is **out of scope** here (separate
  patch, see design.md Non-Goals).

## Capabilities

### New Capabilities
- `tui-pit-pet`: the TUI pet — roster + `P` cycling + persistence,
  one-row sprites and moods, species-true one-row motion, click routing,
  and species-voiced chatter with its earned triggers and rate limits.

### Modified Capabilities
- (none — `pit-pet` stays the web wall's spec; `tui-wall`'s strip
  requirement is additive-compatible: the pet only occupies filler
  columns and never displaces tabs, notice, badge, or chip.)

## Impact

- Wall: new `wall/src/ui/pet.rs` (sprites, mood, movement, chatter
  tables); `ui/strip.rs` (pet span in the filler + its column range for
  click routing); `state/wall_state.rs` + `state/store.rs` (`pet`
  choice, `daemon_live` flag); `state/persistence.rs` (`pet` field in
  `wall.json`, schema stays v1 — field is optional); `runtime.rs` (300 ms
  pet tick, `P` binding, click routing, chatter scheduling into
  `Notices`, `AppEvent::Connection`); `ui/help.rs` (`P` row).
- Daemon: none.
- Web UI: none (Arthur/Papito/Segan names and personalities are shared by
  convention, not by code — the web `pet.js` is JS, the TUI is Rust).
