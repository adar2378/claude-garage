//! The plugin host as the App sees it: a listing and widget content in, slot
//! items, click regions, keys and effects out. No daemon: events are applied
//! directly, as the SSE task would send them.
use super::*;
use crate::actions::ids;
use crate::slots::{SlotId, SlotRegistry};
use garage_protocol::plugins::{PluginList, UiActionFrame, WidgetsSnapshot};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde_json::json;

fn drawn(app: &mut App, width: u16, height: u16, now: i64) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| app.draw_frame(f, now)).unwrap();
    terminal.backend().buffer().clone()
}

fn app() -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0", 0);
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store.sessions_fetched(vec![si("a", "one"), si_status("a", "two", "idle")]);
    app
}

fn listing(v: serde_json::Value) -> PluginList {
    serde_json::from_value(json!({ "plugins": v })).unwrap()
}

fn bt(extra: serde_json::Value) -> serde_json::Value {
    let mut p = json!({
        "id": "bt", "name": "Bt", "enabled": true, "active": true,
        "actions": [
            {"id": "bt:go", "title": "Go", "default_key": "u", "contexts": ["session"]},
            {"id": "bt:ask", "title": "Ask", "default_key": "i", "contexts": ["global"], "confirm": true},
            {"id": "bt:look", "title": "Look", "contexts": ["global"], "pane": "details"}
        ],
        "buttons": [], "widgets": [{"id": "bar", "slot": "strip.right"}], "panes": []
    });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

fn strip_widget(rev: u64, text: &str, click: &str) -> WidgetsSnapshot {
    serde_json::from_value(json!({"plugins": {"bt": {"rev": rev, "widgets": {"bar": {"slot": "strip.right", "scopes": {"*": [
        {"priority": 5, "segments": [{"text": text, "click": {"action": click, "args": {"n": 1}}}]}
    ]}}}}}})).unwrap()
}

fn key(app: &mut App, k: &str, now: i64) -> Vec<Effect> {
    let outcome = app.router.on_garage_key(&mut app.store, &app.keymap, k, now);
    let mut effects = outcome.effects;
    if let Some((id, ctx)) = outcome.action {
        effects.extend(app.dispatch_action_at(id, &ctx, now));
    }
    effects
}

#[test]
fn an_empty_plugin_store_draws_exactly_what_the_core_registry_draws() {
    for (w, h) in [(120, 40), (60, 20)] {
        let mut with_providers = app();
        let mut core_only = app();
        core_only.slots = SlotRegistry::core();
        assert_eq!(drawn(&mut with_providers, w, h, 5_000), drawn(&mut core_only, w, h, 5_000), "{w}x{h}");
        // And an installed plugin that pushed nothing changes nothing either.
        with_providers.apply(AppEvent::Plugins(listing(json!([bt(json!({}))]))));
        assert_eq!(drawn(&mut with_providers, w, h, 5_000), drawn(&mut core_only, w, h, 5_000), "{w}x{h} with an idle plugin");
    }
}

#[test]
fn a_pushed_widget_is_drawn_clickable_and_a_click_runs_the_plugins_action_with_its_args() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({}))]))));
    app.apply(AppEvent::PluginWidgets { snapshot: strip_widget(1, "GO!", "bt:go"), full: true });
    let buf = drawn(&mut app, 120, 40, 1_000);
    let region = app.slot_clicks.iter().find(|r| r.target.plugin.as_deref() == Some("bt")).expect("the widget's click region").clone();
    let text: String = (region.rect.x..region.rect.right()).map(|x| buf[(x, region.rect.y)].symbol()).collect();
    assert_eq!(text, "GO!");
    assert_eq!(region.target.action.as_str(), "bt:go");

    let ctx = crate::ui::hit_targets::click_ctx(app.store.state(), &region, region.rect.x, region.rect.y);
    let effects = app.dispatch_action_at(region.target.action, &ctx, 2_000);
    let [Effect::RunPluginAction { plugin, action, body }] = effects.as_slice() else { panic!("{effects:?}") };
    assert_eq!((plugin.as_str(), action.as_str()), ("bt", "go"));
    assert_eq!(body["trigger"], json!({"kind": "widget", "key": null, "slot": "strip.right", "args": {"n": 1}}));
    assert!(!body.to_string().contains("token"));
}

#[test]
fn only_the_changed_plugins_content_is_replaced_and_a_disabled_plugin_vanishes() {
    let mut app = app();
    let mut second = bt(json!({"id": "other"}));
    second["actions"] = json!([]);
    second["widgets"] = json!([{"id": "bar", "slot": "strip.right"}]);
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({})), second]))));
    let both: WidgetsSnapshot = serde_json::from_value(json!({"plugins": {
        "bt": {"rev": 1, "widgets": {"bar": {"slot": "strip.right", "scopes": {"*": [{"segments": [{"text": "AAA"}]}]}}}},
        "other": {"rev": 1, "widgets": {"bar": {"slot": "strip.right", "scopes": {"*": [{"segments": [{"text": "BBB"}]}]}}}},
    }})).unwrap();
    app.apply(AppEvent::PluginWidgets { snapshot: both, full: true });
    let row0 = |app: &mut App| (0..120).map(|x| drawn(app, 120, 40, 1_000)[(x, 39)].symbol().to_owned()).collect::<String>();
    let all = row0(&mut app);
    assert!(all.contains("AAA") && all.contains("BBB"), "{all}");
    // A `widgets` frame for `other` alone leaves `bt` as it was.
    let only_other: WidgetsSnapshot = serde_json::from_value(json!({"plugins": {
        "other": {"rev": 2, "widgets": {"bar": {"slot": "strip.right", "scopes": {"*": [{"segments": [{"text": "CCC"}]}]}}}},
    }})).unwrap();
    app.apply(AppEvent::PluginWidgets { snapshot: only_other, full: false });
    let now = row0(&mut app);
    assert!(now.contains("AAA") && now.contains("CCC") && !now.contains("BBB"), "{now}");
    assert_eq!(app.plugins.borrow().rev("other"), Some(2));
    // Disabling bt drops its content with the next listing.
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({"active": false, "enabled": false})), bt(json!({"id": "other", "actions": []}))]))));
    let gone = row0(&mut app);
    assert!(!gone.contains("AAA") && gone.contains("CCC"), "{gone}");
}

#[test]
fn a_widget_cannot_click_another_plugins_action_or_one_nobody_declared() {
    let mut app = app();
    let mut victim = bt(json!({"id": "victim"}));
    victim["actions"] = json!([{"id": "victim:poke", "title": "Poke", "contexts": ["global"]}]);
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({})), victim]))));
    for action in ["victim:poke", "bt:ghost", "app.quit-not-a-real-id"] {
        app.apply(AppEvent::PluginWidgets { snapshot: strip_widget(1, "GO!", action), full: true });
        drawn(&mut app, 120, 40, 1_000);
        assert!(app.slot_clicks.iter().all(|r| r.target.plugin.is_none()), "{action}: no plugin click region");
        let row: String = (0..120).map(|x| drawn(&mut app, 120, 40, 1_000)[(x, 39)].symbol().to_owned()).collect();
        assert!(row.contains("GO!"), "{action}: the text stays");
    }
}

#[test]
fn plugin_keys_bind_after_the_listing_and_run_the_action_with_the_focused_session() {
    let mut app = app();
    assert!(key(&mut app, "u", 1000).is_empty(), "unbound before the listing");
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({}))]))));
    let focused = app.store.state().focused_session_id.clone().expect("a focused session");
    let effects = key(&mut app, "u", 2000);
    let [Effect::RunPluginAction { plugin, action, body }] = effects.as_slice() else { panic!("{effects:?}") };
    assert_eq!((plugin.as_str(), action.as_str()), ("bt", "go"));
    assert_eq!(body["trigger"]["kind"], "key");
    assert_eq!(body["trigger"]["key"], "u");
    assert_eq!(body["focus"]["session"]["id"], focused.as_str());
    // A user binding from the listing moves it.
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({"bindings": {"go": "g"}}))]))));
    assert!(key(&mut app, "u", 3000).is_empty());
    assert!(matches!(key(&mut app, "g", 4000).as_slice(), [Effect::RunPluginAction { .. }]));
    // Removing the plugin unbinds its keys.
    app.apply(AppEvent::Plugins(listing(json!([]))));
    assert!(key(&mut app, "g", 5000).is_empty());
}

#[test]
fn an_action_that_needs_a_session_says_so_instead_of_running_without_one() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({}))]))));
    let go = crate::actions::ActionId::lookup("bt:go").unwrap();
    let mut ctx = ActionCtx::from_focus(app.store.state(), ActionSource::Key("u".into()));
    ctx.session = None;
    assert!(app.dispatch_action_at(go, &ctx, 1000).is_empty());
    assert_eq!(app.router.notices.current(1000), Some("Go: focus a session first"));
}

#[test]
fn a_confirm_action_arms_on_the_first_press_and_runs_on_the_second() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({}))]))));
    assert!(key(&mut app, "i", 1000).is_empty(), "first press only arms");
    assert_eq!(app.router.notices.current(1000), Some("Run Ask? click again or press y"));
    let effects = key(&mut app, "i", 1500);
    assert!(matches!(effects.as_slice(), [Effect::RunPluginAction { action, .. }] if action == "ask"), "{effects:?}");

    // A plugin asking for it never confirms its own arm.
    let ask = crate::actions::ActionId::lookup("bt:ask").unwrap();
    let ctx = ActionCtx::from_focus(app.store.state(), ActionSource::plugin("bt"));
    assert!(app.dispatch_action_at(ask, &ctx, 10_000).is_empty());
    assert!(app.dispatch_action_at(ask, &ctx, 10_500).is_empty(), "twice from a plugin still only arms");
}

#[test]
fn a_pane_action_asks_the_daemon_to_open_the_pane() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(json!([bt(json!({"panes": [{"id": "details", "title": "D", "placement": "overlay"}]}))]))));
    let look = crate::actions::ActionId::lookup("bt:look").unwrap();
    let ctx = ActionCtx::from_focus(app.store.state(), ActionSource::Key("l".into()));
    let effects = app.dispatch_action_at(look, &ctx, 1000);
    assert!(matches!(effects.as_slice(), [Effect::OpenPluginPane { plugin, pane, .. }] if plugin == "bt" && pane == "details"), "{effects:?}");
}

fn frame(v: serde_json::Value) -> UiActionFrame {
    serde_json::from_value(v).unwrap()
}

#[test]
fn the_ui_action_endpoint_ignores_a_confirmed_field_in_the_body() {
    let mut app = app();
    let target = "garage/a/two";
    // What the SSE task parses out of a frame whose JSON claims `confirmed`.
    let f = frame(json!({"plugin": "closer", "action": "session.close", "confirmed": true, "target": {"session": target, "confirmed": true}}));
    let (id, ctx) = crate::plugins::context::ui_action_ctx(app.store.state(), &f).unwrap();
    assert_eq!(id, ids::SESSION_CLOSE);
    assert_eq!(ctx.source, ActionSource::Plugin { plugin: "closer".into(), confirmed: false });
    assert_eq!((ctx.session.as_deref(), ctx.workspace.as_deref()), (Some(target), Some("a")));
    assert!(app.dispatch_action_at(id, &ctx, 1000).is_empty(), "it arms, it does not close");
    assert!(app.router.plugin_arm_pending());
    assert_eq!(app.router.notices.current(1000), Some("Close a/two? click again or press y"));
    // Asking again changes nothing: a plugin never confirms.
    assert!(app.dispatch_action_at(id, &ctx, 1500).is_empty());
}

#[test]
fn a_ui_action_on_a_core_action_takes_its_target_from_target_alone() {
    let app = app();
    let focused = app.store.state().focused_session_id.clone().unwrap();
    let f = frame(json!({"plugin": "p", "action": "session.close", "args": {"session": focused}}));
    let (_, ctx) = crate::plugins::context::ui_action_ctx(app.store.state(), &f).unwrap();
    assert_eq!((ctx.session, ctx.workspace), (None, None), "neither the focus nor args pick the session");
    assert!(ctx.args.is_none());
    // An id nobody declared is dropped before it gets anywhere.
    assert!(crate::plugins::context::ui_action_ctx(app.store.state(), &frame(json!({"plugin": "p", "action": "p:nope"}))).is_none());
}

#[test]
fn the_same_plugin_notice_is_not_stacked_and_a_new_one_replaces_it() {
    let mut app = app();
    app.apply(AppEvent::PluginNotice("bt: hello".into()));
    let first = now_ms();
    assert_eq!(app.router.notices.current(first), Some("bt: hello"));
    app.apply(AppEvent::PluginNotice("bt: hello".into()));
    assert_eq!(app.router.notices.current(first), Some("bt: hello"));
    app.apply(AppEvent::PluginNotice("bt: other".into()));
    assert_eq!(app.router.notices.current(now_ms()), Some("bt: other"));
}

#[tokio::test]
async fn an_opened_pane_takes_the_overlay_and_leaves_the_typing_layer_alone() {
    let mut app = app();
    let pane: garage_protocol::plugins::PaneInstance = serde_json::from_value(json!({
        "plugin": "bt", "pane": "details", "title": "Details", "placement": "overlay",
        "session": "plugin/bt/details", "attach": {"kind": "tmux"}
    })).unwrap();
    // While engaged (typing into a session) it is not shown.
    app.store.engage();
    app.apply(AppEvent::PaneOpened(pane.clone()));
    assert_eq!(app.store.state().overlay, None);
    assert!(!app.pane.is_open());
    assert!(app.router.notices.current(now_ms()).is_some_and(|n| n.contains("opened a pane")));
    app.store.disengage();

    app.apply(AppEvent::PaneOpened(pane));
    assert_eq!(app.store.state().overlay, Some(OverlayKind::PluginPane));
    assert_eq!(app.pane.session(), Some("plugin/bt/details"));
    let buf = drawn(&mut app, 100, 30, 1_000);
    let text: String = (0..30).map(|y| (0..100).map(|x| buf[(x, y)].symbol().to_owned()).collect::<String>()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("bt: Details") && text.contains("ctrl+q ctrl+q closes"), "{text}");
    // The pane's own session survives the registry's reconcile (it is not in the grid).
    app.registry.sync(&[]);
    assert!(app.registry.size_of("plugin/bt/details").is_some());
    // The daemon closing it takes the overlay down.
    app.apply(AppEvent::PaneClosed { session: "plugin/bt/details".into() });
    assert_eq!(app.store.state().overlay, None);
    app.registry.sync(&[]);
    assert!(app.registry.size_of("plugin/bt/details").is_none());
}

#[test]
fn pasting_into_the_pane_goes_to_the_pane_and_nowhere_else() {
    assert_eq!(paste_target(KeyLayer::Overlay, Some(OverlayKind::PluginPane)), PasteTarget::Pane);
    assert_eq!(paste_target(KeyLayer::Overlay, Some(OverlayKind::Help)), PasteTarget::Drop);
}

#[test]
fn slots_the_renderers_do_not_scope_get_nothing_from_plugins() {
    let app = app();
    let registry = &app.slots;
    for slot in [SlotId::Overlay, SlotId::ContextMenu] {
        assert!(registry.items(slot, &crate::slots::SlotCtx::default()).is_empty());
    }
}
