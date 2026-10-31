//! The one dispatcher (p22 tui-actions "Named actions"): a key and a click
//! that raise the same action must leave the wall in the same state, and
//! every action id must act through it, the restart ids included.
use super::*;

fn app(workspaces: Vec<WorkspaceInfo>, sessions: Vec<SessionInfo>) -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0", 0);
    app.store.workspaces_fetched(workspaces);
    app.store.sessions_fetched(sessions);
    app
}

fn two_workspaces() -> App {
    app(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si("a", "two"), si("b", "three")],
    )
}

fn click_ctx(app: &App) -> ActionCtx {
    ActionCtx::from_focus(app.store.state(), ActionSource::Mouse { col: 1, row: 1 })
}

fn state_of(app: &App) -> String {
    format!("{:?} q={}", app.store.state(), app.queue_selection)
}

#[test]
fn a_key_and_a_click_for_the_same_action_leave_the_same_state() {
    // (key, action, ctx tweak) — the click side builds its context the way
    // `action_at` does.
    type Tweak = fn(ActionCtx) -> ActionCtx;
    let cases: Vec<(&str, ActionId, Tweak)> = vec![
        ("2", ids::WORKSPACE_FOCUS, |c| c.with_index(Some(1))),
        ("A", ids::OVERLAY_OPEN_QUEUE, |c| c),
        ("]", ids::FOCUS_CYCLE_NEXT, |c| c),
        ("m", ids::SESSION_MAXIMIZE, |c| c),
    ];
    for (key, id, tweak) in cases {
        let mut by_key = two_workspaces();
        let quit = by_key.handle_key(&plain(KeyCode::Char(key.chars().next().unwrap())));
        assert!(!quit);

        let mut by_click = two_workspaces();
        let ctx = tweak(click_ctx(&by_click));
        let effects = by_click.dispatch_action(id, &ctx);
        assert!(effects.is_empty(), "{id:?}");

        assert_eq!(state_of(&by_key), state_of(&by_click), "{key:?} vs click on {id:?}");
    }
}

#[test]
fn the_source_of_a_context_never_changes_what_an_action_does() {
    for id in [ids::OVERLAY_OPEN_QUEUE, ids::SESSION_MAXIMIZE, ids::FOCUS_CYCLE_PREV] {
        let mut a = two_workspaces();
        let mut b = two_workspaces();
        let key_ctx = ActionCtx::from_focus(a.store.state(), ActionSource::Key("x".into()));
        a.dispatch_action(id, &key_ctx);
        b.dispatch_action(id, &click_ctx(&b));
        assert_eq!(state_of(&a), state_of(&b), "{id:?}");
    }
}

#[test]
fn session_focus_lands_on_the_named_session_and_disengages_first() {
    let mut app = two_workspaces();
    app.store.engage();
    assert_eq!(app.store.state().layer, KeyLayer::Engaged);
    let ctx = click_ctx(&app).with_session(id("a", "two"));
    app.dispatch_action(ids::SESSION_FOCUS, &ctx);
    let state = app.store.state();
    assert_eq!(state.layer, KeyLayer::Garage, "a click leaves the engaged tile first");
    assert_eq!(state.focused_session_id.as_deref(), Some(id("a", "two").as_str()));
}

#[test]
fn session_engage_focuses_the_clicked_tile_then_engages_it() {
    let mut app = two_workspaces();
    let ctx = click_ctx(&app).with_session(id("a", "two"));
    app.dispatch_action(ids::SESSION_ENGAGE, &ctx);
    let state = app.store.state();
    assert_eq!(state.layer, KeyLayer::Engaged);
    assert_eq!(state.focused_session_id.as_deref(), Some(id("a", "two").as_str()));
}

#[test]
fn session_restore_focuses_then_restores_with_the_in_flight_guard() {
    let mut app = app(vec![ws("a")], vec![si("a", "live"), si_restorable("a", "old")]);
    let ctx = click_ctx(&app).with_session(id("a", "old"));
    let effects = app.dispatch_action(ids::SESSION_RESTORE, &ctx);
    assert_eq!(effects, vec![Effect::Restore { id: id("a", "old") }]);
    assert_eq!(
        app.store.state().focused_session_id.as_deref(),
        Some(id("a", "old").as_str())
    );
    assert!(app.dispatch_action(ids::SESSION_RESTORE, &ctx).is_empty(), "no double restore");
}

#[test]
fn view_focus_and_overlay_close_act_through_the_dispatcher() {
    let mut app = two_workspaces();
    app.store.move_focused_to_view("backend");
    let ctx = click_ctx(&app).with_view("main".into());
    app.dispatch_action(ids::VIEW_FOCUS, &ctx);
    let ws_name = app.store.state().focused_workspace.clone().unwrap();
    assert_eq!(app.store.state().focused_view_name(&ws_name), "main");

    app.dispatch_action(ids::OVERLAY_OPEN_WORKSPACE_ADD, &click_ctx(&app));
    app.ws_form.insert_char('x');
    assert_eq!(app.store.state().overlay, Some(OverlayKind::WorkspaceAdd));
    app.dispatch_action(ids::OVERLAY_CLOSE, &click_ctx(&app));
    assert_eq!(app.store.state().overlay, None);
    app.dispatch_action(ids::OVERLAY_OPEN_WORKSPACE_ADD, &click_ctx(&app));
    assert_eq!(app.ws_form.input, "", "a fresh form on every open");
}

#[test]
fn opening_the_queue_resets_its_selection_whatever_raised_the_action() {
    let mut app = two_workspaces();
    app.queue_selection = 4;
    app.dispatch_action(ids::OVERLAY_OPEN_QUEUE, &click_ctx(&app));
    assert_eq!(app.queue_selection, 0);
}

#[test]
fn the_restart_ids_produce_their_effects() {
    let mut app = app(vec![ws("a")], vec![si_status("a", "one", "idle"), si_status("a", "two", "idle")]);
    let ctx = ActionCtx::from_focus(app.store.state(), ActionSource::Key("r".into()));
    // Armed double-press: the first press only arms.
    assert!(app.dispatch_action(ids::SESSION_RESTART, &ctx).is_empty());
    let effects = app.dispatch_action(ids::SESSION_RESTART, &ctx);
    assert!(matches!(effects.as_slice(), [Effect::RestartSession { force: false, .. }]), "{effects:?}");

    let effects = app.dispatch_action(ids::WORKSPACE_RESTART, &ctx);
    assert!(matches!(effects.as_slice(), [Effect::RestartWorkspace { ids }] if ids.len() == 2), "{effects:?}");

    assert_eq!(app.dispatch_action(ids::DAEMON_RESTART, &ctx), vec![Effect::RestartDaemon]);
    assert_eq!(app.router.notices.current(now_ms()), Some("restarting the daemon…"));
}

#[test]
fn the_apps_one_keymap_drives_garage_keys_and_overlay_keys_alike() {
    use crate::keymap::Layer;
    let mut app = two_workspaces();
    app.keymap.bind(Layer::Garage, "z", ids::SESSION_MAXIMIZE);
    app.keymap.bind(Layer::Overlay(OverlayKind::Help), "x", ids::OVERLAY_CLOSE);

    app.handle_key(&plain(KeyCode::Char('z')));
    assert!(app.store.state().maximized_session_id.is_some(), "a garage key bound on the app's keymap");

    app.handle_key(&plain(KeyCode::Char('?')));
    assert_eq!(app.store.state().overlay, Some(OverlayKind::Help));
    app.handle_key(&plain(KeyCode::Char('x')));
    assert_eq!(app.store.state().overlay, None, "an overlay key bound on the same keymap");
}
