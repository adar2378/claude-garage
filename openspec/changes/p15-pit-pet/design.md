# Design: p15-pit-pet

## Context

The web wall's pet (`ui/src/lib/pet.js`, `PitPet.jsx`) is a 3-line `<pre>`
positioned above the footer, ticked every 300 ms outside React state. The
TUI wall (Rust/ratatui) has a one-row bottom strip (`ui/strip.rs`) built
as `left tabs | filler | notice · usage · badge · chip`, a render cap of
~30 fps driven by dirty-marking, a `Notices` slot with TTL, click routing
that reuses the strip's painted column ranges (`badge_cols`), `wall.json`
persistence (schema v1, views only), and an SSE task that reconnects
internally but does not surface connection state to `WallState`.

## Goals / Non-Goals

**Goals:**
- Same three pets, same names, same personalities, same derived-mood
  rules as the web spec, in one strip row.
- Chatter that feels like the pet, not a notification system: rare,
  in-character, always second to real attention signals.
- Zero cost when off: no tick, no allocations, no dirty frames.

**Non-Goals:**
- Multi-row art or a reserved pet band (would steal grid rows on small
  terminals; the strip filler is free real estate).
- A separate chatter toggle (pet on = voice on; keeps `P` the only knob).
- Sharing pet data between web and TUI (JS vs Rust; the roster is small
  and hand-ported; the web `pet.js` gains a comment pointing at
  `pet.rs` so the two are edited together).
- Reduced-motion: no OS signal in a terminal; the web's rule has no TUI
  analogue.
- The tmux `extended-keys-format csi-u` daemon fix (Alt+Enter through
  tmux). Real, but unrelated; lands as its own patch.

## Decisions

1. **Pet lives in the strip filler, not its own row.**
   Rendered as spans inserted into `strip_line` between `left` and
   `right`; `StripLine` gains `pet: Option<Range<u16>>` exactly like
   `badge`, so click routing uses painted columns. If `filler <
   sprite_width + 2`, the pet is skipped for that frame (spec "Narrow
   strip hides the pet"). Alternative considered: 3-row band above the
   strip when on — rejected, costs grid rows and needs layout tests to
   change.

2. **`pet.rs` is a pure module.** `Species`, `Mood`, `sprite(species,
   mood, frame) -> (&str, bool /*bang lit*/)`, `PetSim { x, target,
   frame, tick, celebrate_until, prev_blocked }` with `fn step(&mut self,
   mood, filler_width, rng, now_ms)`, `fn mood(state) -> Mood`, and the
   chatter tables + `Chatter { last_said_ms, said_usage: [bool;2],
   was_box }` with `fn next_line(&mut self, ctx) -> Option<&'static
   str>`. No ratatui, no IO, no `Instant` — time and randomness injected
   so every rule unit-tests. Same discipline as `Notices`.

3. **Tick: piggyback on the existing 100 ms poll loop.** `App` keeps
   `pet_next_tick_ms`; in the loop's tick pass, if pet on and `now >=
   next`, call `PetSim::step` and mark dirty only when the sprite or x
   changed. Off → the branch is one `if`. Alternative: a tokio interval
   sending `AppEvent::PetTick` — more plumbing for the same result.

4. **Connection state becomes explicit.** `spawn_sse_task` emits
   `AppEvent::Connection(bool)` on `on_connected` / disconnect (using
   the `SseReconnectMachine` it already owns); `WallState.daemon_live:
   bool` (default true so a slow first connect doesn't box the pet
   before the first fetch; box only after an observed drop). Only the
   pet reads it today; the rail's connection chip may adopt it later.

5. **Persistence: optional `pet` field, schema stays v1.** `serialize`
   writes `"pet": "cat"|"duck"|"pup"` only when not off; `deserialize`
   reads it leniently (unknown → off). No version bump: old wall.json
   files are valid, and old wall builds ignore the field. The
   `ViewsByWorkspace`-only API becomes a small `WallFile { views, pet }`
   struct; callers adjust.

6a. **The pet is homed at the right end of the filler and speaks beside
   itself.** First cut placed the pet at the left (by the tabs) and put
   chatter in the right-aligned notice slot; on a wide terminal that's
   ~100 columns apart and nobody noticed either. Now: home = right end of
   the filler (next to the badge/keys chip, where eyes already go), idle
   strolls stay within the right 40%, alert still runs left to the rail;
   `Pet`-kind notices render as `PetRender.say` immediately left of the
   sprite (right of it when there's no room on the left; dropped for the
   frame if neither fits). `User`-kind notices keep the slot.

6. **Chatter goes through `Notices`, tagged.** `Notices` gains
   `show_pet(text, ttl, now)` that refuses when a non-pet notice is
   active (`kind: NoticeKind::{User, Pet}`); a later user notice always
   overrides a pet one. Rate limit and triggers live in `Chatter`, not
   `Notices`. The line renders in the normal notice slot (FG color) —
   the sprite next to it is the attribution; no name prefix.

7. **`P` binding.** Garage layer only (engaged keys are byte-forwarded
   and must stay so). Shift+P is unused by the wall and by Claude Code's
   garage-layer expectations; `p` lowercase stays free for later.

8. **Chatter content is data.** `const LINES: &[(Intent, Species,
   &[&str])]` in `pet.rs`; adding a line is a one-row edit. Intents:
   Hydrate, Encourage, Proud, Stretch, LateNight, Usage, Back, Petting.
   Selection: filter by species and currently-enabled intents, weight
   (Stretch/Hydrate ×3 after 60 min uptime), pick with the injected rng,
   avoid repeating the last line.

## Risks / Trade-offs

- [Strip crowding on narrow terminals (many workspaces + long notice)] →
  pet hides itself per frame; nothing else moves. Test at 80 cols.
- [Chatter reads as nagging] → 3-min floor, 4 s TTL, alert-suppressed,
  never overrides a user notice. Tunable constants at the top of
  `pet.rs`.
- [`daemon_live` default `true` shows a watching pet during a dead-daemon
  startup] → main already health-checks the daemon before entering the
  TUI, so the wall cannot start against a dead daemon; only drops are
  observable, which matches the flag's semantics.
- [Unicode sprites (`ᴥ`, `°`, `·`) misrender on some fonts] → each
  sprite is measured with `unicode-width` (already a ratatui dep) and
  the pup's face has an ASCII fallback `(o.o)` behind a
  `GARAGE_PET_ASCII=1` env var; default stays the web's glyphs (Ghostty,
  iTerm, kitty render them).
- [300 ms tick keeps the process from being fully idle] → only when the
  pet is on, and only marks dirty on visual change; sleeping pets change
  every other tick (the `z`), so ~2 redraws/s of one strip row.

## Migration Plan

Additive. Ship in the next patch release. Rollback = the `wall.json`
`pet` field is ignored by older builds.

## Open Questions

- None blocking. Line copy is drafted in tasks 5.x and reviewed by the
  user before merge (they named the pets; they should approve the voices).
