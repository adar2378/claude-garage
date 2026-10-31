
## Lane A: state (`state/status.rs`, `state/registry.rs`)

JS `daemon/test/status.test.js` (13 tests) -> `state::status::tests`:

| JS test | Rust test |
|---|---|
| unknown sessions read as idle | `unknown_sessions_read_as_idle` |
| setStatus stores and getStatus reads | `set_stores_and_get_reads` |
| transition events fire once per actual change | `transition_events_fire_once_per_actual_change` (bus instead of `statusEvents`) |
| done holds indefinitely / needs-input never auto-decays | `done_and_needs_input_hold_with_no_auto_decay` (merged; Rust has no timers to mock) |
| done only clears via an explicit setStatus transition | `done_only_clears_via_an_explicit_transition` |
| dropSession forgets state | `drop_session_forgets_state` |
| getStatusEntry defaults to idle/null | `entry_defaults_to_idle_null_for_unknown_ids` |
| hook-set message rides needs-input | `hook_set_message_rides_needs_input` |
| message clears on any transition away... | `message_clears_on_transition_away_and_never_resurrects` |
| messageless needs-input transition leaves message unset | `messageless_needs_input_transition_leaves_message_unset` |
| messageless same-state write must not wipe a hook-set message | `messageless_same_state_write_keeps_hook_message` |
| message never sticks to non-blocked states | `message_never_sticks_to_non_blocked_states` |
| since changes on a real transition... | `since_moves_on_real_transition_and_is_kept_on_repeat` (explicit `set_at` clock) |

New: `transition_event_carries_the_new_since`; `apply_agent_status_maps_and_never_downgrades_blocked_or_done` (`applyAgentStatus` from `poller.js`, covered in JS only by poller tests).

Registry: the JS suite has no registry unit tests (it is exercised through `sessions.*` route tests, which belong to lane D). Rust-only unit tests in `state::registry::tests` (11) cover lazy file creation, malformed file, atomic write/no temp leftovers, unknown-key preservation, `upsertSessionMeta` field-level merge and no-op write, rename, remove (prefix-exact), hook token.
## Lane B: backend (tmux, poller, session_resolve, git)

JS to Rust test mapping. Rust names are the snake_case of the JS title unless noted.

| JS test | Rust test |
| --- | --- |
| `tmux.test.js` parsePaneLine x3 | `backend::tmux::tests::parse_pane_line_*` (3) |
| `tmux.test.js` normalizeTitle x7 | `backend::tmux::tests::normalize_title_*` (7) |
| `poller.test.js` busy/waiting/idle, coarse idle x2, busy clears needs-input | `poller::tests::{busy_maps_to_working, waiting_maps_to_needs_input, idle_maps_to_idle_*, coarse_idle_must_not_stomp_*, busy_clears_needs_input}` (6) |
| `poller.test.js` "poller-sourced needs-input carries no message", "same-state tick must not clear a hook-set message" | not ported here: they assert status-store message retention (lane A, `state::status`) |
| `poller.test.js` diffSessionState x6 | `poller::tests::diff_*` (6) |
| `sessions.title.test.js` "a live session's set pane title surfaces normalized" | `backend::tmux::tests::live_session_set_pane_title_surfaces_normalized` (backend half; the route half is lane D) |
| `sessions.title.test.js` "default title has title: null" | `backend::tmux::tests::live_session_with_default_title_normalizes_to_none` (backend half) |
| `sessions.title.test.js` "restorable entries always have title: null" | not ported here: route-level (lane D) |

New tests with no JS counterpart: `TmuxBackend` against a real isolated server (lifecycle, status off + extended keys idempotence, env/args passthrough, duplicate name, respawn keeps session, panes, capture), argv with and without `GARAGE_TMUX_SOCKET`, `poll_once` with fakes (pid join, cwd fallback, death/title events), `Agent::parse_list`, `resolve_session_ids` (4), `git::resolve_branch` (4).

`TestTmux` lives in `src/test_support.rs` (`crate::test_support::TestTmux`; other crates enable the `test-support` feature).
