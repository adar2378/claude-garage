//! The confirm rule (p24 plugin-host "Confirm rule for dangerous actions"):
//! plugin sources only ARM a dangerous action, a human key or click confirms,
//! a plugin that triggers twice never executes, refused ids never run, and a
//! core action's target comes from the context, never from `args`. An arm
//! remembers where it came from (API request: `y` only; widget click: `y` or
//! its own region), a plugin never replaces a pending arm, and the arm lives
//! only while its prompt is the notice on screen.
use super::*;
use crate::actions::{ActionId, META, meta};
use crate::runtime::router::plugin_gate::ClickConfirm;
use crate::slots::{ClickTarget, RegionId, SlotClickRegion, SlotId};
use ratatui::layout::Rect;

const ONE: &str = "garage/a/one";
const TWO: &str = "garage/a/two";
const OLD: &str = "garage/a/old";
const THREE: &str = "garage/b/three";

fn fixture() -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0");
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

/// The identity of the plugin button that sits on `session`'s tile title.
fn region_of(id: ActionId, session: &str) -> RegionId {
    let x = 10 + session.bytes().map(u16::from).sum::<u16>() % 80;
    SlotClickRegion {
        rect: Rect::new(x, 0, 3, 1),
        target: ClickTarget::new(id).from_plugin("closer"),
        slot: SlotId::TileTitleRight,
        session: Some(session.to_owned()),
        workspace: session.split('/').nth(1).map(str::to_owned),
    }
    .identity()
}

fn widget(app: &App, session: &str) -> ActionCtx {
    let mut ctx = ActionCtx::from_focus(
        app.store.state(),
        ActionSource::Widget {
            plugin: "closer".into(),
            slot: SlotId::TileTitleRight,
        },
    );
    ctx.session = Some(session.to_owned());
    ctx.workspace = Some(session.split('/').nth(1).unwrap().to_owned());
    ctx
}

/// A widget click on the region of `id`'s button, as `click_ctx` builds it.
fn widget_click(app: &App, id: ActionId, session: &str) -> ActionCtx {
    let mut ctx = widget(app, session);
    ctx.region = Some(region_of(id, session));
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
        (
            ids::SESSION_CLOSE,
            [
                Effect::Close {
                    id,
                    meta_only: false,
                    ..
                },
            ],
        ) => id == TWO,
        (
            ids::SESSION_DELETE,
            [
                Effect::Close {
                    id,
                    meta_only: true,
                    ..
                },
            ],
        ) => id == OLD,
        (
            ids::SESSION_RESTART,
            [
                Effect::RestartSession {
                    id, force: false, ..
                },
            ],
        ) => id == TWO,
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
    if id == ids::SESSION_DELETE { OLD } else { TWO }
}

#[test]
fn every_dangerous_id_is_in_this_tests_list() {
    let mut from_table: Vec<&str> = META
        .iter()
        .filter(|m| m.dangerous)
        .map(|m| m.id.as_str())
        .collect();
    let mut listed: Vec<&str> = DANGEROUS.iter().map(|id| id.as_str()).collect();
    from_table.sort_unstable();
    listed.sort_unstable();
    assert_eq!(from_table, listed, "a new dangerous id needs a row here");
}

#[test]
fn plugin_sources_only_arm_and_y_confirms_exactly_once() {
    for id in DANGEROUS {
        for source in ["widget", "plugin"] {
            let mut app = fixture();
            let build = |app: &App| match source {
                "widget" => widget_click(app, id, target_for(id)),
                _ => plugin(app, target_for(id)),
            };
            let ctx = build(&app);
            assert!(
                app.dispatch_action_at(id, &ctx, 1000).is_empty(),
                "{id:?} via {source}: first trigger runs"
            );
            let text = app
                .router
                .notices
                .current(1000)
                .expect("an arm notice")
                .to_owned();
            assert!(text.contains("click again or press y"), "{id:?}: {text}");
            // The same plugin trigger again, and again: still only armed.
            for t in [1100, 1200, 1300] {
                assert!(
                    app.dispatch_action_at(id, &ctx, t).is_empty(),
                    "{id:?} via {source} at {t}"
                );
            }
            // The human's `y` confirms exactly once.
            let effects = key_at(&mut app.router, &mut app.store, "y", 1400);
            assert!(
                confirmed_effect_ok(id, &effects),
                "{id:?} via {source}: {effects:?}"
            );
            assert!(
                key_at(&mut app.router, &mut app.store, "y", 1500).is_empty(),
                "{id:?}: the arm is consumed"
            );
        }
    }
}

#[test]
fn a_widget_arm_is_confirmed_by_a_click_on_its_exact_region_only() {
    let mut app = fixture();
    let armed = widget_click(&app, ids::SESSION_CLOSE, TWO);
    app.dispatch_action_at(ids::SESSION_CLOSE, &armed, 1000);
    let region = armed.region.clone().unwrap();

    // Same action and target, different rect: not the region that armed it.
    let mut moved = region.clone();
    moved.rect.x += 1;
    assert!(matches!(
        app.router.plugin_confirm_click(Some(&moved), 1100),
        ClickConfirm::Cancelled
    ));
    assert!(
        !app.router.plugin_arm_pending(),
        "a wrong-region click cancels the arm"
    );

    // Same rect and target, different item identity (another plugin's button).
    app.dispatch_action_at(ids::SESSION_CLOSE, &armed, 2000);
    let mut other_item = region.clone();
    other_item.plugin = Some("other".into());
    assert!(matches!(
        app.router.plugin_confirm_click(Some(&other_item), 2100),
        ClickConfirm::Cancelled
    ));

    // Same everything but the text it shows: a relabelled item is another
    // region, however alike (a plugin cannot swap the words under the click).
    app.dispatch_action_at(ids::SESSION_CLOSE, &armed, 2500);
    let mut relabelled = region.clone();
    relabelled.label ^= 1;
    assert!(matches!(
        app.router.plugin_confirm_click(Some(&relabelled), 2900),
        ClickConfirm::Cancelled
    ));

    // The exact region confirms, and the confirmed run closes THAT session.
    app.dispatch_action_at(ids::SESSION_CLOSE, &armed, 3000);
    app.router.mark_prompt_drawn(3050);
    let ClickConfirm::Run(action) = app.router.plugin_confirm_click(Some(&region), 3400) else {
        panic!("the armed region confirms");
    };
    let (id, ctx) = *action;
    let effects = app.dispatch_action_at(id, &ctx, 3400);
    assert!(
        confirmed_effect_ok(ids::SESSION_CLOSE, &effects),
        "{effects:?}"
    );
}

#[test]
fn a_click_on_the_armed_region_confirms_only_after_the_guard_and_a_drawn_prompt() {
    let mut app = fixture();
    let armed = widget_click(&app, ids::SESSION_CLOSE, TWO);
    let region = armed.region.clone().unwrap();

    // Past 300 ms but no frame ever showed the prompt: swallowed, arm kept.
    app.dispatch_action_at(ids::SESSION_CLOSE, &armed, 1000);
    assert!(matches!(
        app.router.plugin_confirm_click(Some(&region), 1500),
        ClickConfirm::Swallow
    ));
    assert!(app.router.plugin_arm_pending());

    // Drawn, but a double-click lands inside the guard window: swallowed.
    app.router.mark_prompt_drawn(1100);
    assert!(matches!(
        app.router.plugin_confirm_click(Some(&region), 1200),
        ClickConfirm::Swallow
    ));
    assert!(
        app.router.plugin_arm_pending(),
        "a swallowed click does not cancel"
    );

    // Drawn and past the window: confirms.
    assert!(matches!(
        app.router.plugin_confirm_click(Some(&region), 1300),
        ClickConfirm::Run(_)
    ));
}

#[test]
fn a_prompt_replaced_before_it_was_drawn_is_not_counted_as_drawn() {
    let mut app = fixture();
    let armed = widget_click(&app, ids::SESSION_CLOSE, TWO);
    let region = armed.region.clone().unwrap();
    app.dispatch_action_at(ids::SESSION_CLOSE, &armed, 1000);
    // A frame is drawn while another notice covers the prompt: the arm is
    // gone anyway (its prompt is no longer on screen).
    app.router.notices.show("something else", 3000, 1050);
    app.router.mark_prompt_drawn(1060);
    assert!(matches!(
        app.router.plugin_confirm_click(Some(&region), 1400),
        ClickConfirm::Cancelled
    ));
}

#[test]
fn an_api_arm_cannot_be_confirmed_by_any_mouse_click() {
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, TWO), 1000);
    // Even a click on a region that looks exactly like the button.
    let lookalike = region_of(ids::SESSION_CLOSE, TWO);
    assert!(matches!(
        app.router.plugin_confirm_click(Some(&lookalike), 1100),
        ClickConfirm::Cancelled
    ));
    assert!(!app.router.plugin_arm_pending());
    assert!(
        key_at(&mut app.router, &mut app.store, "y", 1500).is_empty(),
        "and the click cancelled it"
    );
}

#[test]
fn a_core_click_never_confirms_a_plugin_arm() {
    let mut app = fixture();
    app.dispatch_action_at(
        ids::SESSION_CLOSE,
        &widget_click(&app, ids::SESSION_CLOSE, TWO),
        1000,
    );
    // A core click target for the very same action and session.
    let effects = app.dispatch_action_at(ids::SESSION_CLOSE, &mouse(widget(&app, TWO)), 1400);
    assert!(effects.is_empty(), "{effects:?}");
    // Whatever it started (the native close arm), the plugin arm is gone.
    assert!(key_at(&mut app.router, &mut app.store, "y", 1800).is_empty());
}

#[test]
fn a_replaced_notice_cancels_the_arm() {
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, TWO), 1000);
    app.router
        .notices
        .show("something else happened", 3000, 1100);
    assert!(
        key_at(&mut app.router, &mut app.store, "y", 1500).is_empty(),
        "the prompt is gone, so is the arm"
    );
    assert!(!app.router.plugin_arm_pending());
}

#[test]
fn a_plugin_cannot_replace_or_refresh_a_pending_arm() {
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, TWO), 1000);
    // Another target, another action, the same one later: all refused, and
    // the prompt on screen is still the first one.
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, THREE), 2000);
    app.dispatch_action_at(
        ids::DAEMON_RESTART,
        &widget_click(&app, ids::DAEMON_RESTART, TWO),
        2100,
    );
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, TWO), 3500);
    assert_eq!(
        app.router.notices.current(2900),
        Some("Close a/two? click again or press y")
    );
    // Not refreshed: the 3 s window still counts from the first prompt.
    assert!(key_at(&mut app.router, &mut app.store, "y", 4100).is_empty());

    // A pending arm, still live, confirms the ORIGINAL target.
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, TWO), 1000);
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, THREE), 1500);
    let effects = key_at(&mut app.router, &mut app.store, "y", 2000);
    assert!(
        confirmed_effect_ok(ids::SESSION_CLOSE, &effects),
        "{effects:?}"
    );

    // A human's own arm is not replaceable by a plugin either.
    let mut app = fixture();
    let key = ActionCtx::from_focus(app.store.state(), ActionSource::Key("D".into()))
        .with_session(OLD.into());
    app.dispatch_action_at(ids::SESSION_DELETE, &key, 1000);
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, TWO), 1100);
    assert!(
        app.router
            .notices
            .current(1100)
            .unwrap()
            .starts_with("Delete a/old?")
    );
}

#[test]
fn y_inside_the_guard_window_is_ignored_and_the_arm_survives() {
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, TWO), 1000);
    assert!(
        key_at(&mut app.router, &mut app.store, "y", 1299).is_empty(),
        "299 ms: a racing key"
    );
    assert!(app.router.plugin_arm_pending());
    assert_eq!(
        app.router.notices.current(1299),
        Some("Close a/two? click again or press y")
    );
    let effects = key_at(&mut app.router, &mut app.store, "y", 1300);
    assert!(
        confirmed_effect_ok(ids::SESSION_CLOSE, &effects),
        "300 ms: confirmed {effects:?}"
    );
}

#[test]
fn the_prompt_names_workspace_and_label() {
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, THREE), 1000);
    assert_eq!(
        app.router.notices.current(1000),
        Some("Close b/three? click again or press y")
    );
}

#[test]
fn a_forged_confirmed_source_without_the_routers_token_only_arms() {
    let mut app = fixture();
    let mut forged = plugin(&app, TWO);
    forged.source = ActionSource::Plugin {
        plugin: "closer".into(),
        confirmed: true,
    };
    assert!(
        app.dispatch_action_at(ids::SESSION_CLOSE, &forged, 1000)
            .is_empty()
    );
    assert!(
        app.router.plugin_arm_pending(),
        "treated as a plugin trigger: armed, not run"
    );
}

#[test]
fn a_restart_prompt_warns_when_the_session_is_busy_and_forces_exactly_like_r_r() {
    // `one` is working: the warning wording, and the confirm forces.
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_RESTART, &plugin(&app, ONE), 1000);
    assert_eq!(
        app.router.notices.current(1000),
        Some("a/one is working — click again or press y to restart anyway")
    );
    let effects = key_at(&mut app.router, &mut app.store, "y", 1400);
    assert!(
        matches!(effects.as_slice(), [Effect::RestartSession { id, force: true, .. }] if id == ONE),
        "{effects:?}"
    );
    let after = app.router.notices.current(1400).unwrap();
    assert!(
        after.starts_with("restarting ") && after.ends_with("— resumes the conversation"),
        "{after}"
    );

    // Force is what the prompt warned about: a session that went idle between
    // the prompt and the confirm is still restarted with force (D2).
    let mut app = fixture();
    app.dispatch_action_at(
        ids::SESSION_RESTART,
        &widget_click(&app, ids::SESSION_RESTART, ONE),
        1000,
    );
    app.store
        .sessions_fetched(vec![si_status("a", "one", "idle"), si("b", "three")]);
    let effects = key_at(&mut app.router, &mut app.store, "y", 1400);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::RestartSession { force: true, .. }]
        ),
        "{effects:?}"
    );

    // An idle session gets the plain prompt and no force.
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_RESTART, &plugin(&app, TWO), 1000);
    assert_eq!(
        app.router.notices.current(1000),
        Some("Restart a/two? click again or press y")
    );
}

#[test]
fn a_row_that_is_not_a_sessions_own_never_falls_back_to_the_focused_session() {
    let mut app = fixture();
    let focused = app.store.state().focused_session_id.clone();
    assert!(focused.is_some());
    // What `click_ctx` builds for an extra row with no `about_session`.
    let mut row = widget(&app, TWO);
    row.session = None;
    for id in [
        ids::SESSION_CLOSE,
        ids::SESSION_RESTART,
        ids::SESSION_DELETE,
        ids::SESSION_MAXIMIZE,
    ] {
        assert!(app.dispatch_action_at(id, &row, 1000).is_empty(), "{id:?}");
        assert_eq!(
            app.router.notices.current(1000),
            Some("nothing to act on"),
            "{id:?}"
        );
        assert!(!app.router.plugin_arm_pending(), "{id:?}");
    }
    assert_eq!(app.store.state().maximized_session_id, None);
    assert_eq!(app.store.state().focused_session_id, focused);
    // Its workspace is still a valid target.
    app.dispatch_action_at(ids::WORKSPACE_REMOVE, &row, 2000);
    assert_eq!(
        app.router.notices.current(2000),
        Some("Remove workspace a? click again or press y")
    );
}

#[test]
fn a_plugin_triggering_ten_times_never_executes() {
    for id in DANGEROUS {
        let mut app = fixture();
        for i in 0..10 {
            let ctx = if i % 2 == 0 {
                plugin(&app, TWO)
            } else {
                widget(&app, TWO)
            };
            assert!(
                app.dispatch_action_at(id, &ctx, 1000 + i * 50).is_empty(),
                "{id:?} #{i}"
            );
        }
    }
}

#[test]
fn y_confirms_a_pending_arm_and_any_other_key_drops_it() {
    let mut app = fixture();
    let ctx = plugin(&app, TWO);
    assert!(
        app.dispatch_action_at(ids::SESSION_CLOSE, &ctx, 1000)
            .is_empty()
    );
    let effects = key_at(&mut app.router, &mut app.store, "y", 1500);
    assert!(
        confirmed_effect_ok(ids::SESSION_CLOSE, &effects),
        "{effects:?}"
    );

    // `y` with nothing armed is just an unbound key.
    assert!(key_at(&mut app.router, &mut app.store, "y", 1600).is_empty());

    // Arm, press something else, then `y`: the arm is gone.
    app.dispatch_action_at(ids::SESSION_CLOSE, &ctx, 2000);
    key_at(&mut app.router, &mut app.store, "z", 2100);
    assert!(key_at(&mut app.router, &mut app.store, "y", 2200).is_empty());
    assert!(
        !app.router
            .notices
            .current(2200)
            .unwrap_or("")
            .contains("click again"),
        "the arm line is gone"
    );
}

#[test]
fn an_arm_expires_after_the_window() {
    let mut app = fixture();
    let ctx = plugin(&app, TWO);
    app.dispatch_action_at(ids::SESSION_CLOSE, &ctx, 1000);
    assert!(
        key_at(&mut app.router, &mut app.store, "y", 4001).is_empty(),
        "past 3 s"
    );
    // The expired `y` did not confirm and nothing stale stays armed.
    assert!(!app.router.plugin_arm_pending());
}

#[test]
fn args_never_redirect_a_core_action() {
    let mut app = fixture();
    let mut ctx = plugin(&app, ONE);
    ctx.args = Some(Arc::new(
        serde_json::json!({"session": TWO, "workspace": "b", "id": THREE}),
    ));
    app.dispatch_action_at(ids::SESSION_CLOSE, &ctx, 1000);
    let notice = app.router.notices.current(1000).unwrap();
    assert!(notice.starts_with("Close a/one?"), "{notice}");
    // A confirming `y` carries no args at all; the armed target wins.
    let effects = key_at(&mut app.router, &mut app.store, "y", 1400);
    assert!(
        matches!(effects.as_slice(), [Effect::Close { id, .. }] if id == ONE),
        "{effects:?}"
    );
    // The same holds when the confirming context itself carries crafted args.
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_CLOSE, &plugin(&app, ONE), 1000);
    let mut confirm = plugin(&app, ONE);
    confirm.args = Some(Arc::new(serde_json::json!({"session": TWO})));
    app.dispatch_action_at(ids::SESSION_CLOSE, &confirm, 1100); // refused while pending
    let effects = key_at(&mut app.router, &mut app.store, "y", 1400);
    assert!(
        matches!(effects.as_slice(), [Effect::Close { id, .. }] if id == ONE),
        "{effects:?}"
    );
}

#[test]
fn ids_a_plugin_may_never_invoke_are_refused_and_do_nothing() {
    for m in META.iter().filter(|m| !m.plugin_invocable) {
        for source in ["widget", "plugin"] {
            let mut app = fixture();
            app.store.open_overlay(OverlayKind::Help);
            let before = state_of(&app);
            let ctx = if source == "widget" {
                widget(&app, TWO)
            } else {
                plugin(&app, TWO)
            };
            assert!(
                app.dispatch_action_at(m.id, &ctx, 1000).is_empty(),
                "{:?}",
                m.id
            );
            assert_eq!(
                state_of(&app),
                before,
                "{:?} via {source} changed state",
                m.id
            );
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
        assert!(
            app.dispatch_action_at(ids::APP_QUIT, &plugin(&app, TWO), t)
                .is_empty()
        );
    }
}

#[test]
fn an_unknown_plugin_action_id_is_not_refused() {
    let mut app = fixture();
    let id = ActionId::intern("closer:close-all");
    assert!(meta(id).is_none());
    assert!(
        app.dispatch_action_at(id, &plugin(&app, TWO), 1000)
            .is_empty()
    );
    assert!(!app.router.plugin_arm_pending());
    assert_eq!(
        app.router.notices.current(1000),
        None,
        "not refused, not armed"
    );
}

#[test]
fn session_delete_arms_for_a_human_too_and_a_live_tile_is_killed_with_no_history() {
    let mut app = fixture();
    // A human first trigger arms, it does not delete on the spot.
    let key = ActionCtx::from_focus(app.store.state(), ActionSource::Key("D".into()))
        .with_session(OLD.into());
    assert!(
        app.dispatch_action_at(ids::SESSION_DELETE, &key, 1000)
            .is_empty()
    );
    assert!(
        app.router
            .notices
            .current(1000)
            .unwrap()
            .starts_with("Delete a/old?")
    );
    let effects = app.dispatch_action_at(ids::SESSION_DELETE, &mouse(widget(&app, OLD)), 1100);
    assert_eq!(
        effects,
        vec![Effect::Close {
            id: OLD.into(),
            label: "old".into(),
            meta_only: true
        }],
        "ends in DELETE /api/sessions/<id>?meta=1"
    );

    // A live session arms with the no-way-back wording, and a confirm kills it
    // for good (DELETE ?forget=1), it does not just refuse.
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_DELETE, &widget(&app, TWO), 1000);
    assert_eq!(
        app.router.notices.current(1000),
        Some("Delete a/two? (kills it, no history) — click again or press y")
    );
    let effects = key_at(&mut app.router, &mut app.store, "y", 1400);
    assert_eq!(
        effects,
        vec![Effect::Forget {
            id: TWO.into(),
            label: "two".into()
        }],
        "{effects:?}"
    );

    // A closed-session record the store no longer lists can still be forgotten.
    let mut app = fixture();
    let gone = "garage/a/long-gone";
    app.dispatch_action_at(ids::SESSION_DELETE, &plugin(&app, gone), 1000);
    assert_eq!(
        app.router.notices.current(1000),
        Some("Delete a/long-gone? click again or press y")
    );
    let effects = key_at(&mut app.router, &mut app.store, "y", 1400);
    assert!(
        matches!(effects.as_slice(), [Effect::Close { id, meta_only: true, .. }] if id == gone),
        "{effects:?}"
    );
}

#[test]
fn arming_leaves_the_focus_and_the_native_x_machine_alone() {
    let mut app = fixture();
    let focused = app.store.state().focused_session_id.clone();
    app.dispatch_action_at(ids::SESSION_CLOSE, &widget(&app, THREE), 1000);
    assert_eq!(
        app.store.state().focused_session_id,
        focused,
        "a close button does not move focus"
    );
    // The native `x` is unaffected by the plugin arm: it still needs its own
    // two presses (the first key press drops the plugin arm).
    assert!(key_at(&mut app.router, &mut app.store, "x", 1100).is_empty());
    assert_eq!(
        app.router.notices.current(1100),
        Some("press x again to close one")
    );
    let effects = key_at(&mut app.router, &mut app.store, "x", 1200);
    assert!(
        matches!(effects.as_slice(), [Effect::Close { id, .. }] if id == ONE),
        "{effects:?}"
    );
}

#[test]
fn a_button_on_an_unfocused_tile_retargets_the_actions_that_act_on_the_focus() {
    let mut app = fixture();
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(ONE));
    app.dispatch_action_at(ids::SESSION_MAXIMIZE, &widget(&app, THREE), 1000);
    assert_eq!(
        app.store.state().maximized_session_id.as_deref(),
        Some(THREE)
    );
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
    let mut app = App::new(tx, "http://127.0.0.1:1");
    app.store.workspaces_fetched(vec![ws("a"), ws("b")]);
    app.store
        .sessions_fetched(vec![si("a", "one"), si("b", "three")]);
    app.layout = Some(crate::ui::layout::wall_layout(
        Rect::new(0, 0, 120, 40),
        2,
        None,
        false,
        false,
    ));
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
    assert_eq!(
        app.router.notices.current(now_ms()),
        Some("Close b/three? click again or press y")
    );
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(ONE));
    assert!(rx.try_recv().is_err(), "no effect ran on the arming click");

    // A double-click is swallowed: not confirmed, not cancelled.
    assert!(!app.handle_click(button.x + 1, button.y));
    assert!(
        rx.try_recv().is_err(),
        "a hasty second click does not confirm"
    );
    assert!(app.router.plugin_arm_pending());

    // The prompt is drawn and the guard window passes ...
    let regions = app.slot_clicks.clone();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| app.draw_frame(f, now_ms())).unwrap();
    app.slot_clicks = regions; // the draw rebuilt them without the test's plugin button
    tokio::time::sleep(Duration::from_millis(350)).await;

    // ... then a click on the armed region: the close effect runs for THAT tile
    // (the daemon is unreachable here, which the failure notice proves).
    assert!(!app.handle_click(button.x + 1, button.y));
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(&event, AppEvent::Notice { text, .. } if text.starts_with("close failed")));
}

#[test]
fn a_click_elsewhere_cancels_the_arm_and_the_button_then_only_re_arms() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:1");
    app.store.workspaces_fetched(vec![ws("a"), ws("b")]);
    app.store
        .sessions_fetched(vec![si("a", "one"), si("b", "three")]);
    app.layout = Some(crate::ui::layout::wall_layout(
        Rect::new(0, 0, 120, 40),
        2,
        None,
        false,
        false,
    ));
    let tile = app.layout.as_ref().unwrap().tiles[1];
    let button = Rect::new(tile.x + 4, tile.y, 3, 1);
    app.slot_clicks = vec![SlotClickRegion {
        rect: button,
        target: ClickTarget::new(ids::SESSION_CLOSE).from_plugin("closer"),
        slot: SlotId::TileTitleRight,
        session: Some(THREE.into()),
        workspace: Some("b".into()),
    }];
    let armed = |app: &App| {
        app.router.notices.current(now_ms()) == Some("Close b/three? click again or press y")
    };
    assert!(!app.handle_click(button.x, button.y));
    assert!(armed(&app));
    // A click on a core target (the tile body) cancels it ...
    assert!(!app.handle_click(tile.x + 12, tile.y + 3));
    assert!(!app.router.plugin_arm_pending());
    // ... so the next click on the button arms afresh instead of closing.
    assert!(!app.handle_click(button.x, button.y));
    assert!(armed(&app));
}

#[test]
fn a_plugin_cannot_move_the_focus_while_the_user_is_engaged() {
    for id in [
        ids::SESSION_FOCUS,
        ids::SESSION_ENGAGE,
        ids::SESSION_RESTORE,
        ids::SESSION_MAXIMIZE,
        ids::WORKSPACE_FOCUS,
    ] {
        let mut app = fixture();
        app.store.engage();
        assert_eq!(app.store.state().focused_session_id.as_deref(), Some(ONE));
        let mut ctx = plugin(&app, THREE);
        ctx.workspace = Some("b".into());
        let effects = app.dispatch_action_at(id, &ctx, 1000);
        assert!(effects.is_empty(), "{id:?}: {effects:?}");
        // Typing still goes to the session the user engaged.
        let state = app.store.state();
        assert_eq!(state.layer, KeyLayer::Engaged, "{id:?}");
        assert_eq!(state.focused_session_id.as_deref(), Some(ONE), "{id:?}");
        assert_eq!(state.focused_workspace.as_deref(), Some("a"), "{id:?}");
        let notice = app
            .router
            .notices
            .current(1000)
            .expect("a notice says what was refused");
        assert!(
            notice.contains("plugin closer") && notice.contains("Ctrl+G"),
            "{notice}"
        );
    }
}

#[test]
fn a_plugin_may_still_act_on_the_engaged_session_and_move_focus_when_not_engaged() {
    let mut app = fixture();
    app.store.engage();
    app.dispatch_action_at(ids::SESSION_MAXIMIZE, &plugin(&app, ONE), 1000);
    assert_eq!(
        app.store.state().layer,
        KeyLayer::Engaged,
        "its own session is not a switch"
    );

    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_FOCUS, &plugin(&app, THREE), 1000);
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(THREE));
}

#[test]
fn a_human_click_on_a_plugin_button_still_leaves_the_engaged_session() {
    let mut app = fixture();
    app.store.engage();
    app.dispatch_action_at(ids::SESSION_MAXIMIZE, &widget(&app, THREE), 1000);
    assert_eq!(app.store.state().layer, KeyLayer::Garage);
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(THREE));
}

#[test]
fn a_plugin_may_never_engage_a_session_but_a_click_on_its_button_may() {
    // From the garage layer, for the focused session and for another one.
    for session in [ONE, TWO] {
        let mut app = fixture();
        let effects = app.dispatch_action_at(ids::SESSION_ENGAGE, &plugin(&app, session), 1000);
        assert!(effects.is_empty());
        assert_eq!(
            app.store.state().layer,
            KeyLayer::Garage,
            "{session}: typing never moves on a plugin's say-so"
        );
        let notice = app.router.notices.current(1000).expect("the user is told");
        assert!(
            notice.contains("plugin closer") && notice.contains("only you can engage"),
            "{notice}"
        );
    }
    // The same id from a human click on a plugin button lands like any click.
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_ENGAGE, &widget(&app, TWO), 1000);
    assert_eq!(app.store.state().layer, KeyLayer::Engaged);
}

#[test]
fn a_burst_of_plugin_focus_moves_lands_once() {
    let mut app = fixture();
    app.dispatch_action_at(ids::SESSION_FOCUS, &plugin(&app, TWO), 1000);
    assert_eq!(app.store.state().focused_session_id.as_deref(), Some(TWO));
    for t in [1010, 1100, 1400] {
        app.dispatch_action_at(ids::SESSION_FOCUS, &plugin(&app, THREE), t);
        assert_eq!(
            app.store.state().focused_session_id.as_deref(),
            Some(TWO),
            "dropped at {t}"
        );
    }
    // Other plugin ids are not focus moves; a human is never limited.
    app.dispatch_action_at(ids::SESSION_FOCUS, &widget(&app, THREE), 1410);
    assert_eq!(
        app.store.state().focused_session_id.as_deref(),
        Some(THREE),
        "a click is not a plugin burst"
    );
    app.dispatch_action_at(ids::SESSION_FOCUS, &plugin(&app, ONE), 1600);
    assert_eq!(
        app.store.state().focused_session_id.as_deref(),
        Some(ONE),
        "after the gap it lands again"
    );
}

#[test]
fn a_plugin_sourced_jump_blocked_focuses_the_blocked_session_but_never_engages() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0");
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store
        .sessions_fetched(vec![si("a", "one"), si_status("a", "stuck", "needs-input")]);
    let stuck = "garage/a/stuck";
    app.dispatch_action_at(ids::JUMP_BLOCKED, &plugin(&app, ONE), 1000);
    let state = app.store.state();
    assert_eq!(
        state.layer,
        KeyLayer::Garage,
        "typing never moves on a plugin's say-so"
    );
    assert_eq!(
        state.focused_session_id.as_deref(),
        Some(stuck),
        "the new focus stays"
    );
    let notice = app.router.notices.current(1000).expect("the user is told");
    assert!(
        notice.contains("plugin closer tried to put you into a session")
            && notice.contains("press Enter to engage"),
        "{notice}"
    );
}

#[test]
fn a_human_jump_blocked_still_lands_engaged() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0");
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store
        .sessions_fetched(vec![si("a", "one"), si_status("a", "stuck", "needs-input")]);
    let ctx = ActionCtx::from_focus(app.store.state(), ActionSource::Key("a".into()));
    app.dispatch_action_at(ids::JUMP_BLOCKED, &ctx, 1000);
    assert_eq!(app.store.state().layer, KeyLayer::Engaged);
    assert_eq!(
        app.store.state().focused_session_id.as_deref(),
        Some("garage/a/stuck")
    );
}

#[test]
fn a_plugin_jump_blocked_through_the_open_triage_queue_never_engages() {
    use crate::state::wall_state::OverlayKind;
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0");
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store
        .sessions_fetched(vec![si("a", "one"), si_status("a", "stuck", "needs-input")]);
    app.store.open_overlay(OverlayKind::TriageQueue);
    app.dispatch_action_at(ids::JUMP_BLOCKED, &plugin(&app, ONE), 1000);
    let state = app.store.state();
    assert_ne!(state.layer, KeyLayer::Engaged);
    assert_eq!(state.focused_session_id.as_deref(), Some("garage/a/stuck"));
    let notice = app.router.notices.current(1000).expect("the user is told");
    assert!(notice.contains("press Enter to engage"), "{notice}");
}
