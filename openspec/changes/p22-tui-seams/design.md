## Context

Today's input path, all in `wall/src/runtime.rs` unless noted:

- `App::apply` (line ~1830) is the single state loop's event match. Key events go to `App::handle_key` (~1941), which splits by layer: overlay → `handle_overlay_key` (~989) plus per-overlay helpers (`handle_view_picker_key` ~1109, `handle_worktree_finish_key` ~1189); engaged → `handle_engaged_key` (~2034); garage → `GarageRouter::on_garage_key` (~478).
- `GarageRouter` (~432) owns arming state (`armed_close`, `armed_remove`, `armed_restart`, `restart_prefix`), busy guards (`spawning`, `installing`, `restoring`) and `Notices`. It turns a key string into `GarageCommand` via `store::garage_command_for` / `restart_command_for` (`state/store.rs` ~107 / ~145), then either calls `WallStore::dispatch` (~778) or returns `Effect`s.
- `Effect` (~331) is the IO list; `run_effect` (~1406) performs it on a blocking task and reports back as an `AppEvent`.
- Mouse: `App::handle_mouse` (~2119) → `handle_click` (~2151), `handle_pet_click` (~2303), `handle_view_picker_click` (~2473), `handle_wheel` (~2516), using `ui/hit_targets.rs` (`tile_index_at`, `rail_target_at`, `triage_row_index_at`, `view_picker_row_index_at`).
- Rendering: `App::draw_frame` (~2600) and `draw_tile` (~2749) call `ui/tile.rs::render_tile`/`title_line`, `ui/rail.rs::render_rail`, `ui/strip.rs::render_strip`/`strip_line`, plus overlay renderers.
- The pit pet is spread through `App`: fields `pet_sim`, `pet_chatter`, `pet_mood` (~1788–1808), `handle_pet_click`, `pet_tick` (~2370), the `xorshift_next` PRNG (~186), and `strip::PetRender`.

`GarageCommand` (store.rs ~30) is already a proto-action list: 26 variants covering focus, jump, spawn, close, restore, views, overlays, install, restart, quit. It has no stable string ids, no context payload, and only the garage layer uses it.

## Goals / Non-Goals

**Goals:** one named list of wall behaviours; keys and mouse resolve through it; named UI slots that core already renders through; `runtime.rs` and `store.rs` split so no file is over ~1,200 lines; the pet behind a boundary; zero visible change.

**Non-Goals:** a user keymap file (later change), plugin access to actions or slots (p24), animations (p23), changing any binding or wording, changing the daemon or API.

## Decisions

**1. `Action` = stable string id + typed payload.** A new `wall/src/actions/` module defines `ActionId` (a `&'static str` newtype, dotted names) and `ActionCtx { workspace, session, index, selection, worktree, source }` (`index` carries the `1`–`9` workspace number for `workspace.focus`), where `source` is `Key(String)` or `Mouse { col, row }`. `GarageCommand` variants map 1:1 onto ids:

| id | today |
|---|---|
| `workspace.focus` (index in the context) | `FocusWorkspace(n)` |
| `focus.cycle.prev` / `focus.cycle.next` | `CycleFocus(±1)` |
| `jump.blocked` | `Jump` |
| `overlay.open.queue`, `overlay.open.help`, `overlay.open.view_picker`, `overlay.open.workspace_add` | `OpenQueue`, `ToggleHelp`, `OpenViewPicker`, `WorkspaceAdd` |
| `session.spawn`, `session.spawn.worktree` | `Spawn { worktree }` |
| `session.engage`, `session.maximize`, `session.close`, `session.restore_all`, `session.detach`, `session.open_window` | matching variants |
| `session.restart`, `workspace.restart`, `daemon.restart` | `Restart*` |
| `view.cycle`, `workspace.remove`, `workspace.remove.kill`, `hooks.install`, `app.quit` | matching variants |
| `session.focus`, `session.restore`, `pet.pet`, `pet.cycle_species` | today only reachable by click or internal paths |

Ids are the contract p24 exposes. Alternative: keep the enum and add plugin hooks later. Rejected: plugins and a config file need string names anyway, and doing it now keeps one routing path.

**2. Arming stays in the router, not in the keymap.** Double-press arms (`x x`, `X X`, `r r`), the `X`→`K` kill confirm and the one-shot `r` prefix are behaviour, not bindings. The keymap resolves a key (or a chord prefix + key) to an id; `GarageRouter` keeps its arming state and decides whether `session.close` arms or confirms. The disarm rule ("any other key disarms") stays where it is, ahead of dispatch, so wording and timing are byte-identical.

**3. Keymap = ordered table, defaults in code.** `wall/src/keymap.rs` holds `Keymap { layer: Garage | Overlay(kind) | Chord(prefix), key: String, action: ActionId }`, built by `Keymap::defaults()` from the exact strings in `garage_command_for` / `restart_command_for` / the overlay `match` arms. Engaged-layer bytes are not in the keymap: engaged input stays verbatim (`input/encode.rs`, spec tui-key-routing "Verbatim byte re-encoding") apart from the existing disengage combo. A later change loads overrides from `~/.garage/config.toml` on top of these defaults.

**4. Mouse resolves to actions through hit targets.** `ui/hit_targets.rs` gains `fn action_at(layout, state, col, row) -> Option<(ActionId, ActionCtx)>` wrapping the existing `tile_index_at` / `rail_target_at` / `triage_row_index_at` / `view_picker_row_index_at`. `handle_click` becomes "resolve, then dispatch". Label rows (p18) still map to `None`.

**5. Slot registry: core renders, contributors provide segments.** `wall/src/slots/` defines `SlotId` (`tile.title.right`, `rail.row`, `rail.header`, `strip.right`, `overlay`, `context.menu`) and `SlotItem { text: Vec<Span>, on_click: Option<ActionId>, priority: i16 }`. A slot provider is `fn(&WallState, &SlotCtx) -> Vec<SlotItem>`. Core providers registered in this change:
- `tile.title.right` ← the context meter now built in `tile::title_line` (`context_meter_text`).
- `strip.right` ← the usage chip (`strip::usage_chip_text`) and the pet render.
- `rail.header` ← the p18 family label row.

Renderers ask the registry for items and lay them out by priority, truncating from the lowest priority when space runs out. Output must match today's frame byte-for-byte, which the p20 snapshots check. `overlay` and `context.menu` are defined and render nothing yet (no context menu exists today); they exist so p24 does not change the registry's shape. Alternative: add slots only when plugins arrive. Rejected: the first real user should be core, so the shape is proven before it is promised to plugins.

**6. File split.** Target layout (moved code, not rewritten):
- `runtime/mod.rs`: `run`, `App` struct, `apply`, `state_loop`.
- `runtime/effects.rs`: `Effect`, `run_effect`, notice helpers (`failure_notice`, `install_notice`, `restart_notice`, `restart_all_notice`).
- `runtime/tasks.rs`: `spawn_input_task`, `spawn_fetch_task`, `spawn_usage_task`, `spawn_sse_task`, `spawn_signal_task`, `fetch_and_send`.
- `runtime/router.rs`: `GarageRouter`, `Notices`, `NoticeKind`.
- `runtime/overlays.rs`: `handle_overlay_key` and each overlay's key and click handlers.
- `runtime/mouse.rs`: `handle_mouse`, `handle_click`, `handle_wheel`, `tile_at`.
- `runtime/draw.rs`: `draw_frame`, `draw_tile`.
- `runtime/tests/`: the existing `mod tests` (~1,560 lines) split per area.
- `state/store.rs` keeps `WallStore` and its transitions; `GarageCommand` and the key tables move to `actions/` and `keymap.rs`; the pure reconcile helpers (`reconcile`, `refocus_vacated_views`, `with_focused_workspace`, `focus_in_view`, `focus_session_state`) move to `state/reconcile.rs`.

Each move is its own commit, with tests green after each, so a bisect lands on one move.

**7. Pet behind `PetHost`.** A `ui/pet/` module exposes one `PetHost` struct (owns `PetSim`, `Chatter`, mood, PRNG) with `tick(now, &WallState) -> Option<Notice>`, `click(now)`, and a `strip.right` slot provider. `App` holds `Option<PetHost>`. A `pet` cargo feature (default on) compiles it; with the feature off the wall builds and every non-pet test passes. p25 replaces `PetHost` with a plugin that fills the same slot.

## Risks / Trade-offs

- **Silent behaviour drift during the split.** Mitigation: p20's insta snapshots of the rail, strip, tiles and each overlay; the existing 519 unit tests; one move per commit; a live check on the user's wall at the end.
- **Ids become a public promise in p24.** Names chosen here are hard to change later. Mitigation: dotted, noun-first names; a short list in the spec; p24 can still add aliases.
- **Slot layout cost.** Asking providers every frame adds allocation. Mitigation: providers are pure and cheap today (three of them); measure with the p20 render benchmark if one exists, otherwise check frame time stays flat.
- **Feature flag rot.** `--no-default-features` builds only stay working if CI builds them. Mitigation: add that build to the p20 CI matrix (one job).

## Open Questions

- Does `context.menu` need a trigger (right-click) in this change, or only the slot? Leaning: slot only; the trigger comes with the first item that needs it.
