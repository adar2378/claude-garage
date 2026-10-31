//! Fetch reconciliation, layers, grid, jump, command mapping, status events,
//! maximize and the lifecycle commands.
use super::*;

// ── fetch reconciliation ────────────────────────────────────────────

#[test]
fn first_fetch_focuses_the_first_group_and_grids_its_sessions() {
    let store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si("a", "two"), si("b", "other")],
    );
    assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
    assert_eq!(store.state().focused_session_id, Some(id("a", "one")));
    assert_eq!(
        store.state().gridded_session_ids,
        [id("a", "one"), id("a", "two")]
    );
    assert_eq!(store.state().layer, KeyLayer::Garage);
    assert_eq!(store.state().keys_target_chip(), "keys → garage");
}

#[test]
fn usage_fetched_replaces_the_account_wide_usage_and_nothing_else() {
    use crate::api::models::UsageWindow;

    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert_eq!(store.state().usage, UsageInfo::default());
    let before_version = store.version();

    store.usage_fetched(UsageInfo {
        five_hour: Some(UsageWindow { used_percentage: 24, resets_at: None }),
        seven_day: None,
    });
    assert_eq!(
        store.state().usage.five_hour.as_ref().map(|w| w.used_percentage),
        Some(24)
    );
    assert_eq!(store.state().usage.seven_day, None);
    assert!(store.version() > before_version);
    // Sessions/focus untouched by an account-wide usage refetch.
    assert_eq!(store.state().focused_session_id, Some(id("a", "one")));
}

#[test]
fn worktree_flag_derives_from_dir_differing_from_the_registered_dir() {
    let wt = SessionInfo {
        dir: Some("/repos/.garage-worktrees/a-wt".to_owned()),
        ..si("a", "wt")
    };
    let store = store_with(vec![ws("a")], vec![si("a", "plain"), wt]);
    let plain = store.state().session_by_id(Some(&id("a", "plain"))).unwrap();
    assert!(!plain.worktree);
    let wt = store.state().session_by_id(Some(&id("a", "wt"))).unwrap();
    assert!(wt.worktree);
}

#[test]
fn a_vanished_focused_workspace_falls_back_to_the_first_group() {
    let mut store = store_with(vec![ws("a"), ws("b")], vec![si("a", "one")]);
    store.workspaces_fetched(vec![ws("b")]);
    store.sessions_fetched(vec![si("b", "other")]);
    assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
    assert_eq!(store.state().focused_session_id, Some(id("b", "other")));
}

#[test]
fn p8_3_x_remove_transition_registry_only_removal_resurfaces_live_sessions_unregistered() {
    // Before: `a` is a registered workspace with two live sessions.
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si("a", "two"), si("b", "other")],
    );
    assert!(store.state().group_by_name(Some("a")).unwrap().registered);

    // DELETE /api/workspaces/a (registry-only) → the refetch lists no
    // workspace `a`, but its tmux sessions are still live and listed.
    store.workspaces_fetched(vec![ws("b")]);
    store.sessions_fetched(vec![si("a", "one"), si("a", "two"), si("b", "other")]);

    let group = store
        .state()
        .group_by_name(Some("a"))
        .expect("live sessions must never become invisible");
    assert!(
        !group.registered,
        "the group is now synthesized from tmux discovery"
    );
    let ids: Vec<&str> = group.sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, [id("a", "one"), id("a", "two")]);
    assert!(store
        .state()
        .session_by_id(Some(&id("a", "one")))
        .unwrap()
        .live());
    // Registered groups keep registry order first; the synthesized group
    // trails them (salience build_groups insertion order).
    let names: Vec<&str> = store.state().groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["b", "a"]);
}

// ── layer transitions ───────────────────────────────────────────────

#[test]
fn engage_requires_a_focused_live_session_empty_wall_is_a_no_op() {
    let mut store = store_with(vec![ws("a")], vec![]);
    store.engage();
    assert_eq!(store.state().layer, KeyLayer::Garage);
}

#[test]
fn engage_on_a_restorable_placeholder_is_a_no_op() {
    let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "dead")]);
    assert_eq!(store.state().focused_session_id, Some(id("a", "dead")));
    store.engage();
    assert_eq!(store.state().layer, KeyLayer::Garage);
}

#[test]
fn engage_on_a_live_session_flips_the_layer_and_the_chip_in_the_same_state_change() {
    let mut store = store_with(vec![ws("apexlabs")], vec![si("apexlabs", "api-fix")]);
    store.engage();
    assert_eq!(store.state().layer, KeyLayer::Engaged);
    assert_eq!(store.state().keys_target_chip(), "keys → apexlabs/api-fix");
}

#[test]
fn disengage_returns_to_garage_disengaging_from_garage_is_a_no_op() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.disengage();
    assert_eq!(store.state().layer, KeyLayer::Garage);
    store.engage();
    store.disengage();
    assert_eq!(store.state().layer, KeyLayer::Garage);
    assert_eq!(store.state().keys_target_chip(), "keys → garage");
}

#[test]
fn overlays_never_stack_opening_a_second_overlay_is_a_no_op() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::Help);
    assert_eq!(store.state().overlay, Some(OverlayKind::Help));
    store.open_overlay(OverlayKind::TriageQueue);
    assert_eq!(store.state().overlay, Some(OverlayKind::Help));
    assert_eq!(store.state().layer, KeyLayer::Overlay);
}

#[test]
fn an_overlay_cannot_open_while_engaged() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.engage();
    store.open_overlay(OverlayKind::TriageQueue);
    assert_eq!(store.state().layer, KeyLayer::Engaged);
    assert_eq!(store.state().overlay, None);
}

#[test]
fn close_overlay_returns_to_garage_and_clears_the_overlay_slot() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::TriageQueue);
    assert_eq!(store.state().keys_target_chip(), "keys → queue");
    store.close_overlay();
    assert_eq!(store.state().layer, KeyLayer::Garage);
    assert_eq!(store.state().overlay, None);
}

#[test]
fn engagement_is_dropped_when_the_engaged_session_leaves_the_listing() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.engage();
    assert_eq!(store.state().layer, KeyLayer::Engaged);
    store.sessions_fetched(vec![si("a", "two")]);
    assert_eq!(store.state().layer, KeyLayer::Garage);
    assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
}

// ── grid: 6-cap with LRU swap-in ────────────────────────────────────

fn seven() -> Vec<SessionInfo> {
    (1..=7).map(|i| si("a", &format!("s{i}"))).collect()
}

#[test]
fn a_seventh_session_stays_off_the_grid_rail_only() {
    let store = store_with(vec![ws("a")], seven());
    assert_eq!(store.state().gridded_session_ids.len(), 6);
    assert!(!store.state().gridded_session_ids.contains(&id("a", "s7")));
}

#[test]
fn focusing_the_overflow_session_swaps_it_into_the_lru_slot_other_tiles_keep_positions() {
    let mut store = store_with(vec![ws("a")], seven());
    // Touch every tile except s1, making s1 the least-recently-focused.
    for i in 2..=6 {
        store.focus_session(&id("a", &format!("s{i}")));
    }
    store.focus_session(&id("a", "s7"));
    // s7 took s1's slot (slot 0) — slot-stable swap-in.
    assert_eq!(
        store.state().gridded_session_ids,
        [
            id("a", "s7"),
            id("a", "s2"),
            id("a", "s3"),
            id("a", "s4"),
            id("a", "s5"),
            id("a", "s6"),
        ]
    );
    assert_eq!(store.state().focused_session_id, Some(id("a", "s7")));
}

#[test]
fn lru_follows_focus_recency_not_grid_order() {
    let mut store = store_with(vec![ws("a")], seven());
    store.focus_session(&id("a", "s1")); // s1 is now most-recent; s2 is LRU
    store.focus_session(&id("a", "s7"));
    assert!(store.state().gridded_session_ids.contains(&id("a", "s1")));
    assert!(!store.state().gridded_session_ids.contains(&id("a", "s2")));
    let pos = store
        .state()
        .gridded_session_ids
        .iter()
        .position(|s| *s == id("a", "s7"));
    assert_eq!(pos, Some(1), "s2's old slot");
}

#[test]
fn below_the_cap_focusing_an_ungridded_session_appends_it() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si("b", "other")],
    );
    // Cross-workspace focus switches workspace and grids the session.
    store.focus_session(&id("b", "other"));
    assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
    assert_eq!(store.state().gridded_session_ids, [id("b", "other")]);
}

#[test]
fn cycle_focus_wraps_in_both_directions() {
    let mut store = store_with(
        vec![ws("a")],
        vec![si("a", "s1"), si("a", "s2"), si("a", "s3")],
    );
    store.cycle_focus(-1);
    assert_eq!(store.state().focused_session_id, Some(id("a", "s3")));
    store.cycle_focus(1);
    assert_eq!(store.state().focused_session_id, Some(id("a", "s1")));
    store.cycle_focus(1);
    assert_eq!(store.state().focused_session_id, Some(id("a", "s2")));
}

#[test]
fn a_gone_gridded_session_is_pruned_and_the_grid_refills_from_the_rail_overflow() {
    let mut store = store_with(vec![ws("a")], seven());
    store.sessions_fetched(seven().into_iter().filter(|s| s.label != "s3").collect());
    assert_eq!(store.state().gridded_session_ids.len(), 6);
    assert!(store.state().gridded_session_ids.contains(&id("a", "s7")));
    assert!(!store.state().gridded_session_ids.contains(&id("a", "s3")));
}

// ── a-jump ──────────────────────────────────────────────────────────

#[test]
fn jumps_to_the_longest_waiting_blocked_session_across_workspaces_and_lands_engaged() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![
            si("a", "one"),
            si_status("a", "young", "needs-input", Some(5000)),
            si_status("b", "old", "needs-input", Some(100)),
        ],
    );
    assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
    assert!(store.jump_to_longest_waiting());
    assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
    assert_eq!(store.state().focused_session_id, Some(id("b", "old")));
    assert_eq!(store.state().layer, KeyLayer::Engaged);
    assert_eq!(store.state().keys_target_chip(), "keys → b/old");
}

#[test]
fn an_overflow_blocked_session_is_swapped_into_the_grid_by_the_jump() {
    // 7 blocked sessions: salience keeps listing order, so the grid holds
    // s1..s6 and the oldest-waiting s7 is overflow until the jump.
    let sessions: Vec<SessionInfo> = (1..=7)
        .map(|i| si_status("a", &format!("s{i}"), "needs-input", Some(800 - i)))
        .collect();
    let mut store = store_with(vec![ws("a")], sessions);
    assert!(!store.state().gridded_session_ids.contains(&id("a", "s7")));
    assert!(store.jump_to_longest_waiting());
    assert!(store.state().gridded_session_ids.contains(&id("a", "s7")));
    assert_eq!(store.state().focused_session_id, Some(id("a", "s7")));
    assert_eq!(store.state().layer, KeyLayer::Engaged);
}

#[test]
fn with_nothing_blocked_the_jump_is_a_no_op_returning_false() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let before = store.version();
    assert!(!store.jump_to_longest_waiting());
    assert_eq!(store.version(), before, "state untouched");
    assert_eq!(store.state().layer, KeyLayer::Garage);
}

#[test]
fn the_jump_only_fires_from_the_garage_layer() {
    let mut store = store_with(
        vec![ws("a")],
        vec![si("a", "one"), si_status("a", "blocked", "needs-input", Some(1000))],
    );
    store.focus_session(&id("a", "one"));
    store.engage();
    assert!(!store.jump_to_longest_waiting());
    assert_eq!(store.state().focused_session_id, Some(id("a", "one")));
    assert_eq!(store.state().layer, KeyLayer::Engaged);
}

// ── garage command mapping ──────────────────────────────────────────

#[test]
fn maps_every_bound_key() {
    assert_eq!(
        garage_command_for("1"),
        Some(GarageCommand::FocusWorkspace(0))
    );
    assert_eq!(
        garage_command_for("9"),
        Some(GarageCommand::FocusWorkspace(8))
    );
    assert_eq!(garage_command_for("["), Some(GarageCommand::CycleFocus(-1)));
    assert_eq!(garage_command_for("]"), Some(GarageCommand::CycleFocus(1)));
    assert_eq!(garage_command_for("a"), Some(GarageCommand::Jump));
    assert_eq!(garage_command_for("A"), Some(GarageCommand::OpenQueue));
    assert_eq!(
        garage_command_for("n"),
        Some(GarageCommand::Spawn { worktree: false })
    );
    assert_eq!(
        garage_command_for("N"),
        Some(GarageCommand::Spawn { worktree: true })
    );
    assert_eq!(garage_command_for("\n"), Some(GarageCommand::Engage));
    assert_eq!(garage_command_for("\r"), Some(GarageCommand::Engage));
    assert_eq!(garage_command_for("?"), Some(GarageCommand::ToggleHelp));
    assert_eq!(garage_command_for("q"), Some(GarageCommand::Quit));
}

#[test]
fn unbound_keys_map_to_nothing_garage_typing_never_reaches_an_agent() {
    assert_eq!(garage_command_for("z"), None);
    assert_eq!(garage_command_for("0"), None);
    assert_eq!(garage_command_for(" "), None);
}

// ── p11: install-statusline key (spec tui-context-meters "Install
// affordance") ────────────────────────────────────────────────────────

#[test]
fn shift_i_installs_statusline_lowercase_i_stays_free() {
    assert_eq!(
        garage_command_for("I"),
        Some(GarageCommand::InstallStatusline)
    );
    assert_eq!(garage_command_for("i"), None, "lowercase i is unbound");
}

#[test]
fn install_statusline_is_always_declined_by_dispatch() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(!store.dispatch(GarageCommand::InstallStatusline));
}

// ── p12: standalone-window key ("t" opens the focused session in its
// own OS terminal, an effect the store always declines) ────────────────

#[test]
fn t_maps_to_open_window_an_effect_the_store_declines() {
    assert_eq!(garage_command_for("t"), Some(GarageCommand::OpenWindow));
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(!store.dispatch(GarageCommand::OpenWindow));
}

#[test]
fn p8_1_lifecycle_keys_map_to_their_commands() {
    assert_eq!(garage_command_for("m"), Some(GarageCommand::Maximize));
    assert_eq!(garage_command_for("R"), Some(GarageCommand::RestoreAll));
    assert_eq!(garage_command_for("x"), Some(GarageCommand::Close));
    assert_eq!(garage_command_for("w"), Some(GarageCommand::WorkspaceAdd));
}

#[test]
fn p8_3_shift_x_maps_to_workspace_removal_an_effect_the_store_declines() {
    assert_eq!(
        garage_command_for("X"),
        Some(GarageCommand::WorkspaceRemove)
    );
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(!store.dispatch(GarageCommand::WorkspaceRemove));
    assert_eq!(
        store.state().layer,
        KeyLayer::Garage,
        "declining must leave state untouched"
    );
}

#[test]
fn digits_focus_workspaces_in_salience_order() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si_status("b", "blocked", "needs-input", Some(1000))],
    );
    // b is blocked, so it holds rail position 1.
    assert!(store.dispatch(GarageCommand::FocusWorkspace(0)));
    assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
    assert!(store.dispatch(GarageCommand::FocusWorkspace(1)));
    assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
    // Out-of-range digit: handled (consumed) but state unchanged.
    store.dispatch(GarageCommand::FocusWorkspace(8));
    assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
}

#[test]
fn question_mark_toggles_the_help_overlay() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(store.dispatch(GarageCommand::ToggleHelp));
    assert_eq!(store.state().overlay, Some(OverlayKind::Help));
    assert!(store.dispatch(GarageCommand::ToggleHelp));
    assert_eq!(store.state().layer, KeyLayer::Garage);
    assert_eq!(store.state().overlay, None);
}

#[test]
fn spawn_and_quit_are_caller_effects_not_store_transitions() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(!store.dispatch(GarageCommand::Spawn { worktree: false }));
    assert!(!store.dispatch(GarageCommand::Quit));
    assert_eq!(store.state().layer, KeyLayer::Garage);
}

#[test]
fn state_commands_are_inert_outside_the_garage_layer() {
    let mut store = store_with(vec![ws("a"), ws("b")], vec![si("a", "one")]);
    store.engage();
    assert!(!store.dispatch(GarageCommand::FocusWorkspace(1)));
    assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
    assert!(!store.dispatch(GarageCommand::Jump));
}

// ── status events ───────────────────────────────────────────────────

#[test]
fn status_changed_reorders_salience_and_stamps_the_new_since() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si("b", "other")],
    );
    store.status_changed(&id("b", "other"), "needs-input", Some(4242));
    assert_eq!(store.state().groups[0].name, "b");
    let s = store.state().session_by_id(Some(&id("b", "other"))).unwrap();
    assert_eq!(s.status, "needs-input");
    assert_eq!(s.since, Some(4242));
}

#[test]
fn a_transition_away_from_needs_input_clears_the_message_mirrors_the_daemon_store() {
    let blocked = SessionInfo {
        message: Some("Claude needs your permission".to_owned()),
        ..si_status("a", "one", "needs-input", Some(1000))
    };
    let mut store = store_with(vec![ws("a")], vec![blocked]);
    assert_eq!(
        store
            .state()
            .session_by_id(Some(&id("a", "one")))
            .unwrap()
            .message
            .as_deref(),
        Some("Claude needs your permission")
    );
    store.status_changed(&id("a", "one"), "working", Some(9000));
    assert_eq!(
        store
            .state()
            .session_by_id(Some(&id("a", "one")))
            .unwrap()
            .message,
        None
    );
}

#[test]
fn status_changed_for_an_unknown_id_is_ignored_until_the_refetch() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let before = store.version();
    store.status_changed("garage/a/ghost", "needs-input", Some(1));
    assert_eq!(store.version(), before, "state untouched");
}

// ── maximize (p8.1, spec tui-wall "Maximized tile") ─────────────────

#[test]
fn m_toggles_the_focused_tile_full_grid_and_back() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    assert_eq!(store.state().maximized_session_id, None);
    store.dispatch(GarageCommand::Maximize);
    assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
    store.dispatch(GarageCommand::Maximize);
    assert_eq!(store.state().maximized_session_id, None);
}

#[test]
fn focusing_another_session_exits_maximize() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.toggle_maximize();
    assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
    store.focus_session(&id("a", "two"));
    assert_eq!(store.state().maximized_session_id, None);
    assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
}

#[test]
fn cycling_focus_exits_maximize() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.toggle_maximize();
    store.cycle_focus(1);
    assert_eq!(store.state().maximized_session_id, None);
}

#[test]
fn switching_workspace_exits_maximize() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si("b", "other")],
    );
    store.toggle_maximize();
    store.focus_workspace(1);
    assert_eq!(store.state().maximized_session_id, None);
}

#[test]
fn re_focusing_the_maximized_session_keeps_it_maximized() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.toggle_maximize();
    store.focus_session(&id("a", "one"));
    assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
}

#[test]
fn engagement_keeps_maximize_maximize_plus_engage_is_the_workflow() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.toggle_maximize();
    store.engage();
    assert_eq!(store.state().layer, KeyLayer::Engaged);
    assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
    // m is a garage-layer binding: while engaged the toggle is inert.
    store.toggle_maximize();
    assert_eq!(store.state().maximized_session_id, Some(id("a", "one")));
}

#[test]
fn reconciliation_clears_maximize_when_the_session_dies() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.toggle_maximize();
    store.sessions_fetched(vec![si("a", "two")]);
    assert_eq!(store.state().maximized_session_id, None);
    assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
}

#[test]
fn maximize_with_no_focus_empty_wall_is_a_no_op() {
    let mut store = store_with(vec![ws("a")], vec![]);
    store.toggle_maximize();
    assert_eq!(store.state().maximized_session_id, None);
}

// ── restore / close / workspace-add commands (p8.1) ─────────────────

#[test]
fn engage_returns_false_for_a_restorable_placeholder_caller_restores_instead() {
    let mut store = store_with(vec![ws("a")], vec![si_restorable("a", "gone")]);
    assert!(!store.dispatch(GarageCommand::Engage));
    assert_eq!(
        store.state().layer,
        KeyLayer::Garage,
        "engage must still require a live session"
    );
}

#[test]
fn engage_still_works_and_returns_true_for_a_live_session() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(store.dispatch(GarageCommand::Engage));
    assert_eq!(store.state().layer, KeyLayer::Engaged);
}

#[test]
fn r_and_x_are_effects_the_store_always_declines_them() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(!store.dispatch(GarageCommand::RestoreAll));
    assert!(!store.dispatch(GarageCommand::Close));
    assert_eq!(store.state().layer, KeyLayer::Garage);
}

#[test]
fn w_opens_the_add_workspace_overlay_overlays_never_stack() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(store.dispatch(GarageCommand::WorkspaceAdd));
    assert_eq!(store.state().layer, KeyLayer::Overlay);
    assert_eq!(store.state().overlay, Some(OverlayKind::WorkspaceAdd));
    assert_eq!(store.state().keys_target_chip(), "keys → add workspace");
    assert!(!store.dispatch(GarageCommand::WorkspaceAdd));
    store.close_overlay();
    assert_eq!(store.state().layer, KeyLayer::Garage);
}

#[test]
fn focus_workspace_named_lands_on_the_named_group() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si("b", "other")],
    );
    store.focus_workspace_named("b");
    assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
    assert_eq!(store.state().focused_session_id, Some(id("b", "other")));
    store.focus_workspace_named("ghost"); // unknown name is a no-op
    assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
}

#[test]
fn restorable_session_ids_selects_only_restorable_sessions_of_the_workspace_in_order() {
    let store = store_with(
        vec![ws("a"), ws("b")],
        vec![
            si("a", "live"),
            si_restorable("a", "r1"),
            si_restorable("b", "r-other"),
            si_restorable("a", "r2"),
        ],
    );
    assert_eq!(
        restorable_session_ids(&store.state().sessions, "a"),
        [id("a", "r1"), id("a", "r2")]
    );
    assert_eq!(
        restorable_session_ids(&store.state().sessions, "b"),
        [id("b", "r-other")]
    );
    assert!(restorable_session_ids(&store.state().sessions, "ghost").is_empty());
}
