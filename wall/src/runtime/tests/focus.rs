//! Terminal focus reports drive what the visibility heartbeat says, and are
//! consumed by the wall in every layer (never typed into an engaged session).
use super::*;

fn app() -> App {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(tx, "http://127.0.0.1:0");
    app.store.workspaces_fetched(vec![ws("a")]);
    app.store.sessions_fetched(vec![si("a", "one")]);
    app
}

fn term(ev: Event) -> AppEvent {
    AppEvent::Term(ev)
}

#[test]
fn focus_lost_posts_false_and_gained_posts_true_at_once() {
    let mut app = app();
    assert!(!app.apply(term(Event::FocusLost)));
    assert_eq!(app.visibility_post.take(), Some(false));
    assert!(!app.focus.visible());
    app.apply(term(Event::FocusGained));
    assert_eq!(app.visibility_post.take(), Some(true));
    assert!(app.focus.visible());
}

#[test]
fn the_heartbeat_follows_the_current_state() {
    let mut app = app();
    assert!(app.focus.visible(), "running and no event yet: visible");
    app.apply(term(Event::FocusLost));
    assert!(!app.focus.visible(), "a heartbeat now says false");
    app.apply(term(Event::FocusGained));
    assert!(app.focus.visible());
}

#[test]
fn a_terminal_that_never_reports_focus_stays_visible() {
    let mut app = app();
    for _ in 0..3 {
        app.apply(term(Event::Resize(80, 24)));
    }
    assert!(app.focus.visible());
    assert_eq!(app.visibility_post, None, "nothing to post");
    // a repeated report that changes nothing posts nothing either
    app.apply(term(Event::FocusGained));
    assert_eq!(app.visibility_post, None);
}

#[test]
fn an_engaged_session_still_counts_as_visible_when_focused_and_never_sees_the_reports() {
    let mut app = app();
    app.store.engage();
    assert_eq!(app.store.state().layer, KeyLayer::Engaged);
    let before = format!("{:?}", app.store.state());
    app.apply(term(Event::FocusLost));
    assert!(!app.focus.visible(), "engaged but not focused: not in view");
    app.apply(term(Event::FocusGained));
    assert!(app.focus.visible(), "engaged and focused: in view");
    assert_eq!(
        format!("{:?}", app.store.state()),
        before,
        "the reports are consumed: no key, no disengage, no draft"
    );
}
