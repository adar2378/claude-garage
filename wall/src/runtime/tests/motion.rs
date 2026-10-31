//! p23 motion at the loop level: no timed redraw when nothing animates,
//! the spinner clock only while a spinner shows, and motion off leaving the
//! frame exactly as before. The tiles here have no PTY client (tests never
//! touch tmux), which is exactly the "attaching" state.
use super::*;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::ui::motion::{FX_TICK, SPINNER_TICK};

fn app_with(motion: bool, sessions: Vec<SessionInfo>) -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::with_motion(tx, "http://127.0.0.1:0", 0, motion);
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store.sessions_fetched(sessions);
    app
}

fn draw(app: &mut App, now: i64) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|f| app.draw_frame(f, now)).unwrap();
    let buf = terminal.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn loop_timeout_is_unchanged_when_nothing_animates() {
    assert_eq!(loop_timeout(false, Duration::from_millis(5), None), IDLE_POLL);
    assert_eq!(
        loop_timeout(true, Duration::from_millis(5), None),
        FRAME_CAP - Duration::from_millis(5)
    );
}

#[test]
fn loop_timeout_shortens_to_the_motion_tick_only_while_animating() {
    assert_eq!(loop_timeout(false, Duration::ZERO, Some(FX_TICK)), FX_TICK);
    assert_eq!(
        loop_timeout(false, Duration::from_millis(30), Some(SPINNER_TICK)),
        SPINNER_TICK - Duration::from_millis(30)
    );
    // Overdue: wake at once. Never longer than the idle poll.
    assert_eq!(loop_timeout(false, Duration::from_secs(5), Some(FX_TICK)), Duration::ZERO);
    assert!(loop_timeout(false, Duration::ZERO, Some(Duration::from_secs(9))) <= IDLE_POLL);
    // A dirty loop is never slower than the frame cap, so an event is
    // drawn within a frame whatever is animating.
    assert!(loop_timeout(true, Duration::ZERO, Some(FX_TICK)) <= FRAME_CAP);
}

#[test]
fn an_idle_wall_schedules_no_timed_redraw() {
    // Restorable sessions show a static placeholder: nothing is in flight.
    let mut app = app_with(true, vec![si_restorable("a", "one"), si_restorable("a", "two")]);
    draw(&mut app, 10_000);
    assert_eq!(app.motion.next_tick(), None);
    // And with no sessions at all.
    let mut empty = app_with(true, vec![]);
    draw(&mut empty, 10_000);
    assert_eq!(empty.motion.next_tick(), None);
}

#[test]
fn an_attaching_tile_spins_and_asks_for_the_spinner_tick() {
    let mut app = app_with(true, vec![si("a", "one")]);
    let screen = draw(&mut app, 0);
    assert!(screen.contains("attaching"));
    assert!(!screen.contains("… attaching"), "the throbber replaces the dots");
    assert_eq!(app.motion.next_tick(), Some(SPINNER_TICK));
    // The frame a tick later shows the next throbber step.
    let later = draw(&mut app, SPINNER_TICK.as_millis() as i64);
    assert_ne!(screen, later);
}

#[test]
fn a_restoring_tile_spins_until_the_restore_settles() {
    let mut app = app_with(true, vec![si_restorable("a", "one")]);
    let id = "garage/a/one";
    draw(&mut app, 0);
    assert_eq!(app.motion.next_tick(), None, "restorable: static");
    app.store.focus_session(id);
    let outcome = app.router.on_garage_key(&mut app.store, &app.keymap, "\r", 0);
    if let Some((action, ctx)) = outcome.action {
        let _ = app.dispatch_action(action, &ctx);
    }
    assert!(app.router.restoring().contains(id), "Enter began a restore");
    let screen = draw(&mut app, 0);
    assert!(screen.contains("restoring…") && !screen.contains("⟳ restoring"));
    assert_eq!(app.motion.next_tick(), Some(SPINNER_TICK));
    app.apply(AppEvent::RestoreSettled(id.to_owned()));
    draw(&mut app, 100);
    assert_eq!(app.motion.next_tick(), None);
}

#[test]
fn motion_off_draws_the_static_markers_and_never_ticks() {
    let mut app = app_with(false, vec![si("a", "one")]);
    let screen = draw(&mut app, 0);
    assert!(screen.contains("… attaching"));
    assert_eq!(app.motion.next_tick(), None);
    assert_eq!(draw(&mut app, 500), screen, "frames do not change with time");
}

#[test]
fn the_strip_shows_a_spinner_only_while_the_daemon_is_away() {
    let mut app = app_with(true, vec![si_restorable("a", "one")]);
    assert!(!draw(&mut app, 0).contains("reconnecting"));
    app.apply(AppEvent::Connection(false));
    let screen = draw(&mut app, 0);
    assert!(screen.contains("reconnecting…"));
    assert_eq!(app.motion.next_tick(), Some(SPINNER_TICK));
    app.apply(AppEvent::Connection(true));
    assert!(!draw(&mut app, 200).contains("reconnecting"));
    assert_eq!(app.motion.next_tick(), None);
}

#[test]
fn a_keystroke_is_applied_immediately_while_effects_run() {
    let mut app = app_with(true, vec![si("a", "one")]);
    draw(&mut app, 0);
    app.motion.begin_frame(0);
    app.motion.tile_drawn("garage/a/one", Rect::new(0, 0, 20, 6), true, false);
    assert!(app.motion.effects_running());
    // `?` opens help inside `apply` itself: nothing waits for a frame.
    assert!(!app.apply(AppEvent::Term(Event::Key(plain(KeyCode::Char('?'))))));
    assert_eq!(app.store.state().overlay, Some(OverlayKind::Help));
}
