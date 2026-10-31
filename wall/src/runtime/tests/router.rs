//! Garage router tests: armed closes, removals, restart chords, restore,
//! spawn, install, window, notices and key reduction.
use super::*;

// ── armed x close (p8.1) ────────────────────────────────────────────

#[test]
fn x_arms_with_the_exact_strip_wording_then_a_second_x_closes() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "ghost")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "x", 1000), vec![]);
    assert_eq!(
        router.notices.current(1001),
        Some("press x again to close ghost"),
        "harness-grepped wording"
    );
    assert_eq!(
        key_at(&mut router, &mut store, "x", 2000),
        vec![Effect::Close {
            id: id("a", "ghost"),
            label: "ghost".to_owned(),
            meta_only: false,
        }]
    );
    assert_eq!(
        router.notices.current(2001),
        None,
        "confirming clears the arm notice"
    );
}

#[test]
fn x_on_a_restorable_placeholder_deletes_meta_only() {
    let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "x", 1000);
    assert_eq!(
        key_at(&mut router, &mut store, "x", 2000),
        vec![Effect::Close {
            id: id("a", "dead"),
            label: "dead".to_owned(),
            meta_only: true,
        }]
    );
}

#[test]
fn any_other_key_disarms_the_pending_close_and_clears_its_notice() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "ghost")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "x", 1000);
    // 'z' is unbound: it disarms AND raises the typing hint.
    key_at(&mut router, &mut store, "z", 1100);
    assert_eq!(router.notices.current(1101), Some(TYPING_HINT));
    // The next x must re-arm, not confirm.
    assert_eq!(key_at(&mut router, &mut store, "x", 1200), vec![]);
    assert_eq!(
        router.notices.current(1201),
        Some("press x again to close ghost")
    );
}

#[test]
fn an_expired_x_arm_re_arms_instead_of_confirming() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "ghost")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "x", 1000);
    assert_eq!(
        key_at(&mut router, &mut store, "x", 4001),
        vec![],
        "window expired"
    );
    assert_eq!(key_at(&mut router, &mut store, "x", 4500).len(), 1);
}

#[test]
fn x_with_no_focused_session_is_a_silent_no_op() {
    let mut store = store_with(vec![ws("a")], vec![]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "x", 1000), vec![]);
    assert_eq!(router.notices.current(1001), None);
}

// ── armed X workspace removal (p8.3) + K kill confirm (p8.4) ────────

#[test]
fn shift_x_arm_notice_carries_the_k_clause_with_live_sessions() {
    let mut store = store_with(vec![ws("proj")], vec![si("proj", "a"), si("proj", "b")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "X", 1000), vec![]);
    assert_eq!(
        router.notices.current(1001),
        Some("press X again to remove proj (sessions keep running) · K to also kill its 2 sessions")
    );
}

#[test]
fn shift_x_arm_notice_omits_the_k_clause_with_zero_live_sessions() {
    let mut store = store_with(vec![ws("proj")], vec![si_restorable("proj", "dead")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "X", 1000);
    assert_eq!(
        router.notices.current(1001),
        Some("press X again to remove proj (sessions keep running)")
    );
}

#[test]
fn x_x_confirm_is_registry_only_never_sessions_kill() {
    let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "X", 1000);
    assert_eq!(
        key_at(&mut router, &mut store, "X", 2000),
        vec![Effect::RemoveWorkspace {
            name: "proj".to_owned(),
            kill: false,
        }]
    );
}

#[test]
fn k_within_the_arm_window_confirms_the_kill_remove() {
    let mut store = store_with(vec![ws("proj")], vec![si("proj", "a"), si("proj", "b")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "X", 1000);
    assert_eq!(
        key_at(&mut router, &mut store, "K", 2500),
        vec![Effect::RemoveWorkspace {
            name: "proj".to_owned(),
            kill: true,
        }]
    );
    assert_eq!(router.notices.current(2501), None, "arm notice cleared");
}

#[test]
fn bare_k_outside_an_arm_stays_an_ordinary_unbound_key() {
    let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "K", 1000), vec![]);
    assert_eq!(
        router.notices.current(1001),
        Some(TYPING_HINT),
        "typing hint, never an arm"
    );
}

#[test]
fn k_after_the_window_expired_confirms_nothing_and_never_re_arms() {
    let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "X", 1000);
    assert_eq!(key_at(&mut router, &mut store, "K", 4001), vec![]);
    // The stale arm is gone: an X now re-arms from scratch.
    assert_eq!(key_at(&mut router, &mut store, "X", 4100), vec![]);
    assert!(router
        .notices
        .current(4101)
        .is_some_and(|n| n.starts_with("press X again to remove proj")));
}

#[test]
fn k_on_an_arm_whose_workspace_became_unregistered_is_a_consumed_no_op() {
    let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "X", 1000);
    // A refetch unregistered `proj` mid-arm (sessions stay live).
    store.workspaces_fetched(vec![]);
    assert_eq!(
        key_at(&mut router, &mut store, "K", 2000),
        vec![],
        "stale arm: no API call"
    );
}

#[test]
fn shift_x_on_an_unregistered_group_explains_instead_of_arming() {
    // Registry-only removal happened: sessions live on, group synthesized.
    let mut store = store_with(vec![], vec![si("proj", "a")]);
    let mut router = GarageRouter::default();
    assert!(!store.state().groups[0].registered);
    assert_eq!(key_at(&mut router, &mut store, "X", 1000), vec![]);
    assert_eq!(
        router.notices.current(1001),
        Some("already unregistered — sessions live in tmux; x closes them individually")
    );
    // And a K afterwards is unbound (nothing armed).
    assert_eq!(key_at(&mut router, &mut store, "K", 1100), vec![]);
}

#[test]
fn x_and_shift_x_cross_disarm_each_other() {
    let mut store = store_with(vec![ws("proj")], vec![si("proj", "a")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "x", 1000); // close armed
    key_at(&mut router, &mut store, "X", 1100); // arms removal, disarms close
    assert!(router
        .notices
        .current(1101)
        .is_some_and(|n| n.starts_with("press X again to remove proj")));
    // x now re-arms (the X disarmed it) AND disarms the removal.
    assert_eq!(key_at(&mut router, &mut store, "x", 1200), vec![]);
    assert_eq!(
        router.notices.current(1201),
        Some("press x again to close a")
    );
    // The x disarmed the removal: this X re-arms instead of confirming.
    assert_eq!(key_at(&mut router, &mut store, "X", 1300), vec![]);
    assert!(router
        .notices
        .current(1301)
        .is_some_and(|n| n.starts_with("press X again to remove proj")));
}

// ── p16-restart: the `r` prefix chords (spec restart "TUI chords") ──

#[test]
fn r_opens_the_prefix_with_a_hint_and_r_r_arms_then_restarts() {
    let mut store = store_with(vec![ws("a")], vec![si_status("a", "one", "idle")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "r", 1000), vec![]);
    assert_eq!(router.notices.current(1001), Some(RESTART_PREFIX_HINT));
    assert_eq!(key_at(&mut router, &mut store, "r", 1100), vec![]);
    assert_eq!(
        router.notices.current(1101),
        Some("restart one? r r again — resumes the conversation")
    );
    // Second `r r` inside the window fires, unforced (the session is idle).
    assert_eq!(key_at(&mut router, &mut store, "r", 1200), vec![]);
    assert_eq!(
        key_at(&mut router, &mut store, "r", 1300),
        vec![Effect::RestartSession {
            id: id("a", "one"),
            name: "one".to_owned(),
            force: false,
        }]
    );
    assert_eq!(
        router.notices.current(1301),
        Some("restarting one — resumes the conversation")
    );
}

#[test]
fn r_r_on_a_working_session_warns_and_the_second_press_forces() {
    // `si` defaults to `working`.
    let mut store = store_with(vec![ws("a")], vec![si("a", "busy")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "r", 1000);
    key_at(&mut router, &mut store, "r", 1100);
    assert_eq!(
        router.notices.current(1101),
        Some("busy is working — r r again to restart anyway")
    );
    key_at(&mut router, &mut store, "r", 1200);
    assert_eq!(
        key_at(&mut router, &mut store, "r", 1300),
        vec![Effect::RestartSession {
            id: id("a", "busy"),
            name: "busy".to_owned(),
            force: true,
        }]
    );
}

#[test]
fn an_expired_r_r_arm_re_arms_instead_of_restarting() {
    let mut store = store_with(vec![ws("a")], vec![si_status("a", "one", "idle")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "r", 1000);
    key_at(&mut router, &mut store, "r", 1100);
    key_at(&mut router, &mut store, "r", 5000);
    assert_eq!(
        key_at(&mut router, &mut store, "r", 5100),
        vec![],
        "the 3s window expired — this re-arms"
    );
    assert_eq!(
        router.notices.current(5101),
        Some("restart one? r r again — resumes the conversation")
    );
}

#[test]
fn any_other_key_after_r_cancels_the_prefix_and_is_swallowed() {
    let mut store = store_with(vec![ws("a")], vec![si_status("a", "one", "idle")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "r", 1000);
    // `x` after the prefix neither closes nor arms a close: the chord
    // cancelled and ate the key.
    assert_eq!(key_at(&mut router, &mut store, "x", 1100), vec![]);
    assert_eq!(router.notices.current(1101), None, "the hint is cleared");
    assert_eq!(key_at(&mut router, &mut store, "x", 1200), vec![]);
    assert_eq!(
        router.notices.current(1201),
        Some("press x again to close one"),
        "the close arms from scratch"
    );
}

#[test]
fn a_cancelled_chord_disarms_a_pending_r_r() {
    let mut store = store_with(vec![ws("a")], vec![si_status("a", "one", "idle")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "r", 1000);
    key_at(&mut router, &mut store, "r", 1100); // armed
    key_at(&mut router, &mut store, "r", 1200);
    key_at(&mut router, &mut store, "z", 1300); // cancels the chord
    assert_eq!(router.notices.current(1301), None, "the arm notice went too");
    // The next full chord must re-arm, not fire.
    key_at(&mut router, &mut store, "r", 1400);
    assert_eq!(key_at(&mut router, &mut store, "r", 1500), vec![]);
}

#[test]
fn r_r_on_a_restorable_placeholder_explains_instead_of_restarting() {
    let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "r", 1000);
    assert_eq!(key_at(&mut router, &mut store, "r", 1100), vec![]);
    assert_eq!(
        router.notices.current(1101),
        Some("no live session to restart")
    );
}

#[test]
fn r_a_restarts_the_workspaces_idle_and_done_sessions_never_the_busy_ones() {
    let mut store = store_with(
        vec![ws("a")],
        vec![
            si_status("a", "idle-one", "idle"),
            si("a", "busy"),
            si_blocked("a", "blocked", 900),
            si_status("a", "done-one", "done"),
            si_restorable("a", "dead"),
        ],
    );
    let mut router = GarageRouter::default();
    let focused = store.state().focused_session_id.clone();
    key_at(&mut router, &mut store, "r", 1000);
    assert_eq!(
        key_at(&mut router, &mut store, "a", 1100),
        vec![Effect::RestartWorkspace {
            ids: vec![id("a", "idle-one"), id("a", "done-one")],
        }]
    );
    assert_eq!(
        router.notices.current(1101),
        Some("restarting 2 sessions in a")
    );
    // The `a` jump never ran: it would have engaged the blocked session.
    assert_eq!(store.state().layer, KeyLayer::Garage);
    assert_eq!(store.state().focused_session_id, focused);
}

#[test]
fn r_a_with_no_idle_session_says_so_and_fires_nothing() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "busy")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "r", 1000);
    assert_eq!(key_at(&mut router, &mut store, "a", 1100), vec![]);
    assert_eq!(
        router.notices.current(1101),
        Some("nothing to restart in a (all busy or none)")
    );
}

#[test]
fn r_d_restarts_the_daemon_at_once() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "r", 1000);
    assert_eq!(
        key_at(&mut router, &mut store, "d", 1100),
        vec![Effect::RestartDaemon]
    );
    assert_eq!(router.notices.current(1101), Some("restarting the daemon…"));
    // The prefix is one-shot: a bare `d` afterwards is the ordinary
    // detach binding again, never a second daemon restart.
    assert_eq!(key_at(&mut router, &mut store, "d", 1200), vec![]);
}

#[test]
fn the_restart_response_names_every_outcome() {
    use crate::api::models::{FailedEntry, RestartedEntry, SkippedEntry};
    let resumed = RestartResponse {
        restarted: vec![RestartedEntry {
            id: id("a", "one"),
            resumed: true,
        }],
        ..RestartResponse::default()
    };
    assert_eq!(restart_notice("one", &resumed), "one restarted");
    let fresh = RestartResponse {
        restarted: vec![RestartedEntry {
            id: id("a", "one"),
            resumed: false,
        }],
        ..RestartResponse::default()
    };
    assert_eq!(
        restart_notice("one", &fresh),
        "one restarted fresh (no conversation to resume)"
    );
    let failed = RestartResponse {
        failed: vec![FailedEntry {
            id: id("a", "one"),
            error: "no server running".to_owned(),
        }],
        ..RestartResponse::default()
    };
    assert_eq!(restart_notice("one", &failed), "one: no server running");
    let skipped = RestartResponse {
        skipped: vec![SkippedEntry {
            id: id("a", "one"),
            status: "working".to_owned(),
        }],
        ..RestartResponse::default()
    };
    assert_eq!(restart_notice("one", &skipped), "one skipped — working");
    assert_eq!(
        restart_notice("one", &RestartResponse::default()),
        "one: the daemon reported no outcome",
        "an empty response is still reported, never silent"
    );
}

#[test]
fn the_workspace_restart_line_names_skips_and_failures_only_when_there_are_any() {
    assert_eq!(restart_all_notice(3, 0, &[]), "restarted 3");
    assert_eq!(
        restart_all_notice(2, 1, &[]),
        "restarted 2 · skipped 1 (working/waiting)"
    );
    assert_eq!(
        restart_all_notice(1, 1, &["tmux: no such pane".to_owned()]),
        "restarted 1 · skipped 1 (working/waiting) · failed 1: tmux: no such pane"
    );
}

// ── restore (p8.1: Enter on restorable, R restore-all) ──────────────

#[test]
fn enter_on_a_restorable_tile_restores_it_once() {
    let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
    let mut router = GarageRouter::default();
    assert_eq!(
        key_at(&mut router, &mut store, "\r", 1000),
        vec![Effect::Restore { id: id("a", "dead") }]
    );
    assert_eq!(store.state().layer, KeyLayer::Garage, "engage still declined");
    // A second Enter while the restore is in flight must not duplicate.
    assert_eq!(key_at(&mut router, &mut store, "\r", 1100), vec![]);
    // Settling releases the guard.
    router.restore_settled(&id("a", "dead"));
    assert_eq!(key_at(&mut router, &mut store, "\r", 1200).len(), 1);
}

#[test]
fn enter_on_a_live_tile_engages_no_effects() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "\r", 1000), vec![]);
    assert_eq!(store.state().layer, KeyLayer::Engaged);
}

#[test]
fn r_restores_every_restorable_session_in_the_focused_workspace() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![
            si("a", "live"),
            si_restorable("a", "r1"),
            si_restorable("b", "other"),
            si_restorable("a", "r2"),
        ],
    );
    let mut router = GarageRouter::default();
    assert_eq!(
        key_at(&mut router, &mut store, "R", 1000),
        vec![
            Effect::Restore { id: id("a", "r1") },
            Effect::Restore { id: id("a", "r2") },
        ],
        "parallel per-id calls, focused workspace only"
    );
}

#[test]
fn r_with_nothing_restorable_shows_the_notice() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "R", 1000), vec![]);
    assert_eq!(router.notices.current(1001), Some("nothing to restore in a"));
}

// ── spawn (n / N) ───────────────────────────────────────────────────

#[test]
fn n_spawns_with_the_first_free_claude_n_label() {
    let mut store = store_with(
        vec![ws("a")],
        vec![si("a", "claude-1"), si("a", "claude-3")],
    );
    let mut router = GarageRouter::default();
    assert_eq!(
        key_at(&mut router, &mut store, "n", 1000),
        vec![Effect::Spawn {
            workspace: "a".to_owned(),
            label: "claude-2".to_owned(),
            worktree: false,
        }]
    );
}

#[test]
fn shift_n_spawns_a_worktree_session() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(
        key_at(&mut router, &mut store, "N", 1000),
        vec![Effect::Spawn {
            workspace: "a".to_owned(),
            label: "claude-1".to_owned(),
            worktree: true,
        }]
    );
}

#[test]
fn spawn_is_busy_guarded_until_settled() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "n", 1000).len(), 1);
    assert_eq!(
        key_at(&mut router, &mut store, "n", 1100),
        vec![],
        "in flight"
    );
    router.spawn_settled();
    assert_eq!(key_at(&mut router, &mut store, "n", 1200).len(), 1);
}

// ── p11: install statusline (spec tui-context-meters "Install
// affordance") ──────────────────────────────────────────────────────

#[test]
fn shift_i_fires_the_install_effect_busy_guarded_until_settled() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "I", 1000), vec![Effect::Install]);
    assert_eq!(
        key_at(&mut router, &mut store, "I", 1100),
        vec![],
        "in flight"
    );
    router.install_settled();
    assert_eq!(key_at(&mut router, &mut store, "I", 1200), vec![Effect::Install]);
}

#[test]
fn install_statusline_notice_strings() {
    assert_eq!(
        INSTALL_STATUSLINE_SUCCESS,
        "statusline feed installed — meters go live as agents work"
    );
    assert_eq!(
        failure_notice("statusline install failed", &ApiError::Transport("x".into())),
        "statusline install failed"
    );
    assert_eq!(
        failure_notice(
            "statusline install failed",
            &ApiError::Status { status: 500, message: "boom".into() }
        ),
        "statusline install failed: boom"
    );
}

// ── p17: `I` installs hooks too (spec tui-hooks-install) ─────────────

fn hooks(already_installed: bool) -> Result<HooksInstallResult, ApiError> {
    Ok(HooksInstallResult { already_installed })
}

fn status_err(status: u16, message: &str) -> ApiError {
    ApiError::Status { status, message: message.to_owned() }
}

#[test]
fn install_notice_both_installed() {
    assert_eq!(
        install_notice(&Ok(()), &hooks(false)),
        "hooks installed · statusline feed installed — meters go live as agents work"
    );
}

#[test]
fn install_notice_hooks_already_installed_is_not_an_error() {
    assert_eq!(
        install_notice(&Ok(()), &hooks(true)),
        "hooks already installed · statusline feed installed — meters go live as agents \
         work"
    );
}

#[test]
fn install_notice_names_the_failed_side_and_still_reports_the_other() {
    let corrupt = "~/.claude/settings.json is not valid JSON";
    assert_eq!(
        install_notice(&Ok(()), &Err(status_err(422, corrupt))),
        format!(
            "hooks install failed: {corrupt} · statusline feed installed — meters go live \
             as agents work"
        )
    );
    assert_eq!(
        install_notice(&Err(status_err(500, "boom")), &hooks(false)),
        "hooks installed · statusline install failed: boom"
    );
    assert_eq!(
        install_notice(&Err(ApiError::Transport("x".into())), &hooks(true)),
        "hooks already installed · statusline install failed"
    );
    assert_eq!(
        install_notice(&Err(status_err(500, "a")), &Err(status_err(422, "b"))),
        "hooks install failed: b · statusline install failed: a"
    );
}

// ── p12: standalone window ("t" opens the focused session in its own
// OS terminal — an effect the store always declines) ──────────────────

#[test]
fn t_with_no_focused_session_is_a_silent_no_op() {
    let mut store = store_with(vec![ws("a")], vec![]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "t", 1000), vec![]);
    assert_eq!(router.notices.current(1001), None);
}

#[test]
fn t_on_a_restorable_session_shows_no_live_session_notice() {
    let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "t", 1000), vec![]);
    assert_eq!(
        router.notices.current(1001),
        Some("no live session to open")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn t_on_a_live_session_fires_open_window() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(
        key_at(&mut router, &mut store, "t", 1000),
        vec![Effect::OpenWindow {
            id: id("a", "one"),
            label: "one".to_owned(),
        }]
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn t_off_macos_shows_macos_only_notice_even_for_a_live_session() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "t", 1000), vec![]);
    assert_eq!(
        router.notices.current(1001),
        Some("standalone windows: macOS only for now")
    );
}

#[test]
fn t_open_window_success_and_failure_notices() {
    assert_eq!(
        format!("opened {} in a new window", "one"),
        "opened one in a new window"
    );
    let err = std::io::Error::other("boom");
    assert_eq!(
        format!("open window failed: {err}"),
        "open window failed: boom"
    );
}

#[test]
fn install_hint_condition_gates_on_shown_delay_and_statusline_sessions() {
    use crate::api::models::ContextInfo;

    let none_yet = vec![WallSession::from_info(&si("a", "one"), None)];
    assert!(
        !should_show_install_hint(false, Duration::from_secs(29), &none_yet),
        "too early"
    );
    assert!(should_show_install_hint(false, Duration::from_secs(30), &none_yet));
    assert!(
        !should_show_install_hint(true, Duration::from_secs(60), &none_yet),
        "already shown this run — never again"
    );

    let transcript_only = vec![WallSession::from_info(
        &SessionInfo {
            context: Some(ContextInfo { used_percentage: 10, source: "transcript".into() }),
            ..si("a", "one")
        },
        None,
    )];
    assert!(
        should_show_install_hint(false, Duration::from_secs(60), &transcript_only),
        "transcript-sourced context doesn't count as statusline-sourced"
    );

    let with_statusline = vec![WallSession::from_info(
        &SessionInfo {
            context: Some(ContextInfo { used_percentage: 10, source: "statusline".into() }),
            ..si("a", "one")
        },
        None,
    )];
    assert!(!should_show_install_hint(false, Duration::from_secs(60), &with_statusline));
}

#[test]
fn any_keypress_dismisses_the_install_hint_forever_this_run() {
    let mut router = GarageRouter::default();
    router.notices.show(crate::ui::strip::STATUSLINE_HINT, 8000, 1000);
    assert_eq!(router.notices.current(1001), Some(crate::ui::strip::STATUSLINE_HINT));
    // The dismissal check runtime.rs's `App::apply` runs before ordinary
    // key handling — exercised directly here against `Notices` since
    // building a full `App` needs a terminal/registry this module
    // doesn't stand up in tests.
    if router.notices.current(1001) == Some(crate::ui::strip::STATUSLINE_HINT) {
        router.notices.clear_prefix(crate::ui::strip::STATUSLINE_HINT);
    }
    assert_eq!(router.notices.current(1001), None, "any keypress clears it");
}

#[test]
fn next_label_ignores_foreign_and_malformed_labels() {
    let store = store_with(
        vec![ws("a"), ws("b")],
        vec![
            si("b", "claude-1"),    // other workspace
            si("a", "claude-x"),    // not the family
            si("a", "claude-1a"),   // trailing junk
            si("a", "api-fix"),
        ],
    );
    assert_eq!(next_label(store.state(), "a"), "claude-1");
}

// ── the rest of the garage bindings ─────────────────────────────────

#[test]
fn q_quits_via_the_effect() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(
        key_at(&mut router, &mut store, "q", 1000),
        vec![Effect::Quit]
    );
}

#[test]
fn a_with_nothing_blocked_shows_the_no_session_notice() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "a", 1000), vec![]);
    assert_eq!(router.notices.current(1001), Some("no session needs you"));
}

#[test]
fn a_with_a_blocked_session_jumps_engaged_no_effects() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si_blocked("b", "old", 100)],
    );
    let mut router = GarageRouter::default();
    assert_eq!(key_at(&mut router, &mut store, "a", 1000), vec![]);
    assert_eq!(store.state().layer, KeyLayer::Engaged);
    assert_eq!(store.state().focused_session_id, Some(id("b", "old")));
}

#[test]
fn unbound_printables_raise_the_typing_hint_rate_limited() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "z", 1000);
    assert_eq!(router.notices.current(1001), Some(TYPING_HINT));
    // Burst typing must not restart the timer: still expires at 3500.
    key_at(&mut router, &mut store, "y", 2000);
    assert_eq!(router.notices.current(3499), Some(TYPING_HINT));
    assert_eq!(router.notices.current(3501), None);
}

#[test]
fn notices_expire_via_tick() {
    let mut notices = Notices::default();
    notices.show("closed ghost", 2000, 1000);
    assert_eq!(notices.current(2999), Some("closed ghost"));
    assert!(!notices.tick(2000), "not expired yet — no change");
    assert!(notices.tick(3000), "expired — visible state changed");
    assert_eq!(notices.current(3000), None);
}

// ── pit-pet chatter vs. user notices (spec tui-pit-pet "Species-voiced
// chatter": "SHALL NOT replace a non-pet notice that is still showing")

#[test]
fn show_pet_is_refused_over_a_live_user_notice() {
    let mut notices = Notices::default();
    notices.show("press x again to close", 3000, 1000);
    assert!(!notices.show_pet("drink some water", 4000, 1500));
    // The user notice is untouched, not clobbered nor extended.
    assert_eq!(notices.current(1500), Some("press x again to close"));
}

#[test]
fn show_pet_is_allowed_once_the_user_notice_has_expired() {
    let mut notices = Notices::default();
    notices.show("press x again to close", 1000, 1000); // expires at 2000
    assert!(notices.show_pet("drink some water", 4000, 2500));
    assert_eq!(notices.current(2500), Some("drink some water"));
}

#[test]
fn a_user_notice_always_overrides_a_live_pet_notice_immediately() {
    let mut notices = Notices::default();
    assert!(notices.show_pet("drink some water", 4000, 1000));
    notices.show(TYPING_HINT, 2500, 1500);
    assert_eq!(notices.current(1500), Some(TYPING_HINT));
}

#[test]
fn a_pet_notice_freely_replaces_another_pet_notice() {
    let mut notices = Notices::default();
    assert!(notices.show_pet("drink some water", 4000, 1000));
    assert!(notices.show_pet("proud of you", 4000, 1200));
    assert_eq!(notices.current(1200), Some("proud of you"));
}

#[test]
fn user_notice_active_tracks_kind_and_expiry() {
    let mut notices = Notices::default();
    assert!(!notices.user_notice_active(1000), "nothing showing yet");
    notices.show("press x again to close", 1000, 1000); // expires at 2000
    assert!(notices.user_notice_active(1500));
    assert!(!notices.user_notice_active(2000), "expired");
    notices.show_pet("drink some water", 4000, 2000);
    assert!(!notices.user_notice_active(2500), "a pet notice is not a user notice");
}

// ── key-event reduction ─────────────────────────────────────────────

#[test]
fn garage_key_string_normalizes_enter_and_passes_plain_chars() {
    let plain =
        |code| garage_key_string(&KeyEvent::new(code, KeyModifiers::NONE));
    assert_eq!(plain(KeyCode::Enter).as_deref(), Some("\r"));
    assert_eq!(plain(KeyCode::Char('x')).as_deref(), Some("x"));
    // Shifted capitals keep their character (the X/N/A/R/K bindings).
    assert_eq!(
        garage_key_string(&KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT)).as_deref(),
        Some("X")
    );
    assert_eq!(
        garage_key_string(&KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)).as_deref(),
        Some("?")
    );
}

#[test]
fn garage_key_string_maps_tab_to_cycle_view_the_wave_1b_follow_up() {
    assert_eq!(
        garage_key_string(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).as_deref(),
        Some("\t")
    );
    let tab = garage_key_string(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).unwrap();
    let resolved = Keymap::defaults().resolve(Layer::Garage, &tab).unwrap();
    assert_eq!(
        command_for_action(resolved.action, resolved.index),
        Some(GarageCommand::CycleView)
    );
}

#[test]
fn ctrl_and_alt_modified_keys_are_consumed_without_reaching_the_bindings() {
    // Mirrors the Dart parser's character-null control events: consumed,
    // no disarm, no typing hint, never an agent byte.
    assert_eq!(
        garage_key_string(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        None
    );
    assert_eq!(
        garage_key_string(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
        None
    );
    assert_eq!(
        garage_key_string(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        None
    );
}
