//! Whole-frame behaviour of `App::draw_frame` through a `TestBackend`:
//! the slot click regions it records and the animation it reports.
use super::*;
use crate::slots::{ClickTarget, SlotCtx, SlotId, SlotItem};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::text::Span;

fn drawn(app: &mut App, width: u16, height: u16, now: i64) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| app.draw_frame(f, now)).unwrap();
    terminal.backend().buffer().clone()
}

fn app_with_two_sessions() -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0");
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store
        .sessions_fetched(vec![si("a", "one"), si("a", "two")]);
    app
}

fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn a_wall_with_no_plugin_items_records_no_clicks_and_asks_for_no_animation() {
    let mut app = app_with_two_sessions();
    drawn(&mut app, 120, 40, 1_000);
    // The core strip may carry the pet; core tiles, rail and titles add none.
    assert!(
        app.slot_clicks.iter().all(|r| r.slot == SlotId::StripRight),
        "only the strip's own items: {:?}",
        app.slot_clicks
    );
    assert_eq!(app.slots.animation_interval(), None);
    assert!(app.gutter_sessions.is_empty());
}

#[test]
fn plugin_items_in_every_painted_slot_become_click_regions_where_they_were_drawn() {
    let mut app = app_with_two_sessions();
    let target = |a| ClickTarget::new(a).from_plugin("p");
    app.slots
        .register(SlotId::TileTitleRight, move |_: &SlotCtx| {
            vec![SlotItem {
                on_click: Some(target(ids::SESSION_MAXIMIZE)),
                ..SlotItem::new(vec![Span::raw(" [m]")], 1)
            }]
        });
    app.slots.register(SlotId::RailRow, move |_: &SlotCtx| {
        vec![SlotItem {
            on_click: Some(target(ids::SESSION_CLOSE)),
            ..SlotItem::new(vec![Span::raw(" [x]")], 1)
        }]
    });
    app.slots.register(SlotId::StripRight, move |_: &SlotCtx| {
        vec![SlotItem {
            on_click: Some(target(ids::OVERLAY_OPEN_HELP)),
            ..SlotItem::new(vec![Span::raw(" [?] ")], 1)
        }]
    });
    let buf = drawn(&mut app, 120, 40, 1_000);
    for region in &app.slot_clicks {
        let text: String = (region.rect.x..region.rect.right())
            .map(|x| buf[(x, region.rect.y)].symbol())
            .collect();
        let expected = match region.slot {
            SlotId::TileTitleRight => " [m]",
            SlotId::RailRow => " [x]",
            SlotId::StripRight => " [?] ",
            other => panic!("unexpected slot {other:?}"),
        };
        assert_eq!(text.trim(), expected.trim(), "{region:?}");
    }
    let by = |slot| app.slot_clicks.iter().filter(|r| r.slot == slot).count();
    assert_eq!(by(SlotId::TileTitleRight), 2, "one per tile");
    assert_eq!(by(SlotId::RailRow), 2, "one per session row");
    assert_eq!(by(SlotId::StripRight), 1);
    // A tile's region is scoped to its own session.
    let titles: Vec<_> = app
        .slot_clicks
        .iter()
        .filter(|r| r.slot == SlotId::TileTitleRight)
        .collect();
    assert_ne!(titles[0].session, titles[1].session);
}

#[test]
fn an_animated_strip_item_changes_frame_with_the_clock_and_reports_its_interval() {
    let mut app = app_with_two_sessions();
    app.slots.register(SlotId::StripRight, |_: &SlotCtx| {
        vec![
            SlotItem::animated(
                vec![vec![Span::raw(" <o> ")], vec![Span::raw(" <-> ")]],
                300,
                1,
            )
            .unwrap(),
        ]
    });
    let at0 = drawn(&mut app, 120, 40, 0);
    assert!(row_text(&at0, 39).contains("<o>"), "{}", row_text(&at0, 39));
    assert_eq!(
        app.slots.animation_interval(),
        Some(Duration::from_millis(300))
    );
    let at300 = drawn(&mut app, 120, 40, 300);
    assert!(
        row_text(&at300, 39).contains("<->"),
        "{}",
        row_text(&at300, 39)
    );
    assert!(!row_text(&at300, 39).contains("<o>"));
}

#[test]
fn removing_the_animated_item_stops_the_animation_on_the_next_frame() {
    let mut app = app_with_two_sessions();
    app.slots.register(SlotId::TileTitleRight, |_: &SlotCtx| {
        vec![SlotItem::animated(vec![vec![Span::raw("a")], vec![Span::raw("b")]], 200, 1).unwrap()]
    });
    drawn(&mut app, 120, 40, 0);
    assert_eq!(
        app.slots.animation_interval(),
        Some(Duration::from_millis(200))
    );
    // A fresh registry (the plugin went away) draws no animation.
    app.slots = crate::slots::SlotRegistry::core();
    drawn(&mut app, 120, 40, 100);
    assert_eq!(app.slots.animation_interval(), None);
}

#[test]
fn a_provider_sees_the_geometry_of_the_frame_that_was_drawn() {
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(None));
    let mut app = app_with_two_sessions();
    let sink = seen.clone();
    app.slots
        .register(SlotId::StripRight, move |ctx: &SlotCtx| {
            *sink.lock().unwrap() = ctx.geometry.cloned();
            Vec::new()
        });
    // Frame 1: nothing is known yet (the default geometry).
    drawn(&mut app, 120, 40, 0);
    assert_eq!(seen.lock().unwrap().as_ref().unwrap().strip_width, 0);
    // Frame 2 sees frame 1's real geometry ...
    drawn(&mut app, 120, 40, 33);
    let geometry = seen.lock().unwrap().clone().unwrap();
    assert_eq!(geometry.strip_width, 120);
    let filler = geometry
        .strip_filler
        .clone()
        .expect("a 120-column strip has a filler");
    assert!(
        filler.start == 0 && filler.end <= 120 && filler.start < filler.end,
        "{filler:?}"
    );
    assert_eq!(geometry.rail_width, 27);
    assert_eq!(
        app.slots.geometry(),
        geometry,
        "and the registry keeps it for the plugin snapshot"
    );
    // ... and a resize is picked up on the next draw.
    drawn(&mut app, 60, 30, 66);
    drawn(&mut app, 60, 30, 99);
    assert_eq!(seen.lock().unwrap().as_ref().unwrap().strip_width, 60);
}
