//! The confirm rule (p24 plugin-host "Confirm rule for dangerous actions"):
//! plugin sources only ARM a dangerous action, a human key or click confirms,
//! a plugin that triggers twice never executes, refused ids never run, and a
//! core action's target comes from the context, never from `args`.
use super::*;
use crate::actions::{meta, ActionId, META};
use crate::slots::{ClickTarget, SlotClickRegion, SlotId};
use ratatui::layout::Rect;

const ONE: &str = "garage/a/one";
const TWO: &str = "garage/a/two";
const OLD: &str = "garage/a/old";
const THREE: &str = "garage/b/three";

fn fixture() -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0", 0);
    app.store.workspaces_fetched(vec![ws("a"), ws("b")]);
    app.store.sessions_fetched(vec![
        si("a", "one"),
        si_status("a", "two", "idle"),
        si_restorable("a", "old"),
        si("b", "three"),
    ]);
    app
}

fn state_of(app: &App) -> String {
    format!("{:?} q={}", app.store.state(), app.queue_selection)
}

fn widget(app: &App, session: &str) -> ActionCtx {
    let mut ctx = ActionCtx::from_focus(
        app.store.state(),
        ActionSource::Widget { plugin: "closer".into(), slot: SlotId::TileTitleRight },
    );
    ctx.session = Some(session.to_owned());
    ctx.workspace = Some(session.split('/').nth(1).unwrap().to_owned());
    ctx
}

fn plugin(app: &App, session: &str) -> ActionCtx {
    let mut ctx = widget(app, session);
    ctx.source = ActionSource::plugin("closer");
    ctx
}

fn mouse(mut ctx: ActionCtx) -> ActionCtx {
    ctx.source = ActionSource::Mouse { col: 3, row: 0 };
    ctx
}

const DANGEROUS: [ActionId; 9] = [
    ids::SESSION_CLOSE,
    ids::SESSION_DELETE,
    ids::SESSION_RESTART,
    ids::WORKSPACE_RESTART,
    ids::DAEMON_RESTART,
    ids::WORKSPACE_REMOVE,
    ids::WORKSPACE_REMOVE_KILL,
    ids::SESSION_RESTORE_ALL,
    ids::HOOKS_INSTALL,
];

/// What a confirmed run of each dangerous id must ask for, on the fixture.
fn confirmed_effect_ok(id: ActionId, effects: &[Effect]) -> bool {
    match (id, effects) {
        (ids::SESSION_CLOSE, [Effect::Close { id, meta_only: false, .. }]) => id == TWO,
        (ids::SESSION_DELETE, [Effect::Close { id, meta_only: true, .. }]) => id == OLD,
        (ids::SESSION_RESTART, [Effect::RestartSession { id, force: false, .. }]) => id == TWO,
        (ids::WORKSPACE_RESTART, [Effect::RestartWorkspace { ids }]) => ids == &[TWO.to_owned()],
        (ids::DAEMON_RESTART, [Effect::RestartDaemon]) => true,
        (ids::WORKSPACE_REMOVE, [Effect::RemoveWorkspace { name, kill: false }]) => name == "a",
        (ids::WORKSPACE_REMOVE_KILL, [Effect::RemoveWorkspace { name, kill: true }]) => name == "a",
        (ids::SESSION_RESTORE_ALL, [Effect::Restore { id }]) => id == OLD,
        (ids::HOOKS_INSTALL, [Effect::Install]) => true,
        _ => false,
    }
}

/// The session a dangerous id is aimed at in these tests (`delete` forgets a
/// closed one; the rest act on the live idle session).
fn target_for(id: ActionId) -> &'static str {
    if id == ids::SESSION_DELETE {
        OLD
    } else {
        TWO
    }
}

#[test]
fn every_dangerous_id_is_in_this_tests_list() {
    let mut from_table: Vec<&str> = META.iter().filter(|m| m.dangerous).map(|m| m.id.as_str()).collect();
    let mut listed: Vec<&str> = DANGEROUS.iter().map(|id| id.as_str()).collect();
    from_table.sort_unstable();
    listed.sort_unstable();
    assert_eq!(from_table, listed, "a new dangerous id needs a row here");
}

#[test]
fn plugin_sources_only_arm_and_a_human_click_confirms() {
    for id in DANGEROUS {
        for source in ["widget", "plugin"] {
            let mut app = fixture();
            let build = |app: &App| match source {
                "widget" => widget(app, target_for(id)),
                _ => plugin(app, target_for(id)),
            };
            let ctx = build(&app);
            assert!(app.dispatch_action_at(id, &ctx, 1000).is_empty(), "{id:?} via {source}: first trigger runs");
            let text = app.router.notices.current(1000).expect("an arm notice").to_owned();
            assert!(text.ends_with("? click again or press y"), "{id:?}: {text}");
            // The same plugin trigger again, and again: still only armed.
            for t in [1100, 1200, 1300] {
                assert!(app.dispatch_action_at(id, &ctx, t).is_empty(), "{id:?} via {source} at {t}");
            }
            // A human click on the armed region confirms exactly once.
            let effects = app.dispatch_action_at(id, &mouse(build(&app)), 1400);
            assert!(confirmed_effect_ok(id, &effects), "{id:?} via {source}: {effects:?}");
            assert!(
                app.dispatch_action_at(id, &mouse(build(&app)), 1500).is_empty(),
                "{id:?}: the arm is consumed, a second human click only starts over"
            );
        }
    }
}

#[test]
fn a_plugin_triggering_ten_times_never_executes() {
    for id in DANGEROUS {
        let mut app = fixture();
        for i in 0..10 {
            let ctx = if i % 2 == 0 { plugin(&app, TWO) } else { widget(&app, TWO) };
            assert!(app.dispatch_action_at(id, &ctx, 1000 + i * 50).is_empty(), "{id:?} #{i}");
        }
    }
}

#[test]
fn y_confirms_a_pending_arm_and_any_other_key_drops_it() {
    let mut app = fixture();
    let ctx = plugin(&app, TWO);
    assert!(app.dispatch_action_at(ids::SESSION_CLOSE, &ctx, 1000).is_empty());
    let effects = key_at(&mut app.router, &mut app.store, "y", 1500);
    assert!(confirmed_effect_ok(ids::SESSION_CLOSE, &effects), "{effects:?}");

    // `y` with nothing armed is just an unbound key.
    assert!(key_at(&mut app.router, &mut app.store, "y", 1600).is_empty());

    // Arm, press something else, then `y`: the arm is gone.
    app.dispatch_action_at(ids::SESSION_CLOSE, &ctx, 2000);
    key_at(&mut app.router, &mut app.store, "z", 2100);
    assert!(key_at(&mut app.router, &mut app.store, "y", 2200).is_empty());
    assert!(!app.router.notices.current(2200).unwrap_or("").contains("click again"), "the arm line is gone");
}

#[test]
fn an_arm_expires_after_the_window() {
    let mut app = fixture();
    let ctx = plugin(&app, TWO);
    app.dispatch_action_at(ids::SESSION_CLOSE, &ctx, 1000);
    assert!(app.dispatch_action_at(ids::SESSION_CLOSE, &mouse(ctx.clone()), 4001).is_empty(), "past 3 s");
    // The expired click did not confirm and did not run: nothing stale armed.
    assert!(key_at(&mut app.router, &mut app.store, "y", 4100).is_empty());
}

#[test]
fn a_click_aimed_at_another_target_does_not_confirm() {
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &widget(&app, TWO), 1000);
    let other = mouse(widget(&app, THREE));
    assert!(app.dispatch_action_at(ids::SESSION_CLOSE, &other, 1100).is_empty());
}

#[test]
fn args_never_redirect_a_core_action() {
    let mut app = fixture();
    let mut ctx = plugin(&app, ONE);
    ctx.args = Some(Arc::new(serde_json::json!({"session": TWO, "workspace": "b", "id": THREE})));
    app.dispatch_action_at(ids::SESSION_CLOSE, &ctx, 1000);
    let notice = app.router.notices.current(1000).unwrap();
    assert!(notice.starts_with("Close one?"), "{notice}");
    // A confirming human trigger carries no args at all; the armed target wins.
    let effects = app.dispatch_action_at(ids::SESSION_CLOSE, &mouse(plugin(&app, ONE)), 1100);
    assert!(matches!(effects.as_slice(), [Effect::Close { id, .. }] if id == ONE), "{effects:?}");
    // The same holds when the confirming context itself carries crafted args.
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, ONE), 1000);
    let mut confirm = mouse(plugin(&app, ONE));
    confirm.args = Some(Arc::new(serde_json::json!({"session": TWO})));
    let effects = app.dispatch_action_at(ids::SESSION_CLOSE, &confirm, 1100);
    assert!(matches!(effects.as_slice(), [Effect::Close { id, .. }] if id == ONE), "{effects:?}");
}

#[test]
fn ids_a_plugin_may_never_invoke_are_refused_and_do_nothing() {
    for m in META.iter().filter(|m| !m.plugin_invocable) {
        for source in ["widget", "plugin"] {
            let mut app = fixture();
            app.store.open_overlay(OverlayKind::Help);
            let before = state_of(&app);
            let ctx = if source == "widget" { widget(&app, TWO) } else { plugin(&app, TWO) };
            assert!(app.dispatch_action_at(m.id, &ctx, 1000).is_empty(), "{:?}", m.id);
            assert_eq!(state_of(&app), before, "{:?} via {source} changed state", m.id);
            assert_eq!(
                app.router.notices.current(1000),
                Some("that action is not available to plugins"),
                "{:?}",
                m.id
            );
            assert!(!app.router.plugin_arm_pending(), "{:?} must not arm", m.id);
        }
    }
    // app.quit in particular never raises a quit effect, however often.
    let mut app = fixture();
    for t in 0..5 {
        assert!(app.dispatch_action_at(ids::APP_QUIT, &plugin(&app, TWO), t).is_empty());
    }
}

#[test]
fn an_unknown_plugin_action_id_is_not_refused() {
    let mut app = fixture();
    let id = ActionId::intern("closer:close-all");
    assert!(meta(id).is_none());
    assert!(app.dispatch_action_at(id, &plugin(&app, TWO), 1000).is_empty());
    assert!(!app.router.plugin_arm_pending());
    assert_eq!(app.router.notices.current(1000), None, "not refused, not armed");
}

#[test]
fn session_delete_arms_for_a_human_too_and_forgets_only_a_closed_session() {
    let mut app = fixture();
    // A human first trigger arms, it does not delete on the spot.
    let key = ActionCtx::from_focus(app.store.state(), ActionSource::Key("D".into())).with_session(OLD.into());
    assert!(app.dispatch_action_at(ids::SESSION_DELETE, &key, 1000).is_empty());
    assert!(app.router.notices.current(1000).unwrap().starts_with("Delete old?"));
    let effects = app.dispatch_action_at(ids::SESSION_DELETE, &mouse(widget(&app, OLD)), 1100);
    assert_eq!(
        effects,
        vec![Effect::Close { id: OLD.into(), label: "old".into(), meta_only: true }],
        "ends in DELETE /api/sessions/<id>?meta=1"
    );

    // A live session is refused with a hint, even after a confirm.
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_DELETE, &widget(&app, TWO), 1000);
    let effects = app.dispatch_action_at(ids::SESSION_DELETE, &mouse(widget(&app, TWO)), 1100);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        app.router.notices.current(1100),
        Some("close it first — delete forgets a closed session")
    );

    // A closed-session record the store no longer lists can still be forgotten.
    let mut app = fixture();
    let gone = "garage/a/long-gone";
    app.dispatch_action_at(ids::SESSION_DELETE, &plugin(&app, gone), 1000);
    let effects = app.dispatch_action_at(ids::SESSION_DELETE, &mouse(plugin(&app, gone)), 1100);
    assert!(matches!(effects.as_slice(), [Effect::Close { id, meta_only: true, .. }] if id == gone), "{effects:?}");
}

#[test]
fn arming_leaves_the_focus_and_the_native_x_machine_alone() {
    let mut app = fixture();
    let focused = app.store.state().focused_session_id.clone();
    app.dispatch_action_at(ids::SESSION_CLOSE, &widget(&app, THREE), 1000);
    assert_eq!(app.store.state().focused_session_id, focused, "a close button does not move focus");
    // The native `x` is unaffected by the plugin arm: it still needs its own
    // two presses (the first key press drops the plugin arm).
    assert!(key_at(&mut app.router, &mut app.store, "x", 1100).is_empty());
    assert_eq!(app.router.notices.current(1100), Some("press x again to close one"));
    let effects = key_at(&mut app.router, &mut app.store, "x", 1200);
    assert!(matches!(effects.as_slice(), [Effect::Close { id, .. }] if id == ONE), "{effects:?}");
}

#[test]
fn a_button_on_an_unfocused_tile_retargets_the_actions_that_act_on_the_focus() {
    let mut app = fixture();
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(ONE));
    app.dispatch_action_at(ids::SESSION_MAXIMIZE, &widget(&app, THREE), 1000);
    assert_eq!(app.store.state().maximized_session_id.as_deref(), Some(THREE));
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(THREE));

    // A workspace-scoped click spawns into the clicked workspace: the focus
    // follows it (the spawn itself is an effect, not run here).
    let mut app = fixture();
    let mut ctx = widget(&app, TWO);
    ctx.workspace = Some("b".into());
    app.dispatch_action_at(ids::WORKSPACE_FOCUS, &ctx, 1000);
    assert_eq!(app.store.state().focused_workspace.as_deref(), Some("b"));
}

#[tokio::test]
async fn two_real_clicks_on_a_plugin_close_button_close_the_tile_it_sits_on() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:1", 0);
    app.store.workspaces_fetched(vec![ws("a"), ws("b")]);
    app.store.sessions_fetched(vec![si("a", "one"), si("b", "three")]);
    app.layout = Some(crate::ui::layout::wall_layout(Rect::new(0, 0, 120, 40), 2, None, false, false));
    let tile = app.layout.as_ref().unwrap().tiles[1];
    let button = Rect::new(tile.x + 4, tile.y, 3, 1);
    app.slot_clicks = vec![SlotClickRegion {
        rect: button,
        target: ClickTarget::new(ids::SESSION_CLOSE).from_plugin("closer"),
        slot: SlotId::TileTitleRight,
        session: Some(THREE.into()),
        workspace: Some("b".into()),
    }];
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(ONE));

    // First click: arms, nothing is closed, focus did not move.
    assert!(!app.handle_click(button.x + 1, button.y));
    assert_eq!(app.router.notices.current(now_ms()), Some("Close three? click again or press y"));
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(ONE));
    assert!(rx.try_recv().is_err(), "no effect ran on the arming click");

    // Second click on the armed region: the close effect runs for THAT tile
    // (the daemon is unreachable here, which the failure notice proves).
    assert!(!app.handle_click(button.x + 1, button.y));
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(matches!(&event, AppEvent::Notice { text, .. } if text.starts_with("close failed")));
}
