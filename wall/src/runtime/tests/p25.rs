//! p25 core items: sidebar hide/resize and mouse selection with copy, driven
//! through the real key and mouse entry points.
use super::*;
use crate::input::selection::Selection;
use crate::state::wall_state::RailPrefs;
use crate::ui::layout::wall_layout_with_rail;
use crate::ui::registry::FrozenTile;
use crate::ui::scroll::ScrollModel;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

fn app_with_layout() -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0", 0);
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store
        .sessions_fetched(vec![si("a", "one"), si("a", "two")]);
    relayout(&mut app);
    app
}

fn relayout(app: &mut App) {
    let n = app.store.state().gridded_session_ids.len();
    app.layout = Some(wall_layout_with_rail(
        Rect::new(0, 0, 120, 40),
        n,
        None,
        false,
        false,
        app.store.state().rail.layout_width(),
    ));
}

fn mouse(app: &mut App, kind: MouseEventKind, col: u16, row: u16) {
    app.handle_mouse(&MouseEvent {
        kind,
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

const LEFT: MouseButton = MouseButton::Left;

/// Give the first tile a frozen screen with known text (no tmux needed).
fn first_tile_with_text(app: &mut App) -> (String, Rect) {
    let ids = app.store.state().gridded_session_ids.clone();
    app.registry.sync(&ids);
    let mut parser = vt100::Parser::new(20, 40, 0);
    parser.process(b"hello world\r\nsecond line");
    app.registry.set_frozen(
        &ids[0],
        Some(FrozenTile {
            model: ScrollModel::freeze(50, 5).unwrap(),
            parser,
            new_lines: 0,
            counted_at: std::time::Instant::now(),
        }),
    );
    let rect = app.layout.as_ref().unwrap().tiles[0];
    (ids[0].clone(), rect)
}

// ── sidebar ───────────────────────────────────────────────────────────

#[test]
fn b_hides_the_rail_and_the_grid_takes_its_columns() {
    let mut app = app_with_layout();
    let before = app.layout.as_ref().unwrap().grid.width;
    app.handle_key(&plain(KeyCode::Char('b')));
    assert!(app.store.state().rail.hidden);
    relayout(&mut app);
    let l = app.layout.as_ref().unwrap();
    assert_eq!(l.rail.width, 0);
    assert_eq!(l.grid.width, before + RailPrefs::DEFAULT_WIDTH);
    app.handle_key(&plain(KeyCode::Char('b')));
    assert!(!app.store.state().rail.hidden);
}

#[test]
fn angle_keys_resize_within_bounds_and_bring_a_hidden_rail_back() {
    let mut app = app_with_layout();
    app.handle_key(&plain(KeyCode::Char('>')));
    assert_eq!(app.store.state().rail.width, 30);
    app.handle_key(&plain(KeyCode::Char('<')));
    app.handle_key(&plain(KeyCode::Char('<')));
    assert_eq!(app.store.state().rail.width, 26);
    for _ in 0..40 {
        app.handle_key(&plain(KeyCode::Char('<')));
    }
    assert_eq!(app.store.state().rail.width, RailPrefs::MIN);
    for _ in 0..40 {
        app.handle_key(&plain(KeyCode::Char('>')));
    }
    assert_eq!(app.store.state().rail.width, RailPrefs::MAX);
    app.handle_key(&plain(KeyCode::Char('b')));
    app.handle_key(&plain(KeyCode::Char('<')));
    assert!(!app.store.state().rail.hidden, "resizing shows the rail");
}

#[test]
fn engaged_typing_never_triggers_rail_keys() {
    let mut app = app_with_layout();
    app.store.engage();
    app.handle_key(&plain(KeyCode::Char('b')));
    app.handle_key(&plain(KeyCode::Char('>')));
    assert_eq!(app.store.state().rail, RailPrefs::default());
}

#[test]
fn dragging_the_divider_resizes_and_a_no_op_drag_saves_nothing() {
    let mut app = app_with_layout();
    let rev = app.store.views_revision();
    let divider = RailPrefs::DEFAULT_WIDTH - 1;
    mouse(&mut app, MouseEventKind::Down(LEFT), divider, 5);
    assert_eq!(app.drag, Some(super::mouse::Drag::Rail));
    mouse(&mut app, MouseEventKind::Drag(LEFT), 39, 5);
    mouse(&mut app, MouseEventKind::Up(LEFT), 39, 5);
    assert_eq!(app.store.state().rail.width, 40);
    assert!(app.drag.is_none());
    assert!(app.store.views_revision() > rev, "a resize is saved");

    // Dragged far left: clamped to the minimum, never to nothing.
    relayout(&mut app);
    mouse(&mut app, MouseEventKind::Down(LEFT), 39, 5);
    mouse(&mut app, MouseEventKind::Drag(LEFT), 2, 5);
    mouse(&mut app, MouseEventKind::Up(LEFT), 2, 5);
    assert_eq!(app.store.state().rail.width, RailPrefs::MIN);

    // Grab and release in place: same width, no new save.
    relayout(&mut app);
    let rev = app.store.views_revision();
    mouse(&mut app, MouseEventKind::Down(LEFT), RailPrefs::MIN - 1, 5);
    mouse(&mut app, MouseEventKind::Drag(LEFT), RailPrefs::MIN - 1, 5);
    mouse(&mut app, MouseEventKind::Up(LEFT), RailPrefs::MIN - 1, 5);
    assert_eq!(app.store.views_revision(), rev);
}

#[test]
fn a_hidden_rail_has_no_divider_to_grab() {
    let mut app = app_with_layout();
    app.handle_key(&plain(KeyCode::Char('b')));
    relayout(&mut app);
    mouse(&mut app, MouseEventKind::Down(LEFT), 27, 5);
    assert_ne!(app.drag, Some(super::mouse::Drag::Rail));
}

// ── selection and copy ────────────────────────────────────────────────

#[test]
fn dragging_over_tile_text_selects_it_and_copies_on_release() {
    let mut app = app_with_layout();
    let (id, rect) = first_tile_with_text(&mut app);
    let (x, y) = (rect.x + 1, rect.y + 1);
    mouse(&mut app, MouseEventKind::Down(LEFT), x, y);
    assert!(app.selection.is_none(), "a bare press selects nothing");
    mouse(&mut app, MouseEventKind::Drag(LEFT), x + 4, y);
    assert_eq!(
        app.selection,
        Some(Selection {
            session: id,
            anchor: (0, 0),
            head: (4, 0)
        })
    );
    mouse(&mut app, MouseEventKind::Up(LEFT), x + 4, y);
    assert_eq!(app.copies, vec!["hello".to_owned()]);
    assert!(app.selection.is_some(), "the highlight stays for Ctrl+C");
}

#[test]
fn a_drag_across_rows_copies_both_lines_and_a_click_copies_nothing() {
    let mut app = app_with_layout();
    let (_, rect) = first_tile_with_text(&mut app);
    let (x, y) = (rect.x + 1, rect.y + 1);
    mouse(&mut app, MouseEventKind::Down(LEFT), x + 6, y);
    mouse(&mut app, MouseEventKind::Drag(LEFT), x + 5, y + 1);
    mouse(&mut app, MouseEventKind::Up(LEFT), x + 5, y + 1);
    assert_eq!(app.copies, vec!["world\nsecond".to_owned()]);

    let mut plain_click = app_with_layout();
    let (_, rect) = first_tile_with_text(&mut plain_click);
    mouse(&mut plain_click, MouseEventKind::Down(LEFT), rect.x + 2, rect.y + 2);
    mouse(&mut plain_click, MouseEventKind::Up(LEFT), rect.x + 2, rect.y + 2);
    assert!(plain_click.copies.is_empty());
    assert!(plain_click.selection.is_none());
}

#[test]
fn ctrl_c_with_a_highlight_copies_and_is_consumed_even_while_engaged() {
    let mut app = app_with_layout();
    let (_, rect) = first_tile_with_text(&mut app);
    let (x, y) = (rect.x + 1, rect.y + 1);
    mouse(&mut app, MouseEventKind::Down(LEFT), x, y);
    mouse(&mut app, MouseEventKind::Drag(LEFT), x + 4, y);
    mouse(&mut app, MouseEventKind::Up(LEFT), x + 4, y);
    app.store.engage();
    app.copies.clear();
    let quit = app.handle_key(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(!quit);
    assert_eq!(app.copies, vec!["hello".to_owned()]);
    assert!(app.selection.is_none(), "copying clears the highlight");
    assert_eq!(
        app.store.state().layer,
        KeyLayer::Engaged,
        "the key was swallowed, not routed anywhere"
    );
}

#[test]
fn any_other_key_or_the_wheel_drops_the_highlight_without_copying_again() {
    let mut app = app_with_layout();
    let (_, rect) = first_tile_with_text(&mut app);
    let (x, y) = (rect.x + 1, rect.y + 1);
    for clear in [0, 1] {
        mouse(&mut app, MouseEventKind::Down(LEFT), x, y);
        mouse(&mut app, MouseEventKind::Drag(LEFT), x + 4, y);
        mouse(&mut app, MouseEventKind::Up(LEFT), x + 4, y);
        app.copies.clear();
        assert!(app.selection.is_some());
        if clear == 0 {
            app.handle_key(&plain(KeyCode::Char('j')));
        } else {
            mouse(&mut app, MouseEventKind::ScrollUp, x, y);
        }
        assert!(app.selection.is_none());
        assert!(app.copies.is_empty());
    }
}

#[test]
fn the_highlight_paints_reverse_video_only_on_the_selected_cells() {
    let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 20, 4));
    let inner = Rect::new(2, 1, 10, 2);
    let sel = Selection {
        session: "s".into(),
        anchor: (8, 0),
        head: (1, 1),
    };
    crate::input::selection::paint(&mut buf, inner, &sel);
    let rev = |c: u16, r: u16| {
        buf[(c, r)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    };
    assert!(rev(2 + 8, 1) && rev(2 + 9, 1), "first row from the anchor");
    assert!(!rev(2 + 7, 1));
    assert!(rev(2, 2) && rev(3, 2), "last row up to the head");
    assert!(!rev(4, 2));
}
