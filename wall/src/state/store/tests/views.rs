//! Views (p10): detach/rejoin, cycle, move-to-view, focus_view and the
//! vacated-default-view regressions.
use super::*;

// ── views (p10): detach/rejoin, cycle, per-view grid, persistence hook ──

fn eight() -> Vec<SessionInfo> {
    (1..=8).map(|i| si("a", &format!("s{i}"))).collect()
}

#[test]
fn p10_new_session_joins_the_default_view_leaving_other_views_unchanged() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "two"));
    assert!(store.detach_focused()); // "two" -> its own solo view "two"

    // A new session arrives (spawn/restore/unregistered discovery all
    // reconcile the same way): it must land in the default view, and
    // the solo view must be untouched.
    store.sessions_fetched(vec![si("a", "one"), si("a", "two"), si("a", "three")]);
    let views = store.state().views_for("a");
    let main = views.iter().find(|v| v.name == DEFAULT_VIEW).unwrap();
    let solo = views.iter().find(|v| v.name == "two").unwrap();
    assert_eq!(main.session_ids, [id("a", "one"), id("a", "three")]);
    assert_eq!(solo.session_ids, [id("a", "two")]);
}

#[test]
fn p10_d_detaches_a_multi_session_view_to_a_solo_view_and_back() {
    // Spec tui-views "Detach and rejoin", scenario "d detaches to a
    // solo view".
    let mut store =
        store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two"), si("a", "three")]);
    store.focus_session(&id("a", "three"));

    assert!(store.detach_focused());
    assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));
    assert_eq!(store.state().focused_view_name("a"), "three");
    assert_eq!(store.state().gridded_session_ids, [id("a", "three")]);
    let origin = store
        .state()
        .views_for("a")
        .into_iter()
        .find(|v| v.name == DEFAULT_VIEW)
        .unwrap();
    assert_eq!(origin.session_ids.len(), 2, "the origin view has 2 sessions");

    assert!(store.detach_focused()); // pressing d again
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
    assert_eq!(store.state().focused_session_id, Some(id("a", "three")));
    assert!(store.state().views.get("a").is_none_or(Vec::is_empty));
}

#[test]
fn p10_detach_names_the_solo_view_after_the_label_avoiding_the_reserved_default_name() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "main"), si("a", "other")]);
    store.focus_session(&id("a", "main"));
    assert!(store.detach_focused());
    // "main" is reserved for the default view — derive_view_name parity.
    assert_eq!(store.state().focused_view_name("a"), "main-2");
}

#[test]
fn p10_an_empty_non_default_view_is_removed_when_its_last_session_dies() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "two"));
    store.detach_focused();
    assert_eq!(store.state().views_for("a").len(), 2);

    store.sessions_fetched(vec![si("a", "one")]); // "two" is gone entirely
    assert_eq!(store.state().views_for("a").len(), 1);
    assert!(store.state().views.get("a").is_none_or(Vec::is_empty));
    assert_eq!(
        store.state().focused_view_name("a"),
        DEFAULT_VIEW,
        "a focused view pruned out from under it falls back to the default"
    );
    assert_eq!(store.state().focused_session_id, Some(id("a", "one")));
}

#[test]
fn p10_grid_cap_and_lru_are_scoped_to_the_focused_view_not_the_whole_workspace() {
    // 8 sessions in the default view — one already over the 6-cap.
    let mut store = store_with(vec![ws("a")], eight());
    assert_eq!(store.state().gridded_session_ids.len(), 6);

    // Detach the overflow session s8 into its own solo view.
    store.focus_session(&id("a", "s8"));
    assert!(store.detach_focused());
    assert_eq!(store.state().gridded_session_ids, [id("a", "s8")], "a solo view is never capped");

    // The default view still holds 7 (s1..s7) — the cap still applies
    // to it alone, unaffected by the sibling solo view.
    store.cycle_view();
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
    assert_eq!(store.state().gridded_session_ids.len(), 6);
    assert!(!store.state().gridded_session_ids.contains(&id("a", "s8")));
    let rail: Vec<&str> =
        store.state().groups[0].sessions.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(rail.len(), 8, "the rail still lists every session across every view");
}

#[test]
fn p10_tab_cycles_the_focused_workspaces_views_and_focus_follows() {
    let mut store =
        store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two"), si("a", "three")]);
    store.focus_session(&id("a", "three"));
    store.detach_focused();
    assert_eq!(store.state().focused_view_name("a"), "three");

    store.cycle_view();
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
    assert_eq!(store.state().focused_session_id, Some(id("a", "one")));

    store.cycle_view();
    assert_eq!(store.state().focused_view_name("a"), "three");
    assert_eq!(store.state().focused_session_id, Some(id("a", "three")));
}

#[test]
fn p10_tab_is_a_no_op_with_a_single_view() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    let before = store.version();
    store.cycle_view();
    assert_eq!(store.version(), before, "one view: nothing to cycle to");
}

#[test]
fn p10_jump_to_longest_waiting_crosses_views_focusing_the_targets_view() {
    // Spec tui-views "Salience is never trapped by views", scenario
    // "a-jump crosses views".
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si("b", "keep"), si("b", "later")],
    );
    store.focus_session(&id("b", "later"));
    assert!(store.detach_focused()); // "later" -> its own solo view "later"
    store.focus_workspace_named("a");
    assert_eq!(store.state().focused_workspace.as_deref(), Some("a"));

    store.sessions_fetched(vec![
        si("a", "one"),
        si("b", "keep"),
        si_status("b", "later", "needs-input", Some(500)),
    ]);
    assert!(store.jump_to_longest_waiting());
    assert_eq!(store.state().focused_workspace.as_deref(), Some("b"));
    assert_eq!(store.state().focused_view_name("b"), "later");
    assert_eq!(store.state().focused_session_id, Some(id("b", "later")));
    assert_eq!(store.state().layer, KeyLayer::Engaged);
}

#[test]
fn p10_focus_session_crosses_views_within_the_same_workspace_rail_click_semantics() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "two"));
    store.detach_focused(); // "two" -> solo view "two", focused
    store.cycle_view(); // back to the default view, focus on "one"
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);

    // A rail click (or any `focus_session` call) on a session sitting
    // in a background view must bring that view into focus too.
    store.focus_view_of(&id("a", "two"));
    assert_eq!(store.state().focused_view_name("a"), "two");
    assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
    assert_eq!(store.state().gridded_session_ids, [id("a", "two")]);
}

#[test]
fn p10_rail_ordering_and_blocked_count_are_view_independent() {
    let mut store =
        store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two"), si("a", "three")]);
    store.focus_session(&id("a", "three"));
    store.detach_focused(); // "three" -> its own view; still the same workspace/rail.

    store.sessions_fetched(vec![
        si("a", "one"),
        si_status("a", "two", "needs-input", Some(10)),
        si_status("a", "three", "needs-input", Some(5)),
    ]);
    assert_eq!(store.state().blocked_count(), 2, "blocked count spans every view");
    let labels: Vec<&str> = store.state().groups[0].sessions.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        ["two", "three", "one"],
        "salience partitions needs-input first regardless of view membership"
    );
}

#[test]
fn p10_load_views_prunes_dead_sessions_and_is_idempotent_no_spurious_dirty_bump() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    let mut loaded: HashMap<String, Vec<View>> = HashMap::new();
    loaded.insert(
        "a".to_owned(),
        vec![View {
            name: "solo".to_owned(),
            session_ids: vec![id("a", "two"), "garage/a/ghost".to_owned()],
        }],
    );
    let before = store.views_revision();
    store.load_views(loaded);
    assert_eq!(
        store.state().views.get("a").unwrap(),
        &[View { name: "solo".to_owned(), session_ids: vec![id("a", "two")] }],
        "the dead 'ghost' id is pruned on load"
    );
    assert!(store.views_revision() > before, "pruning something is a dirty change");

    let after_load = store.views_revision();
    store.sessions_fetched(vec![si("a", "one"), si("a", "two")]); // nothing new to prune
    assert_eq!(
        store.views_revision(),
        after_load,
        "a refetch that prunes nothing must not bump the debounced-save hook"
    );
}

#[test]
fn p10_detach_bumps_views_revision_cycle_view_does_not() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    let before = store.views_revision();
    store.focus_session(&id("a", "two"));
    store.detach_focused();
    assert!(store.views_revision() > before);

    let after_detach = store.views_revision();
    store.cycle_view();
    assert_eq!(
        store.views_revision(),
        after_detach,
        "Tab only moves focus, never membership — never dirty"
    );
}

#[test]
fn p10_d_is_a_no_op_with_an_empty_wall_or_outside_the_garage_layer() {
    let mut empty = store_with(vec![ws("a")], vec![]);
    assert!(!empty.detach_focused());

    let mut engaged = store_with(vec![ws("a")], vec![si("a", "one")]);
    engaged.engage();
    assert!(!engaged.detach_focused());
    assert_eq!(engaged.state().layer, KeyLayer::Engaged);
}

#[test]
fn p10_d_and_tab_map_to_their_commands_and_dispatch() {
    assert_eq!(garage_command_for("d"), Some(GarageCommand::DetachFocused));
    assert_eq!(garage_command_for("\t"), Some(GarageCommand::CycleView));

    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    assert!(store.dispatch(GarageCommand::DetachFocused));
    assert_eq!(store.state().focused_view_name("a"), "one");
    assert!(store.dispatch(GarageCommand::CycleView)); // 2 views now: cycles.
}

// ── move_focused_to_view / D (spec tui-views "Move to a group") ─────

#[test]
fn p10_shift_d_maps_to_open_view_picker_and_opens_the_overlay() {
    assert_eq!(garage_command_for("D"), Some(GarageCommand::OpenViewPicker));
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(store.dispatch(GarageCommand::OpenViewPicker));
    assert_eq!(store.state().overlay, Some(OverlayKind::ViewPicker));
    assert_eq!(store.state().keys_target_chip(), "keys → move to group");
}

#[test]
fn p10_shift_d_is_a_no_op_on_an_empty_wall() {
    let mut store = store_with(vec![ws("a")], vec![]);
    assert!(!store.dispatch(GarageCommand::OpenViewPicker));
    assert_eq!(store.state().overlay, None);
}

#[test]
fn p10_move_focused_to_view_creates_a_new_group_and_moves_more_sessions_into_it() {
    // Spec tui-views "Move to a group", scenario "Building a
    // three-session group".
    let mut store =
        store_with(vec![ws("a")], vec![si("a", "A"), si("a", "B"), si("a", "C")]);
    store.focus_session(&id("a", "A"));
    assert!(store.move_focused_to_view("backend"));
    assert_eq!(store.state().focused_view_name("a"), "backend");
    assert_eq!(store.state().focused_session_id, Some(id("a", "A")));

    store.focus_view_of(&id("a", "B"));
    assert!(store.move_focused_to_view("backend"));
    store.focus_view_of(&id("a", "C"));
    assert!(store.move_focused_to_view("backend"));

    let views = store.state().views_for("a");
    let names: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["backend"], "every session left main for backend");
    let backend = views.iter().find(|v| v.name == "backend").unwrap();
    assert_eq!(
        backend.session_ids,
        [id("a", "A"), id("a", "B"), id("a", "C")]
    );
}

#[test]
fn p10_moving_the_last_session_out_of_a_view_removes_it() {
    // Spec tui-views "Move to a group", scenario "Move-out removes empty
    // views".
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "two"));
    store.detach_focused(); // "two" -> its own solo view "two"
    assert_eq!(store.state().views_for("a").len(), 2);

    assert!(store.move_focused_to_view(DEFAULT_VIEW));
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
    let views = store.state().views_for("a");
    assert_eq!(views.len(), 1, "solo-x is gone, only main remains");
    assert!(views.iter().all(|v| v.name != "two"));
}

#[test]
fn p10_move_focused_to_view_into_an_existing_named_view_merges_in() {
    let mut store = store_with(
        vec![ws("a")],
        vec![si("a", "one"), si("a", "two"), si("a", "three")],
    );
    store.focus_session(&id("a", "one"));
    store.detach_focused(); // "one" -> solo view "one"
    store.focus_view_of(&id("a", "two"));
    assert!(store.move_focused_to_view("one"));
    let view = store
        .state()
        .views_for("a")
        .into_iter()
        .find(|v| v.name == "one")
        .unwrap();
    assert_eq!(view.session_ids, [id("a", "one"), id("a", "two")]);
}

#[test]
fn p10_move_focused_to_view_bumps_views_revision_and_requires_the_garage_layer() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.engage();
    assert!(!store.move_focused_to_view("backend"));

    store.disengage();
    let before = store.views_revision();
    assert!(store.move_focused_to_view("backend"));
    assert!(store.views_revision() > before);
}

// ── focus_view (view-strip click target) ─────────────────────────────

#[test]
fn p10_focus_view_lands_on_the_views_first_session_and_focuses_it() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "two"));
    store.detach_focused(); // "two" -> solo view "two"
    assert_eq!(store.state().focused_view_name("a"), "two");

    assert!(store.focus_view(DEFAULT_VIEW));
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
    assert_eq!(store.state().focused_session_id, Some(id("a", "one")));

    assert!(store.focus_view("two"));
    assert_eq!(store.state().focused_view_name("a"), "two");
    assert_eq!(store.state().focused_session_id, Some(id("a", "two")));
}

#[test]
fn p10_focus_view_is_a_no_op_for_an_unknown_view_or_no_focused_workspace() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    assert!(!store.focus_view("ghost"));
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
}

// ── bug fix: a fully-vacated default view must never trap focus ────────
// (verification.md "Wall bug found during 4.2": every session `D`-moved
// out of a workspace's default view, then a fresh TUI process starts —
// `focused_view` isn't persisted, so it assumes DEFAULT_VIEW, which now
// has zero members and nothing (`Tab`, `]`) could ever reach the
// populated named view). Covers (a) reconcile/load refocus, (b)
// `cycle_view` never dead-ending, and (c) the empty-workspace hint being
// driven by a state field that's fixed by (a), so it never fires for an
// empty default sitting next to a populated named view.

#[test]
fn p10_bug_restart_shape_load_views_with_everything_in_a_named_view_focuses_it() {
    // The exact repro: two sessions, both already assigned (as if
    // restored from `wall.json`) to "backend", loaded into a brand-new
    // store whose `focused_view` map is empty (so it starts out assuming
    // DEFAULT_VIEW, per-workspace, same as a real process restart).
    let mut store = store_with(vec![ws("a")], vec![si("a", "alpha"), si("a", "beta")]);
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW, "sanity: starts on the default");

    let mut loaded: HashMap<String, Vec<View>> = HashMap::new();
    loaded.insert(
        "a".to_owned(),
        vec![View {
            name: "backend".to_owned(),
            session_ids: vec![id("a", "alpha"), id("a", "beta")],
        }],
    );
    store.load_views(loaded);

    assert_eq!(
        store.state().focused_view_name("a"),
        "backend",
        "the default is now empty; focus must land on the only real view"
    );
    assert_eq!(store.state().gridded_session_ids, [id("a", "alpha"), id("a", "beta")]);
    assert_eq!(store.state().focused_session_id, Some(id("a", "alpha")));
}

#[test]
fn p10_bug_reconcile_refocuses_when_a_live_death_vacates_the_focused_view_and_default_is_also_empty() {
    // Same trap, reached via a live refetch instead of a cold load: two
    // named views, default empty, focus on "frontend"; its only member
    // dies. Pruning drops "frontend" (spec: empty non-default views are
    // removed), which would normally fall back to DEFAULT_VIEW — but
    // DEFAULT_VIEW is *also* still empty, so it must keep going to
    // "backend" instead of landing in the same trap by another route.
    let mut store = store_with(vec![ws("a")], vec![si("a", "x"), si("a", "y")]);
    store.focus_session(&id("a", "x"));
    assert!(store.move_focused_to_view("backend"));
    store.focus_view_of(&id("a", "y"));
    assert!(store.move_focused_to_view("frontend"));
    assert_eq!(store.state().focused_view_name("a"), "frontend");

    store.sessions_fetched(vec![si("a", "x")]); // "y" (frontend's only member) dies
    assert_eq!(
        store.state().focused_view_name("a"),
        "backend",
        "default is still empty too — must land on the only real view left"
    );
    assert_eq!(store.state().gridded_session_ids, [id("a", "x")]);
}

#[test]
fn p10_bug_refocus_is_a_no_op_when_the_focused_view_already_has_members() {
    // Guard against over-firing: a normal, already-correct focus must
    // never be disturbed by the refocus pass.
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "two"));
    assert!(store.detach_focused()); // "two" -> solo view "two", non-empty default remains
    assert_eq!(store.state().focused_view_name("a"), "two");

    store.sessions_fetched(vec![si("a", "one"), si("a", "two")]); // no-op refetch
    assert_eq!(store.state().focused_view_name("a"), "two", "already on a real view — untouched");
}

#[test]
fn p10_bug_a_truly_empty_workspace_is_left_on_the_default_not_redirected() {
    // No sessions at all: nothing non-empty to redirect to, so the
    // (distinct, correct) "no workspaces/sessions" render path is
    // untouched by the refocus pass.
    let store = store_with(vec![ws("a")], vec![]);
    assert_eq!(store.state().focused_view_name("a"), DEFAULT_VIEW);
    assert!(store.state().gridded_session_ids.is_empty());
}

#[test]
fn p10_bug_cycle_view_escapes_an_empty_focused_view_even_with_only_one_real_view_left() {
    // Direct unit test of cycle_view's own dead-end guard (b), forcing
    // exactly the stuck shape independent of the reconcile-side fix (a):
    // `focused_view` pointing at the now-empty default while "backend"
    // is the workspace's only real view.
    let mut store = store_with(vec![ws("a")], vec![si("a", "alpha"), si("a", "beta")]);
    store.focus_session(&id("a", "alpha"));
    assert!(store.move_focused_to_view("backend"));
    store.focus_view_of(&id("a", "beta"));
    assert!(store.move_focused_to_view("backend"));
    store.state.focused_view.insert("a".to_owned(), DEFAULT_VIEW.to_owned());
    assert_eq!(store.state().views_for("a").len(), 1, "sanity: only \"backend\" is real");

    store.cycle_view();

    assert_eq!(store.state().focused_view_name("a"), "backend");
    assert_eq!(store.state().gridded_session_ids, [id("a", "alpha"), id("a", "beta")]);
}

#[test]
fn p10_bug_cycle_view_stays_put_with_a_single_real_view_already_focused() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.focus_session(&id("a", "one"));
    assert!(store.move_focused_to_view("backend")); // the only view, already focused
    let before = store.version();

    store.cycle_view();

    assert_eq!(store.version(), before, "one real view, already on it: nothing to cycle to");
}

#[test]
fn p10_bug_vacated_default_view_never_renders_the_empty_workspace_hint() {
    // runtime.rs's "no sessions in this workspace" hint fires purely off
    // `gridded_session_ids.is_empty()` once a workspace group exists at
    // all — it has no idea about `views`/`focused_view`. So the (a) fix
    // must keep that field non-empty here, or an empty *default* view
    // would wrongly render as though the whole *workspace* were empty.
    let mut store = store_with(vec![ws("a")], vec![si("a", "alpha"), si("a", "beta")]);
    let mut loaded: HashMap<String, Vec<View>> = HashMap::new();
    loaded.insert(
        "a".to_owned(),
        vec![View {
            name: "backend".to_owned(),
            session_ids: vec![id("a", "alpha"), id("a", "beta")],
        }],
    );
    store.load_views(loaded);

    assert!(
        !store.state().gridded_session_ids.is_empty(),
        "must never look empty — the hint would wrongly claim the workspace has no sessions"
    );

    // Contrast: a workspace with genuinely zero sessions legitimately
    // renders an empty grid — the hint is correct there, untouched.
    let empty_store = store_with(vec![ws("b")], vec![]);
    assert!(empty_store.state().gridded_session_ids.is_empty());
}
