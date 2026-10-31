//! Worktree-finish overlay tests (spec tui-worktree-finish).
use super::*;

// ── p17: worktree finish overlay (spec tui-worktree-finish) ─────────

fn wt_record(branch: &str) -> WorktreeRecord {
    WorktreeRecord {
        path: format!("/w/{branch}"),
        branch: branch.to_owned(),
        repo_dir: "/r".to_owned(),
        target: Some("main".to_owned()),
    }
}

fn finish_open(store: &mut WallStore, branch: &str) -> WorktreeFinishState {
    let mut finish = WorktreeFinishState::default();
    assert!(!show_worktree_finish(store, &mut finish, wt_record(branch)));
    assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));
    finish
}

fn finish_key(
    store: &mut WallStore,
    finish: &mut WorktreeFinishState,
    c: char,
    now_ms: i64,
) -> Vec<Effect> {
    handle_worktree_finish_key(store, &plain(KeyCode::Char(c)), finish, now_ms)
}

#[test]
fn worktree_close_opens_the_finish_overlay_even_from_engaged_or_another_overlay() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(store.engage());
    let mut finish = WorktreeFinishState::default();
    assert!(!show_worktree_finish(&mut store, &mut finish, wt_record("garage/x")));
    assert_eq!(store.state().layer, KeyLayer::Overlay);
    assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));

    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::WorkspaceAdd);
    let mut finish = WorktreeFinishState::default();
    assert!(show_worktree_finish(&mut store, &mut finish, wt_record("garage/x")), "displaced");
    assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));
}

#[test]
fn a_second_worktree_close_queues_behind_the_first() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut finish = finish_open(&mut store, "garage/a");
    assert!(!show_worktree_finish(&mut store, &mut finish, wt_record("garage/b")));
    assert_eq!(finish.current().unwrap().branch, "garage/a", "first stays on screen");
    assert_eq!(finish_key(&mut store, &mut finish, 'k', 1000), vec![]);
    assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish), "b still waits");
    assert_eq!(finish.current().unwrap().branch, "garage/b");
    assert_eq!(finish_key(&mut store, &mut finish, 'k', 1100), vec![]);
    assert_eq!(store.state().overlay, None);
}

#[test]
fn m_merges_once_and_ignores_choices_while_busy() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut finish = finish_open(&mut store, "garage/x");
    assert_eq!(
        finish_key(&mut store, &mut finish, 'm', 1000),
        vec![Effect::FinishWorktree {
            record: wt_record("garage/x"),
            action: FinishAction::Merge,
        }]
    );
    for c in ['m', 'd', 'k'] {
        assert_eq!(finish_key(&mut store, &mut finish, c, 1100), vec![], "{c} while busy");
    }
    assert_eq!(
        handle_worktree_finish_key(&mut store, &plain(KeyCode::Esc), &mut finish, 1100),
        vec![]
    );
    assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish), "still open");
}

#[test]
fn d_arms_and_a_second_d_discards_any_other_key_disarms() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut finish = finish_open(&mut store, "garage/x");
    assert_eq!(finish_key(&mut store, &mut finish, 'd', 1000), vec![], "first d arms");
    assert!(finish.discard_armed(1001));
    // An unbound key is consumed (modal) and disarms.
    assert_eq!(finish_key(&mut store, &mut finish, 'z', 1100), vec![]);
    assert!(!finish.discard_armed(1101));
    assert_eq!(finish_key(&mut store, &mut finish, 'd', 1200), vec![], "re-arms");
    assert_eq!(
        finish_key(&mut store, &mut finish, 'd', 1300),
        vec![Effect::FinishWorktree {
            record: wt_record("garage/x"),
            action: FinishAction::Discard,
        }]
    );
}

#[test]
fn keep_and_esc_close_without_any_request() {
    for key in [plain(KeyCode::Char('k')), plain(KeyCode::Esc)] {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut finish = finish_open(&mut store, "garage/x");
        assert_eq!(
            handle_worktree_finish_key(&mut store, &key, &mut finish, 1000),
            vec![],
            "keep never sends a finish request"
        );
        assert_eq!(store.state().overlay, None);
        assert_eq!(finish.current(), None);
    }
}

#[test]
fn the_overlay_is_modal_wall_keys_do_nothing() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut finish = finish_open(&mut store, "garage/x");
    for c in ['q', 'x', 'n', '?'] {
        assert_eq!(finish_key(&mut store, &mut finish, c, 1000), vec![]);
    }
    assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));
    assert_eq!(store.state().keys_target_chip(), "keys → finish worktree");
}

#[test]
fn a_finish_error_keeps_the_overlay_open_with_the_message() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut finish = finish_open(&mut store, "garage/x");
    finish_key(&mut store, &mut finish, 'm', 1000);
    let notice = worktree_finish_settled(
        &mut store,
        &mut finish,
        &wt_record("garage/x"),
        FinishAction::Merge,
        Some("worktree has uncommitted changes".to_owned()),
    );
    assert_eq!(notice, None);
    assert_eq!(store.state().overlay, Some(OverlayKind::WorktreeFinish));
    assert_eq!(finish.error.as_deref(), Some("worktree has uncommitted changes"));
    assert_eq!(finish.busy, None);
    // Choices are live again: keep closes, no request.
    assert_eq!(finish_key(&mut store, &mut finish, 'k', 2000), vec![]);
    assert_eq!(store.state().overlay, None);
}

#[test]
fn a_finish_success_closes_with_a_strip_notice() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut finish = finish_open(&mut store, "garage/x");
    finish_key(&mut store, &mut finish, 'm', 1000);
    let notice = worktree_finish_settled(
        &mut store,
        &mut finish,
        &wt_record("garage/x"),
        FinishAction::Merge,
        None,
    );
    assert_eq!(notice.as_deref(), Some("merged garage/x into main"));
    assert_eq!(store.state().overlay, None);
}
