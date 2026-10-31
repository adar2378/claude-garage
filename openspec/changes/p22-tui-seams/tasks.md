# Tasks: p22-tui-seams

## 0. Preconditions

- [x] 0.1 insta snapshots of rail, strip and tile titles (moved here from p20; overlay snapshots and three-OS CI stay with p20)
- [x] 0.2 Baseline recorded: 527 lib tests pass (519 + 8 snapshots)

## 1. Split without behaviour change (`wall/src/runtime.rs`, `wall/src/state/store.rs`)

- [x] 1.1 Move effects: `Effect`, `run_effect`, notice helpers → `runtime/effects.rs`
- [x] 1.2 Move background tasks: `spawn_*_task`, `fetch_and_send` → `runtime/tasks.rs`
- [x] 1.3 Move `GarageRouter`, `Notices`, `NoticeKind` → `runtime/router.rs`
- [x] 1.4 Move overlay key and click handlers → `runtime/overlays.rs`
- [x] 1.5 Move mouse handling (`handle_mouse`, `handle_click`, `handle_wheel`, `tile_at`) → `runtime/mouse.rs`
- [x] 1.6 Move `draw_frame`, `draw_tile` → `runtime/draw.rs`
- [x] 1.7 Split `runtime.rs` tests per area under `runtime/tests/`
- [x] 1.8 Move pure reconcile helpers out of `store.rs` → `state/reconcile.rs`
- [x] 1.9 After each step: tests and snapshots green, one commit per step

## 2. Actions and keymap

- [x] 2.1 `actions/` module: `ActionId`, `ActionCtx`, the id list from design.md decision 1, `GarageCommand` mapped onto ids
- [x] 2.2 `keymap.rs`: `Keymap::defaults()` reproducing `garage_command_for`, `restart_command_for` and the overlay key arms exactly
- [x] 2.3 `GarageRouter::on_garage_key` resolves through the keymap; arming, disarm and prefix logic unchanged
- [x] 2.4 Help and triage-queue key handlers resolve through the keymap (text-input overlays keep typing; view picker keys stay as is)
- [x] 2.5 Unit tests: every default binding resolves to its id; every id in the spec list is reachable from a key or a click; chord prefixes still one-shot

## 3. Mouse through actions

- [x] 3.1 `hit_targets::action_at` wrapping the existing hit-target functions
- [x] 3.2 `handle_click` dispatches resolved actions; family label rows still inert
- [x] 3.3 Tests: each clickable target resolves to the same id as its key

## 4. Slot registry

- [x] 4.1 (layout drops by priority only when given a width budget; the core surfaces pass none, to keep their output byte-identical) `slots/` module: `SlotId`, `SlotItem`, provider registration, priority layout with truncation
- [x] 4.2 Core providers: tile context meter → `tile.title.right`; usage chip → `strip.right`; family label → `rail.header` (the pet joins `strip.right` in section 5)
- [x] 4.3 `overlay` and `context.menu` slots defined, empty
- [x] 4.4 Snapshots unchanged byte-for-byte

## 5. Pit pet boundary

- [ ] 5.1 `ui/pet/` with `PetHost` (sim, chatter, mood, PRNG); `App` holds `Option<PetHost>`
- [ ] 5.2 `pet` cargo feature, default on; `cargo build --no-default-features` and its tests pass
- [ ] 5.3 Add the no-default-features build to CI

## 6. Verification

- [ ] 6.1 `cd wall && cargo test --lib --bins` green, same or higher count; `cargo clippy --all-targets` clean; snapshots unchanged
- [x] 6.2 No file under `wall/src/runtime/` or `wall/src/state/` over ~1,200 lines
- [ ] 6.3 Live check on the user's wall (user relaunches with `GARAGE_TUI_BIN` on the branch build; main session only, ask first): keys `1`-`9`, `[` `]`, `a`, `A`, `n`, `x x`, `X X`, `X K`, `r r`, `r a`, `?`, `m`, `d`, `D`, `Tab`; clicks on tiles, rail rows, triage rows; pet clicks
- [ ] 6.4 Record results in `verification.md`
