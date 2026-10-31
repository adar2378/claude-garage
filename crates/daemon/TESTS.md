
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
