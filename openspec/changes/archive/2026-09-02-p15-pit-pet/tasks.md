# Tasks: p15-pit-pet

## 1. Pet core (`wall/src/ui/pet.rs`)

- [x] 1.1 `Species` (Cat/Duck/Pup, names, `cycle()`, `from_str`/`as_str`) and `Mood` (Sleep/Watch/Alert/Box/Celebrate)
- [x] 1.2 Sprite table: one-row sprites per species × mood × frame, with `bang` flag; width via `unicode-width`; `GARAGE_PET_ASCII` fallback for the pup face
- [x] 1.3 `mood(daemon_live, sessions) -> Mood` (box > sleep-empty > alert > watch > sleep) + tests
- [x] 1.4 `PetSim::step(mood, filler_width, rng, now_ms)`: alert → target 0; cat aloof 40%; duck waddle offset; pup zoomies on celebrate; celebrate window per species; returns `changed: bool` + tests (duck never blinks, pup blinks, cat ignores strolls)
- [x] 1.5 Chatter tables (`Intent × Species → &[&str]`), `Chatter::next_line(ctx, rng)` with 3-min floor, alert suppression, uptime weighting, late-night window, usage ≥80% once per window, box→live "you're back", proud-on-celebrate exemption + tests

## 2. State + persistence

- [x] 2.1 `WallState.pet: Species option` and `daemon_live: bool` (default true); `store.set_pet`, `store.connection(live)`
- [x] 2.2 `persistence.rs`: `WallFile { views, pet }`; write `pet` when not off; lenient read; round-trip tests; existing view tests unchanged
- [x] 2.3 `AppEvent::Connection(bool)` emitted from `spawn_sse_task` on connect/drop; handled in the state loop

## 3. Strip + click routing

- [x] 3.1 `strip_line`: insert pet spans in the filler (left-anchored at `sim.x`, amber `!`), hide when filler too narrow; `StripLine.pet: Option<Range<u16>>`; tests at 80 and 200 cols
- [x] 3.2 `runtime.rs`: keep `pet_cols` like `badge_cols`; click on pet → `jump_to_longest_waiting` when alert, else petting (celebrate 1.2 s + petting reply)
- [x] 3.3 `Notices`: `NoticeKind`, `show_pet` refuses over a live user notice; user `show` always overrides; tests

## 4. Runtime wiring

- [x] 4.1 300 ms pet tick in the loop's tick pass; dirty only on change; no work when off
- [x] 4.2 `P` in the garage layer cycles species, persists, shows arrival notice ("Arthur the shop cat is on the wall" / "Papito…" / "Segan…" / "the strip is quiet again")
- [x] 4.3 Chatter scheduling in the same tick: build `ChatterCtx` (uptime, local hour, usage, mood, blocked transition) and route the line through `show_pet`
- [x] 4.4 `help.rs`: `("P", "cycle the pit pet: Arthur / Papito / Segan / off")`

## 5. Voice copy (user reviews before merge)

- [x] 5.1 Draft 2–3 lines per intent per species (Hydrate, Encourage, Proud, Stretch, LateNight, Usage, Back, Petting) in `pet.rs`
- [x] 5.2 Show the full table to the user; apply edits

## 6. Docs + verification

- [x] 6.1 README: "The pit pet" section gains a TUI paragraph (`P`, one-row sprites, chatter); `ui/src/lib/pet.js` header comment points at `pet.rs`
- [x] 6.2 CHANGELOG entry; `cargo test` and `cargo clippy` green
- [x] 6.3 E2E in Ghostty against real sessions: `P` cycles all four, sleep/watch/alert/celebrate observed via a real Notification hook, alert click jumps, petting replies, one chatter line observed (temporarily lower the floor via env `GARAGE_PET_CHATTER_MS`), daemon stop/start boxes and un-boxes, quit clean; record in `verification.md`
