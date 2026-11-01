//! The `p` plugins overlay as the App sees it: the key that opens and closes
//! it, selection, the enable/disable request Space raises, the refresh from
//! the daemon's `plugins` event and the inline error. No daemon: events are
//! applied directly, as the SSE task would send them.
use super::*;
use crate::runtime::overlays::{handle_plugins_key, plugin_toggle_settled};
use crate::ui::plugins_overlay::PluginsOverlayState;
use garage_protocol::plugins::PluginList;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;

fn listing(v: serde_json::Value) -> PluginList {
    serde_json::from_value(json!({ "plugins": v })).unwrap()
}

fn two() -> serde_json::Value {
    json!([
        {"id": "usage-bar", "name": "Usage bar", "enabled": true, "active": true,
         "source": "bundled", "permissions": ["widgets"]},
        {"id": "pet-extras", "name": "Pet extras", "enabled": false, "active": false,
         "source": "local", "permissions": ["ui.invoke"]},
    ])
}

fn app() -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0");
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store.sessions_fetched(vec![si("a", "one")]);
    app
}

fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| app.draw_frame(f, 5_000)).unwrap();
    let buf = terminal.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `p` through the production garage path, then the overlay's own keys.
fn press_p(app: &mut App) {
    let outcome = app
        .router
        .on_garage_key(&mut app.store, &app.keymap, "p", 1000);
    let (id, ctx) = outcome.action.expect("p is bound");
    app.dispatch_action_at(id, &ctx, 1000);
}

fn overlay_key(
    store: &mut WallStore,
    state: &mut PluginsOverlayState,
    list: &PluginList,
    code: KeyCode,
) -> Vec<Effect> {
    handle_plugins_key(store, &plain(code), state, list)
}

#[test]
fn p_opens_the_overlay_and_p_or_esc_closes_it() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(two())));
    press_p(&mut app);
    assert_eq!(app.store.state().overlay, Some(OverlayKind::Plugins));
    assert!(screen(&mut app).contains("usage-bar"));

    app.handle_key(&plain(KeyCode::Char('p')));
    assert_eq!(app.store.state().overlay, None);

    press_p(&mut app);
    app.handle_key(&plain(KeyCode::Esc));
    assert_eq!(app.store.state().overlay, None);
}

#[test]
fn j_k_and_arrows_move_the_selection_and_wrap() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::Plugins);
    let list = listing(two());
    let mut state = PluginsOverlayState::default();
    for (code, want) in [
        (KeyCode::Char('j'), 1),
        (KeyCode::Char('j'), 0),
        (KeyCode::Char('k'), 1),
        (KeyCode::Up, 0),
        (KeyCode::Down, 1),
    ] {
        assert_eq!(overlay_key(&mut store, &mut state, &list, code), vec![]);
        assert_eq!(state.selected, want, "{code:?}");
    }
    assert_eq!(store.state().overlay, Some(OverlayKind::Plugins));
}

#[test]
fn space_asks_to_disable_an_enabled_plugin_and_enable_a_disabled_one() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::Plugins);
    let list = listing(two());
    let mut state = PluginsOverlayState::default();
    assert_eq!(
        overlay_key(&mut store, &mut state, &list, KeyCode::Char(' ')),
        vec![Effect::TogglePlugin {
            id: "usage-bar".into(),
            enable: false
        }]
    );
    // One request at a time: Space is ignored until the call settles.
    assert_eq!(
        overlay_key(&mut store, &mut state, &list, KeyCode::Char(' ')),
        vec![]
    );
    assert_eq!(
        plugin_toggle_settled(&mut state, "usage-bar", None, true),
        None
    );
    overlay_key(&mut store, &mut state, &list, KeyCode::Char('j'));
    assert_eq!(
        overlay_key(&mut store, &mut state, &list, KeyCode::Char(' ')),
        vec![Effect::TogglePlugin {
            id: "pet-extras".into(),
            enable: true
        }]
    );
}

#[test]
fn space_on_an_empty_list_does_nothing() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::Plugins);
    let mut state = PluginsOverlayState::default();
    let empty = listing(json!([]));
    assert_eq!(
        overlay_key(&mut store, &mut state, &empty, KeyCode::Char(' ')),
        vec![]
    );
    assert_eq!(state.busy, None);
}

#[test]
fn the_footer_names_the_remove_command_except_for_a_bundled_plugin() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(two())));
    press_p(&mut app);
    let first = screen(&mut app);
    assert!(first.contains("bundled: on/off only"));
    assert!(!first.contains("plugin remove"));

    app.handle_key(&plain(KeyCode::Char('j')));
    let second = screen(&mut app);
    assert!(second.contains("remove: claude-garage plugin remove pet-extras"));
    assert!(!second.contains("bundled: on/off only"));
}

#[test]
fn the_plugins_event_refreshes_the_open_list_and_clamps_the_selection() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(two())));
    press_p(&mut app);
    app.handle_key(&plain(KeyCode::Char('j')));
    let before = screen(&mut app);
    assert!(before.contains("pet-extras") && before.contains(" off "));

    // The daemon reloaded after a toggle: pet-extras is on, then gone.
    let mut on = two();
    on[1]["enabled"] = json!(true);
    on[1]["active"] = json!(true);
    app.apply(AppEvent::Plugins(listing(on)));
    assert!(
        !screen(&mut app).contains(" off "),
        "enabled state refreshed"
    );

    app.apply(AppEvent::Plugins(listing(json!([two()[0].clone()]))));
    let after = screen(&mut app);
    assert!(!after.contains("pet-extras"));
    assert_eq!(app.plugins_overlay.selected, 0, "selection clamped");
    assert!(after.contains("bundled: on/off only"), "footer follows");
}

#[test]
fn a_failed_toggle_shows_in_the_footer_and_clears_on_the_next_move() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(two())));
    press_p(&mut app);
    app.plugins_overlay.busy = Some("usage-bar".into());
    app.apply(AppEvent::PluginToggleSettled {
        id: "usage-bar".into(),
        error: Some("couldn't turn usage-bar off (daemon error 500)".into()),
    });
    assert_eq!(app.plugins_overlay.busy, None);
    assert!(screen(&mut app).contains("couldn't turn usage-bar off (daemon error 500)"));

    app.handle_key(&plain(KeyCode::Char('j')));
    assert!(!screen(&mut app).contains("failed to load"));
}

#[test]
fn a_failure_after_the_overlay_closed_becomes_a_strip_notice() {
    let mut state = PluginsOverlayState {
        busy: Some("x".into()),
        ..Default::default()
    };
    assert_eq!(
        plugin_toggle_settled(
            &mut state,
            "x",
            Some("couldn't turn x on (daemon error 500)".into()),
            false
        )
        .as_deref(),
        Some("couldn't turn x on (daemon error 500)")
    );
    assert_eq!(state.busy, None);
}

#[test]
fn toggle_errors_name_the_plugin_the_direction_and_the_status() {
    use crate::api::client::ApiError;
    use crate::runtime::effects::plugin_toggle_error;
    let status = |status, message: &str| ApiError::Status {
        status,
        message: message.into(),
    };
    assert_eq!(
        plugin_toggle_error("pet-extras", false, &status(500, "")),
        "couldn't turn pet-extras off (daemon error 500)"
    );
    assert_eq!(
        plugin_toggle_error("pet-extras", true, &status(409, "is bundled")),
        "couldn't turn pet-extras on (daemon error 409: is bundled)"
    );
    assert_eq!(
        plugin_toggle_error("x", true, &ApiError::Transport("down".into())),
        "couldn't turn x on (daemon not reachable)"
    );
}

#[test]
fn an_empty_listing_shows_the_install_hint() {
    let mut app = app();
    app.apply(AppEvent::Plugins(listing(json!([]))));
    press_p(&mut app);
    let text = screen(&mut app);
    assert!(text.contains("No plugins installed"));
    assert!(text.contains("claude-garage plugin install owner/repo"));
}
