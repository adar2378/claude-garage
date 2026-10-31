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

fn draw(app: &mut App) {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| app.draw_frame(f, 1_000)).unwrap();
}

#[test]
fn a_divider_drag_previews_live_and_resizes_the_ptys_only_on_release() {
    let mut app = app_with_layout();
    let ids = app.store.state().gridded_session_ids.clone();
    app.registry.sync(&ids);
    draw(&mut app);
    let before = app.registry.size_of(&ids[0]).unwrap();
    let divider = RailPrefs::DEFAULT_WIDTH - 1;

    mouse(&mut app, MouseEventKind::Down(LEFT), divider, 5);
    mouse(&mut app, MouseEventKind::Drag(LEFT), 39, 5);
    draw(&mut app);
    assert_eq!(
        app.layout.as_ref().unwrap().rail.width,
        40,
        "divider is live"
    );
    assert_eq!(
        app.registry.size_of(&ids[0]),
        Some(before),
        "PTYs untouched"
    );
    assert_eq!(app.store.state().rail.width, RailPrefs::DEFAULT_WIDTH);

    mouse(&mut app, MouseEventKind::Drag(LEFT), 45, 5);
    draw(&mut app);
    assert_eq!(
        app.registry.size_of(&ids[0]),
        Some(before),
        "still untouched"
    );

    mouse(&mut app, MouseEventKind::Up(LEFT), 45, 5);
    draw(&mut app);
    assert_eq!(app.store.state().rail.width, 46);
    let after = app.registry.size_of(&ids[0]).unwrap();
    assert!(
        after.0 < before.0,
        "tiles shrank once: {before:?} -> {after:?}"
    );
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
    assert!(app.selection.is_none(), "the highlight goes once copied");
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
    mouse(
        &mut plain_click,
        MouseEventKind::Down(LEFT),
        rect.x + 2,
        rect.y + 2,
    );
    mouse(
        &mut plain_click,
        MouseEventKind::Up(LEFT),
        rect.x + 2,
        rect.y + 2,
    );
    assert!(plain_click.copies.is_empty());
    assert!(plain_click.selection.is_none());
}

#[test]
fn ctrl_c_always_reaches_the_session_even_with_a_highlight() {
    let mut app = app_with_layout();
    let (id, rect) = first_tile_with_text(&mut app);
    let (x, y) = (rect.x + 1, rect.y + 1);
    // Mid-drag: the highlight is up and the button is still down.
    mouse(&mut app, MouseEventKind::Down(LEFT), x, y);
    mouse(&mut app, MouseEventKind::Drag(LEFT), x + 4, y);
    assert!(app.selection.is_some());
    app.store.engage();
    assert!(app.registry.frozen(&id).is_some());
    let quit = app.handle_key(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(!quit);
    // A key that reaches the PTY snaps a frozen tile live first: that only
    // happens when Ctrl+C was routed to the session, not swallowed.
    assert!(
        app.registry.frozen(&id).is_none(),
        "Ctrl+C reached the session"
    );
    assert!(app.copies.is_empty(), "a key never copies; release does");
    assert!(app.selection.is_none());
    assert_eq!(app.store.state().layer, KeyLayer::Engaged);
}

#[test]
fn any_key_or_the_wheel_drops_a_highlight_without_copying() {
    let mut app = app_with_layout();
    let (_, rect) = first_tile_with_text(&mut app);
    let (x, y) = (rect.x + 1, rect.y + 1);
    for clear in [0, 1] {
        mouse(&mut app, MouseEventKind::Down(LEFT), x, y);
        mouse(&mut app, MouseEventKind::Drag(LEFT), x + 4, y);
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

// ── draft recovery ────────────────────────────────────────────────────

mod draft_flow {
    use super::*;
    use crate::drafts::{DraftStore, Prompt};
    use crate::runtime::drafts::Piece;

    fn app_with_drafts() -> (App, tempfile::TempDir, DraftStore, String) {
        let mut app = app_with_layout();
        let dir = tempfile::tempdir().unwrap();
        app.drafts.enable(dir.path().join("drafts"));
        let store = DraftStore::new(dir.path().join("drafts"));
        let id = app.store.state().gridded_session_ids[0].clone();
        (app, dir, store, id)
    }

    #[test]
    fn typed_text_is_kept_and_a_sent_prompt_forgets_it() {
        let (mut app, _d, store, id) = app_with_drafts();
        app.observe_prompt(&id, Prompt::Text("half a thought".into()), 1000);
        assert_eq!(store.load_text(&id).as_deref(), Some("half a thought"));
        app.observe_prompt(&id, Prompt::Unknown, 2000);
        assert!(store.has_anything(&id), "a dialog proves nothing");
        app.observe_prompt(&id, Prompt::Empty, 3000);
        assert!(!store.has_anything(&id), "sent: nothing left to recover");
    }

    #[test]
    fn a_power_cut_draft_is_offered_once_on_the_empty_prompt_and_typed_back_on_u() {
        let (mut app, _d, store, id) = app_with_drafts();
        store.save_text(&id, "the plan [Image #1] is").unwrap();
        let png = _d.path().join("a.png");
        std::fs::write(&png, b"png").unwrap();
        let kept = store.add_image(&id, &png).unwrap().unwrap();

        // The restored session shows an empty prompt: offer, keep the files.
        assert!(app.observe_prompt(&id, Prompt::Empty, 10_000));
        assert!(store.has_anything(&id), "an open offer protects the draft");
        assert!(
            app.router
                .notices
                .current(10_000)
                .is_some_and(|n| n.contains("press u"))
        );
        assert!(!app.observe_prompt(&id, Prompt::Empty, 11_000), "once only");
        assert!(store.has_anything(&id));
        app.drafts.shown.insert(id.clone(), Prompt::Empty);

        // Another key is not an answer; u is.
        assert!(!app.take_draft_offer("x", 12_000));
        assert!(app.take_draft_offer("u", 12_000));
        let r = &app.drafts.restoring[0];
        assert_eq!(r.session, id);
        let pieces: Vec<&Piece> = r.pieces.iter().collect();
        assert_eq!(pieces[0], &Piece::Text("the plan  is".into()));
        assert_eq!(
            pieces[1],
            &Piece::Image(kept.to_string_lossy().into_owned())
        );

        // Pieces go out one per gap, not all at once.
        assert!(app.draft_tick(12_000));
        assert_eq!(app.drafts.restoring[0].pieces.len(), 1);
        assert!(!app.draft_tick(12_300), "next piece is not due yet");
        assert!(app.draft_tick(12_800));
        assert!(app.drafts.restoring.is_empty());
        assert!(!app.take_draft_offer("u", 13_000), "the offer is spent");
    }

    #[test]
    fn typing_instead_of_restoring_withdraws_the_offer() {
        let (mut app, _d, store, id) = app_with_drafts();
        store.save_text(&id, "old").unwrap();
        app.observe_prompt(&id, Prompt::Empty, 1000);
        app.observe_prompt(&id, Prompt::Text("new words".into()), 2000);
        assert!(!app.take_draft_offer("u", 3000));
        assert_eq!(store.load_text(&id).as_deref(), Some("new words"));
    }

    /// Tick until the background copies are folded in (bounded wait).
    fn wait_for_copies(app: &mut App, now: i64) {
        for _ in 0..200 {
            app.draft_tick(now);
            if app.drafts.inflight.is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("image copy never finished");
    }

    #[test]
    fn a_pasted_image_is_copied_off_the_ui_thread_and_survives_the_moment_before_claude_draws_it() {
        let (mut app, d, store, id) = app_with_drafts();
        let png = d.path().join("shot.png");
        std::fs::write(&png, b"png").unwrap();
        app.note_paste_for_drafts(&id, png.to_str().unwrap(), 1000);
        // The call returned with the copy only started: it is in flight, and
        // an empty prompt meanwhile is not read as "sent" or as a leftover.
        assert!(app.drafts.inflight.contains_key(&id));
        assert!(!app.observe_prompt(&id, Prompt::Empty, 1500));
        wait_for_copies(&mut app, 1600);
        assert_eq!(store.images(&id).len(), 1);
        // Prompt still empty a beat later: not judged yet.
        app.observe_prompt(&id, Prompt::Empty, 2000);
        assert_eq!(store.images(&id).len(), 1);
        // Long after, still empty: it was sent.
        app.observe_prompt(&id, Prompt::Empty, 20_000);
        assert!(store.images(&id).is_empty());
        // And an image pasted this run never raises an offer.
        assert!(app.drafts.offers.is_empty());
    }

    #[test]
    fn a_paste_that_cannot_name_an_image_starts_no_copy() {
        let (mut app, _d, _store, id) = app_with_drafts();
        app.note_paste_for_drafts(&id, "just some words", 1000);
        app.note_paste_for_drafts(&id, &"x.png ".repeat(5000), 1000);
        assert!(app.drafts.inflight.is_empty());
    }

    fn offered(app: &mut App, store: &DraftStore, id: &str) {
        store.save_text(id, "my draft").unwrap();
        app.observe_prompt(id, Prompt::Empty, 1000);
        app.drafts.shown.insert(id.to_owned(), Prompt::Empty);
    }

    #[test]
    fn u_restores_only_into_the_focused_session_and_an_offer_expires_after_a_minute() {
        let (mut app, _d, store, id) = app_with_drafts();
        let other = app.store.state().gridded_session_ids[1].clone();
        assert_eq!(app.store.state().focused_session_id.as_deref(), Some(&*id));
        // The offer belongs to the session that is not focused.
        offered(&mut app, &store, &other);
        assert!(!app.take_draft_offer("u", 2000), "no fallback to offers[0]");
        assert!(app.drafts.restoring.is_empty());
        assert_eq!(app.drafts.offers.len(), 1, "the offer is still waiting");

        // Focused, it can be taken - but only for a minute.
        app.store.focus_session(&other);
        assert_eq!(app.drafts.offers[0].1, 1000 + 60_000, "expires after 60 s");
        assert!(!app.take_draft_offer("u", 1000 + 60_000), "expired");
        assert!(app.drafts.restoring.is_empty());
        assert!(app.take_draft_offer("u", 1000 + 59_000));
        assert_eq!(app.drafts.restoring[0].session, other);
    }

    #[test]
    fn a_restore_stops_when_the_prompt_is_no_longer_empty() {
        let (mut app, _d, store, id) = app_with_drafts();
        store.save_text(&id, "typed back").unwrap();
        let png = _d.path().join("a.png");
        std::fs::write(&png, b"png").unwrap();
        store.add_image(&id, &png).unwrap();
        store.add_image(&id, &png).unwrap();
        app.observe_prompt(&id, Prompt::Empty, 1000);
        app.drafts.shown.insert(id.clone(), Prompt::Empty);
        assert!(app.take_draft_offer("u", 2000));
        assert_eq!(app.drafts.restoring[0].pieces.len(), 3);

        // Text piece goes out; the prompt now shows exactly that (with
        // Claude's own placeholder after an image): carry on.
        app.draft_tick(2000);
        assert_eq!(app.drafts.restoring[0].pieces.len(), 2);
        app.drafts
            .shown
            .insert(id.clone(), Prompt::Text("typed back".into()));
        app.draft_tick(2800);
        assert_eq!(app.drafts.restoring[0].pieces.len(), 1);
        app.drafts
            .shown
            .insert(id.clone(), Prompt::Text("typed back [Image #1]".into()));
        // ...but the user starts typing something else: the rest is dropped.
        app.drafts.shown.insert(
            id.clone(),
            Prompt::Text("typed back [Image #1] and me".into()),
        );
        assert!(app.draft_tick(3600));
        assert!(
            app.drafts.restoring.is_empty(),
            "aborted, nothing more typed"
        );
        assert!(
            app.router
                .notices
                .current(3600)
                .is_some_and(|n| n.contains("stopped"))
        );
    }

    #[test]
    fn a_restore_never_types_into_a_dialog_or_a_non_empty_box() {
        for shown in [Prompt::Unknown, Prompt::Text("something else".into())] {
            let (mut app, _d, store, id) = app_with_drafts();
            offered(&mut app, &store, &id);
            assert!(app.take_draft_offer("u", 2000));
            app.drafts.shown.insert(id.clone(), shown);
            assert!(app.draft_tick(2000), "the stop is announced");
            assert!(app.drafts.restoring.is_empty());
        }
    }

    #[test]
    fn a_user_close_and_a_fresh_spawn_wipe_a_sessions_draft_and_others_stay() {
        for event in [
            AppEvent::SessionClosed("garage/a/one".into()),
            AppEvent::SessionSpawned("garage/a/one".into()),
        ] {
            let (mut app, _d, store, _id) = app_with_drafts();
            let id = "garage/a/one";
            store.save_text(id, "old words").unwrap();
            app.apply(event);
            assert!(!store.has_anything(id), "a reused label starts clean");
            store.save_text("garage/a/two", "keep").unwrap();
            app.apply(AppEvent::SessionClosed(id.into()));
            assert!(store.has_anything("garage/a/two"));
        }
    }

    #[test]
    fn an_image_copy_finishing_after_the_user_closed_the_session_is_discarded() {
        let (mut app, d, store, id) = app_with_drafts();
        let png = d.path().join("shot.png");
        std::fs::write(&png, b"png").unwrap();
        app.user_closed(&id);
        app.note_paste_for_drafts(&id, png.to_str().unwrap(), 1000);
        wait_for_copies(&mut app, 1600);
        assert!(store.images(&id).is_empty());
        // A fresh session with the same id keeps images again.
        app.fresh_session(&id);
        app.note_paste_for_drafts(&id, png.to_str().unwrap(), 2000);
        wait_for_copies(&mut app, 2600);
        assert_eq!(store.images(&id).len(), 1);
    }

    #[test]
    fn old_drafts_are_pruned_hourly_while_running_not_only_at_startup() {
        let (mut app, _d, store, id) = app_with_drafts();
        store.save_text(&id, "stale").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        // First tick prunes with the real 7-day age: a fresh file survives.
        app.draft_tick(10_000_000);
        assert!(store.has_anything(&id));
        // Not again within the hour, whatever the age.
        let hour = 3_600_000;
        assert_eq!(app.drafts.last_prune_ms, 10_000_000);
        app.draft_tick(10_000_000 + hour - 1_000);
        assert_eq!(app.drafts.last_prune_ms, 10_000_000);
        app.draft_tick(10_000_000 + hour);
        assert_eq!(app.drafts.last_prune_ms, 10_000_000 + hour, "hourly");
    }

    #[test]
    fn switching_drafts_off_removes_what_an_earlier_run_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("drafts");
        DraftStore::new(root.clone()).save_text("s", "old").unwrap();
        assert!(root.exists());
        crate::runtime::drafts::DraftSync::disabled(&root);
        assert!(!root.exists());
        crate::runtime::drafts::DraftSync::disabled(&root); // nothing there: fine
    }

    #[test]
    fn closing_a_session_deletes_what_was_kept_and_nothing_is_written_when_off() {
        let (mut app, _d, store, id) = app_with_drafts();
        app.observe_prompt(&id, Prompt::Text("x".into()), 1000);
        app.user_closed(&id);
        assert!(!store.has_anything(&id));

        let mut off = app_with_layout(); // never enabled
        let id = off.store.state().gridded_session_ids[0].clone();
        assert!(!off.observe_prompt(&id, Prompt::Text("x".into()), 1000));
        assert!(!off.take_draft_offer("u", 1000));
    }
}
