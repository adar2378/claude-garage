//! Pit-pet and connection state, and the p16 restart chord arming.
use super::*;

// ── p15: pit pet + connection state ────────────────────────────────

#[test]
fn set_pet_updates_state_and_bumps_views_revision() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert_eq!(store.state().pet, None, "off by default");
    let before = store.views_revision();

    store.set_pet(Some("duck".to_owned()));

    assert_eq!(store.state().pet.as_deref(), Some("duck"));
    assert!(
        store.views_revision() > before,
        "a pet change must be as save-worthy as a view change — same wall.json"
    );
}

#[test]
fn set_pet_off_clears_the_choice() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.set_pet(Some("cat".to_owned()));
    store.set_pet(None);
    assert_eq!(store.state().pet, None);
}

#[test]
fn connection_updates_daemon_live_without_touching_views_revision() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(store.state().daemon_live, "true until an observed drop");
    let before = store.views_revision();

    store.connection(false);

    assert!(!store.state().daemon_live);
    assert_eq!(
        store.views_revision(),
        before,
        "connection state is transient — never persisted, never dirties the save hook"
    );

    store.connection(true);
    assert!(store.state().daemon_live);
    assert_eq!(store.views_revision(), before);
}

// ── p16-restart: the `r` prefix, the `r r` arming, `r a` targets ─────

#[test]
fn the_r_prefix_routes_all_three_chords_and_cancels_on_anything_else() {
    assert_eq!(
        restart_command_for("r"),
        Some(GarageCommand::RestartFocused)
    );
    assert_eq!(
        restart_command_for("a"),
        Some(GarageCommand::RestartWorkspace)
    );
    assert_eq!(restart_command_for("d"), Some(GarageCommand::RestartDaemon));
    // Anything else cancels the prefix (the router swallows the key).
    assert_eq!(restart_command_for("x"), None);
    assert_eq!(restart_command_for("q"), None);
    assert_eq!(restart_command_for(""), None);
    // `r` alone is a prefix, never a command of its own.
    assert_eq!(garage_command_for(RESTART_PREFIX), None);
}

#[test]
fn the_three_restart_commands_are_effects_the_store_declines() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(!store.dispatch(GarageCommand::RestartFocused));
    assert!(!store.dispatch(GarageCommand::RestartWorkspace));
    assert!(!store.dispatch(GarageCommand::RestartDaemon));
}

#[test]
fn r_r_arms_then_the_second_press_fires_within_the_window() {
    let mut armed = ArmedRestart::default();
    assert_eq!(armed.press(&id("a", "one"), false, 1000), None, "armed");
    assert_eq!(armed.armed_id(), Some(id("a", "one").as_str()));
    assert_eq!(
        armed.press(&id("a", "one"), false, 2500),
        Some(false),
        "fires without force on an idle session"
    );
    assert_eq!(armed.armed_id(), None, "firing consumes the arm");
}

#[test]
fn an_expired_r_r_arm_re_arms_instead_of_firing() {
    let mut armed = ArmedRestart::default();
    armed.press(&id("a", "one"), false, 1000);
    assert_eq!(armed.press(&id("a", "one"), false, 4001), None, "expired");
    assert_eq!(armed.press(&id("a", "one"), false, 4500), Some(false));
}

#[test]
fn a_busy_session_armed_busy_fires_with_force() {
    let mut armed = ArmedRestart::default();
    assert_eq!(armed.press(&id("a", "one"), true, 1000), None);
    // Even if the status settled between the presses, the arm the user
    // answered was the busy one — it forces.
    assert_eq!(armed.press(&id("a", "one"), false, 2000), Some(true));
    // The next cycle starts clean: an idle arm never forces.
    assert_eq!(armed.press(&id("a", "one"), false, 3000), None);
    assert_eq!(armed.press(&id("a", "one"), false, 3500), Some(false));
}

#[test]
fn disarming_the_restart_drops_the_force_bit_too() {
    let mut armed = ArmedRestart::default();
    armed.press(&id("a", "one"), true, 1000);
    armed.disarm();
    assert_eq!(armed.armed_id(), None);
    assert_eq!(armed.press(&id("a", "one"), false, 1100), None, "from scratch");
    assert_eq!(armed.press(&id("a", "one"), false, 1200), Some(false));
}

#[test]
fn the_arm_notice_promises_a_resume_and_warns_when_busy() {
    assert_eq!(
        restart_arm_notice("claude-1", false),
        "restart claude-1? r r again — resumes the conversation"
    );
    assert_eq!(
        restart_arm_notice("claude-1", true),
        "claude-1 is working — r r again to restart anyway"
    );
}

#[test]
fn r_a_targets_only_idle_and_done_live_sessions_of_the_focused_workspace() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![
            si_status("a", "idle-one", "idle", Some(1000)),
            si_status("a", "busy", "working", Some(1000)),
            si_status("a", "blocked", "needs-input", Some(1000)),
            si_status("a", "done-one", "done", Some(1000)),
            si_restorable("a", "dead"),
            si_status("b", "elsewhere", "idle", Some(1000)),
        ],
    );
    assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
    assert_eq!(
        store.restart_targets_for_workspace(),
        vec![id("a", "idle-one"), id("a", "done-one")]
    );

    store.focus_workspace_named("b");
    assert_eq!(
        store.restart_targets_for_workspace(),
        vec![id("b", "elsewhere")]
    );
}

#[test]
fn a_workspace_with_nothing_idle_has_no_restart_targets() {
    let store = store_with(
        vec![ws("a")],
        vec![si("a", "busy"), si_restorable("a", "dead")],
    );
    assert!(store.restart_targets_for_workspace().is_empty());
}
