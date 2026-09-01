# Verification: p15-pit-pet

Date: 2026-09-02. Binary: `wall/target/release/garage-wall` built from the
working tree. Daemon 0.3.3 live on :4747 with one real session
(`garage/cto-playground/claude-2`). Terminal: 200×45.

## Automated

- `cargo test` (wall): 440 lib tests + 1 tmux integration test, green.
- `cargo clippy --all-targets`: one pre-existing warning in
  `ui/registry.rs`, none introduced.

## Live, inside tmux (`tmux send-keys` + `capture-pane`)

- Boot with no `pet` in `wall.json`: strip filler empty, no chatter.
- `P` ×4: `=-.-=  z` "Arthur the shop cat is on the wall" → `<(- )___ z`
  "Papito the rubber duck is on the wall" → `(-ᴥ-)  z` "Segan the pit pup
  is on the wall" → empty "the strip is quiet again". One blank gutter
  column between the tabs and the sprite (fixed during verification).
- Persistence: `P` → quit → `wall.json` has `"pet": "cat"` → reboot shows
  Arthur on the first frame (isolated `GARAGE_DIR`).
- `?` overlay lists `P  cycle the pit pet: Arthur / Papito / Segan / off`.
- Motion: sleeping `z` alternates every other tick; idle strolls observed
  (pup at x=24 → 36 → 48 → 60 across 3 s samples).
- Real `Notification` hook (`POST /api/hooks/claude`): sprite → `(°ᴥ°) !`,
  `● 1 blocked` badge, amber tab dot; a chatter line showing at the
  moment of the alert is cleared (fixed during verification); pet moves
  left ~4 cols/s toward the rail.
- Real `Stop` hook: `\(^ᴥ^)/` celebrate + "best human ever!! proud!!"
  (Proud intent, floor-exempt).
- Chatter with `GARAGE_PET_CHATTER_MS=1000`: "STRETCH BREAK!! wiggle
  time!!", "still up?? okay but sleep after!!" (local 03:xx → late-night
  intent enabled), "WATER BREAK!! then back to it!" — lines rotate, none
  repeats back-to-back. Cadence is now derived from the floor
  (mean spacing ≈ 2× floor; default 3 min floor → ~6 min).
- `q`: EXIT=0 every run.

## Live, raw PTY (python `pty.fork`, clicks as SGR bytes)

`tmux send-keys` cannot inject mouse events into a pane (tmux parses them
as its own mouse keys, even with `mouse on`), so clicks were verified on a
direct PTY:

- Petting click on the sleeping pup: `(^ᴥ^)` + "*happy tail noises*" /
  "YES pet me more!!". A petting celebrate no longer triggers a Proud
  line (fixed during verification).
- Alert click after a real `Notification` hook: focus jumps and lands
  engaged (keys chip leaves `garage`), exactly the `a` landing.
- Tile click still engages; badge routing unaffected.

## Follow-up (same day): speech beside the pet

User feedback: pet at the left and its line in the right-aligned slot
were too far apart to notice. Re-verified at 160×40 after homing the pet
at the right end and drawing `Pet` notices beside the sprite:
`KEEP GOING!! you got this!!  (-ᴥ-) z  keys → garage`. User notices still
take the slot (`the strip is quiet again`). 443 tests green.

## Not verified here

- **Box mood on daemon drop** — needs the daemon stopped, which would
  disconnect the user's live wall; the transition is covered by
  `pet::mood` unit tests and the `AppEvent::Connection` wiring test.
  Human check: `kill` the daemon with the pet on → `[·ᴥ·]`; restart →
  "YOU'RE BACK!! i missed you!!" once.
- **Voice copy sign-off** (task 5.2): table shown to the user; awaiting
  edits.
- **Old builds share `wall.json`**: a pre-p15 wall writing the file drops
  the `pet` field (observed once when an older build quit mid-session).
  Documented risk in design.md; harmless after upgrade.
