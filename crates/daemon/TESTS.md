
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
