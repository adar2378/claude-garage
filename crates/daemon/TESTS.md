
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
# Ported tests (JS -> Rust)

## Lane C: telemetry

Fake `TelemetryDeps` (token, set_status, session resolution) and temp dirs for `claude_home`; no tmux, no `claude`, no real `~/.claude`/`~/.garage`.

| JS test | Rust test |
|---|---|
| hooks.test.js: merge preserves unrelated settings and hooks | `api::hooks::tests::merge_preserves_unrelated_settings_and_hooks` |
| hooks.test.js: merge is idempotent | `api::hooks::tests::merge_is_idempotent` |
| hooks.test.js: idle reminder is noise | `api::hooks::tests::idle_reminder_is_noise_blockers_and_unknowns_are_not` |
| statusline.test.js: clampPercentage | `state::context::tests::clamp_percentage_clamps_and_rejects_non_numbers` |
| statusline.test.js: get/set/drop context, rate limits (5 tests) | `state::context::tests::*` |
| statusline.test.js: 401, ignored, resolvable (cwd fallback via fake resolver), model window, rate-only, snippet | `api::statusline::tests::*` |
| statusline.install.test.js: 6 sequential tests | `api::statusline::tests::install_flow_create_idempotent_corrupt_chain_rechain_and_fresh` (one sequence; runs the generated wrapper with `sh`) |
| transcript.test.js: all 18 | `transcript::tests::*` (TTL test uses an injected clock instead of mocked `Date`) |
| usage.test.js: 4 tests | `api::statusline::tests::usage_is_null_then_reflects_and_merges_posts` |
| token-auth.js (no JS test) | `security::token_tests::matches_only_the_exact_token` |
| settings-install.js / claude-home.js (no JS test) | `install::tests::*` |

New (no JS equivalent): hook route auth (401 incl. repeated `token`), ignored events, idle-reminder filter, Notification/Stop status calls with three-valued message, malformed body 400, hook snippet, hook install (create, idempotent, backup, 422).

Not ported: the "real tmux" variants (session creation in statusline.test.js); session resolution is lane B and is faked here.
## Lane E: launcher (`crates/cli`)

`bin/garage.js` has no JS tests; these Rust tests guard its behaviour. All run against a scripted `Host` fake (no real daemon, kill, tmux, port or PATH lookup).

| `bin/garage.js` | Rust | Guarded by (`crates/cli/src/launcher.rs` unless noted) |
|---|---|---|
| `requireBinary`, `checkPrerequisites` | `launcher::check_prerequisites` | `missing_claude_fails_but_a_binary_that_merely_errors_counts_as_present`, `missing_tmux_without_a_terminal_fails_with_the_brew_hint_and_starts_nothing` |
| `offerTmuxInstall` | `launcher::offer_tmux_install` | `missing_tmux_on_a_tty_installs_only_after_an_explicit_yes`, `tmux_install_is_not_offered_without_homebrew_and_a_failed_install_is_fatal` |
| `fetchHealth`, `healthOk` | `Host::fetch_health` (real: `host.rs`) | `stale_means_version_differs_or_is_missing` (the `{}` body case) |
| `daemonIsStale` | `launcher::daemon_is_stale` | `stale_means_version_differs_or_is_missing` |
| `pidListeningOnPort` | `Host::listener_pid` + `parse_listener_pid` | `listener_pid_must_be_exactly_one_distinct_pid_above_one`, `a_pre_upgrade_daemon_without_pid_or_version_falls_back_to_the_port_listener` |
| `stopStaleDaemon`, `waitForPortFree` | `launcher::stop_stale_daemon` | `a_stale_daemon_is_stopped_by_the_pid_it_reports_then_replaced`, `a_reported_pid_of_one_is_never_signalled`, `an_unknown_pid_stops_the_launch_and_signals_nobody`, `an_already_gone_stale_daemon_is_not_an_error`, `a_failed_signal_aborts_before_a_second_daemon_can_start`, `a_port_that_never_frees_aborts_after_the_10s_budget` |
| `startDetachedDaemon`, `waitForHealth` | `Host::spawn_daemon` (re-execs `claude-garage daemon`, own process group, log in `GARAGE_DIR`), `launcher::wait_for_health` | `a_daemon_that_never_comes_up_reports_the_log_path_after_15s` |
| `tuiMain` | `launcher::tui_main` | `no_daemon_starts_one_detached_waits_for_health_then_runs_the_tui`, `a_missing_tui_binary_fails_fast_without_a_side_effect_daemon`, `a_current_daemon_is_reused_untouched`, `the_tui_exit_code_is_passed_through` |
| `resolveTuiBinary`, `wallSourcesNewerThan`, `newestMtimeUnder`, `cargoBuildTuiBinary` | `tui_bin::resolve` | `tui_bin::tests::*` (override, stale rebuild for each watched input, no-cargo warning, build once, failed build, error texts) |
| `waitForHealthChanged` | `launcher::wait_for_health_changed` | `restart_does_not_accept_the_old_process_or_a_stale_version_as_done` |
| `restartSessionsCLI` | `launcher::restart_sessions` | `restart_sessions_sends_all_true_and_force_only_with_all_and_prints_the_summary`, `restart_sessions_reports_the_daemons_error_body_or_the_status` |
| `restartMain` | `launcher::restart_main` | `restart_asks_the_running_daemon_to_swap_and_waits_for_a_new_pid_and_matching_version`, `restart_surfaces_a_daemon_that_predates_the_endpoint`, `restart_with_no_daemon_just_starts_one` |

Deviations from the JS: `garage-wall` beside the running binary is used when there is no `wall/` checkout (installed layout); `help`/unknown-subcommand text and exit code (2) come from clap; `ROOT` is found by walking up from the executable.

## Lane F: workspaces, worktrees, editor, picker, visibility, restart

Fake `Os` (`os::fake::FakeOs`) for every notification, picker and editor launch; temp `GARAGE_DIR`s; real tmux only through `TestTmux`; scratch git repos with repo-local identity. Route tests live in `src/lane_f_tests.rs`, git tests in `git::tests`, pure OS pieces in `os::{mod,macos,other,notify}::tests`.

| JS test | Rust test |
|---|---|
| worktrees.finish.test.js: currentBranch returns the checked-out branch, null for a non-repo | `git::tests::current_branch_returns_the_checked_out_branch_null_for_a_non_repo` |
| worktrees.finish.test.js: dirty worktree refuses merge with 409 | `git::tests::dirty_worktree_refuses_merge_with_409` |
| worktrees.finish.test.js: merge conflict -> 409 with stderr, merge aborted, worktree and branch kept | `git::tests::merge_conflict_is_409_with_stderr_merge_aborted_worktree_and_branch_kept` |
| restart.test.js: returns as soon as the probe reports the predecessor gone | `lane_f_tests::wait_returns_as_soon_as_the_probe_reports_the_predecessor_gone` |
| restart.test.js: returns immediately when the predecessor is already gone | `lane_f_tests::wait_returns_immediately_when_the_predecessor_is_already_gone` |
| restart.test.js: throws, naming the port and pid, once the deadline passes | `lane_f_tests::wait_fails_naming_port_and_pid_once_the_deadline_passes` |

Not ported here (lane D, sessions): `planRestartTargets` (6), `needsPollBeforePlan` (3), `spawnClaudeResumed` (1) from `restart.test.js`.

Rust-only tests (the JS daemon has no tests for these routes):

- workspaces: `put_workspace_validates_name_and_dir_then_registers`, `get_workspaces_lists_name_dir_and_resolved_branch`, `patch_workspace_errors_unknown_bad_name_and_conflict`, `patch_workspace_renames_live_sessions_registry_and_resume_metadata` (real tmux; `ws` vs `ws2` prefix), `patch_workspace_reports_failed_session_renames_and_still_renames_the_registry`, `delete_workspace_plain_is_204_drops_metadata_and_never_touches_tmux`, `delete_workspace_with_sessions_kill_kills_only_that_workspaces_sessions`.
- worktrees: `git::tests::{clean_merge_*, discard_*, keep_*, merge_and_discard_refuse_a_non_garage_branch_*, create_worktree_*}`, `lane_f_tests::finish_*` (validation order, 400 guard, keep echo, 409/500 bodies).
- editor: `resolve_folds_dot_segments_*`, `open_editor_*` (404, path escape 400, dir / `--goto` args, `line ?? 1`, 501 / 500).
- picker: `pick_directory_*` (route), `os::macos::tests::picker_*` (script, `User canceled`, stderr), `os::tests::trailing_slash_*`.
- visibility and notifications: `visibility_requires_client_id_*`, `api::visibility::tests::*` (45 s TTL with an injected clock), `os::notify::tests::*` (edge rule, suppression, unsupported swallowed, bus subscription), `os::macos::tests::{terminal_notifier_*, osascript_fallback_*}`.
- daemon restart: `restart_spawns_the_successor_*` (stand-in `sh` successor, pid handoff env, `daemon.log`), `restart_with_an_unspawnable_successor_*`, `the_default_successor_keeps_the_listening_port`, `default_probe_*`.
