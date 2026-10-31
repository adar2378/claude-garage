//! Engaged-layer and overlay-layer tests (help, triage queue, add-workspace,
//! view picker).
use super::*;

// ── engaged layer ───────────────────────────────────────────────────

#[test]
fn ctrl_g_disengages_and_is_never_forwarded() {
    assert_eq!(
        engaged_input_for(&KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
        EngagedInput::Disengage
    );
}

#[test]
fn esc_is_always_forwarded_while_engaged() {
    // Spec scenario "Esc reaches Claude Code": 0x1b, layer untouched.
    assert_eq!(
        engaged_input_for(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        EngagedInput::Write(vec![0x1b])
    );
}

#[test]
fn engaged_bytes_match_the_spec_scenarios() {
    let cases: [(KeyCode, KeyModifiers, &[u8]); 4] = [
        (KeyCode::Char('c'), KeyModifiers::CONTROL, b"\x03"),
        (KeyCode::Right, KeyModifiers::ALT, b"\x1b[1;3C"),
        (KeyCode::BackTab, KeyModifiers::SHIFT, b"\x1b[Z"),
        (KeyCode::Char('h'), KeyModifiers::NONE, b"h"),
    ];
    for (code, mods, expected) in cases {
        assert_eq!(
            engaged_input_for(&KeyEvent::new(code, mods)),
            EngagedInput::Write(expected.to_vec()),
            "{code:?} {mods:?}"
        );
    }
}

#[test]
fn a_bare_modifier_press_forwards_nothing() {
    use crossterm::event::ModifierKeyCode;
    assert_eq!(
        engaged_input_for(&KeyEvent::new(
            KeyCode::Modifier(ModifierKeyCode::LeftControl),
            KeyModifiers::CONTROL
        )),
        EngagedInput::Nothing
    );
}

// ── overlay layer (task 4.3/4.4: real handlers keyed by OverlayKind) ─

/// Route one overlay key with a permissive dir_exists (tests that need
/// validation inject their own).
fn overlay_key(
    store: &mut WallStore,
    selection: &mut usize,
    form: &mut WorkspaceAddForm,
    key: KeyEvent,
) -> Vec<Effect> {
    let mut view_picker = ViewPickerState::default();
    handle_overlay_key(store, &key, selection, form, &mut view_picker, "/home/me", &|_| true).0
}

#[test]
fn question_mark_toggles_help_and_esc_or_q_also_close_it() {
    for closer in [
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
        plain(KeyCode::Char('q')),
        plain(KeyCode::Esc),
    ] {
        let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
        let mut router = GarageRouter::default();
        key_at(&mut router, &mut store, "?", 1000);
        assert_eq!(store.state().overlay, Some(OverlayKind::Help));
        let (mut sel, mut form) = (0, WorkspaceAddForm::default());
        overlay_key(&mut store, &mut sel, &mut form, closer);
        assert_eq!(store.state().layer, KeyLayer::Garage);
        assert_eq!(store.state().overlay, None);
    }
}

#[test]
fn help_overlay_consumes_other_keys_without_closing() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::Help);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('x')));
    assert_eq!(store.state().overlay, Some(OverlayKind::Help));
}

#[test]
fn queue_j_k_wrap_the_selection_and_esc_closes() {
    let mut store = store_with(
        vec![ws("a")],
        vec![
            si_blocked("a", "b1", 100),
            si_blocked("a", "b2", 200),
            si("a", "live"),
        ],
    );
    store.open_overlay(OverlayKind::TriageQueue);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('j')));
    assert_eq!(sel, 1);
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('j')));
    assert_eq!(sel, 0, "wraps past the end");
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('k')));
    assert_eq!(sel, 1, "wraps backwards too");
    // Garage keys must not leak through the overlay layer.
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char('q')));
    assert_eq!(store.state().overlay, Some(OverlayKind::TriageQueue));
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Esc));
    assert_eq!(store.state().layer, KeyLayer::Garage);
}

#[test]
fn queue_enter_jump_engages_the_selected_session() {
    let mut store = store_with(
        vec![ws("a"), ws("b")],
        vec![si("a", "one"), si_blocked("b", "old", 100), si_blocked("a", "young", 900)],
    );
    store.open_overlay(OverlayKind::TriageQueue);
    let (mut sel, mut form) = (1, WorkspaceAddForm::default());
    // Row 0 = b/old (longest waiting), row 1 = a/young.
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter));
    assert_eq!(store.state().overlay, None);
    assert_eq!(store.state().layer, KeyLayer::Engaged);
    assert_eq!(store.state().focused_session_id, Some(id("a", "young")));
}

#[test]
fn queue_enter_on_an_empty_queue_just_closes() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::TriageQueue);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter));
    assert_eq!(store.state().overlay, None);
    assert_eq!(store.state().layer, KeyLayer::Garage);
}

#[test]
fn workspace_add_types_into_the_field_and_esc_cancels() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::WorkspaceAdd);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    for c in ['~', '/', 'q', 'x'] {
        overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Char(c)));
    }
    assert_eq!(form.input, "~/qx", "q types — it never quits the overlay");
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Backspace));
    assert_eq!(form.input, "~/q");
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Esc));
    assert_eq!(store.state().layer, KeyLayer::Garage);
    assert_eq!(form.input, "", "Esc resets the form");
}

#[test]
fn workspace_add_enter_submits_the_validated_path_as_an_effect() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::WorkspaceAdd);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    form.input = "~/dev/proj".to_owned();
    let effects = overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter));
    assert_eq!(
        effects,
        vec![Effect::AddWorkspace {
            name: "proj".to_owned(),
            dir: "/home/me/dev/proj".to_owned(),
        }]
    );
    assert!(form.busy, "PUT in flight");
    assert_eq!(
        store.state().overlay,
        Some(OverlayKind::WorkspaceAdd),
        "stays open until the settle"
    );
}

#[test]
fn workspace_add_validation_failure_keeps_the_overlay_open_with_the_error() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::WorkspaceAdd);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    form.input = "/nope".to_owned();
    let mut view_picker = ViewPickerState::default();
    let (effects, _) = handle_overlay_key(
        &mut store,
        &plain(KeyCode::Enter),
        &mut sel,
        &mut form,
        &mut view_picker,
        "/h",
        &|_| false,
    );
    assert!(effects.is_empty());
    assert_eq!(form.error.as_deref(), Some("no such directory: /nope"));
    assert_eq!(store.state().overlay, Some(OverlayKind::WorkspaceAdd));
}

#[test]
fn workspace_add_ctrl_o_fires_the_picker_once_and_blocks_enter() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::WorkspaceAdd);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    form.input = "~/dev/proj".to_owned();
    let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    let effects = overlay_key(&mut store, &mut sel, &mut form, ctrl_o);
    assert_eq!(effects, vec![Effect::PickDirectory]);
    assert!(form.picking);
    assert_eq!(form.input, "~/dev/proj", "Ctrl+O never types an o");
    // While the dialog is open: a second Ctrl+O and Enter are no-ops.
    assert!(overlay_key(&mut store, &mut sel, &mut form, ctrl_o).is_empty());
    assert!(overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter)).is_empty());
    assert!(!form.busy);
    // Esc still closes, and clears the in-flight pick.
    overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Esc));
    assert_eq!(store.state().overlay, None);
    assert!(!form.picking);
}

#[test]
fn workspace_add_ctrl_o_is_ignored_while_registering() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::WorkspaceAdd);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    form.busy = true;
    let ctrl_o = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert!(overlay_key(&mut store, &mut sel, &mut form, ctrl_o).is_empty());
    assert!(!form.picking);
}

#[test]
fn pastes_route_only_to_the_tile_or_the_workspace_field() {
    assert_eq!(paste_target(KeyLayer::Engaged, None), PasteTarget::Tile);
    assert_eq!(
        paste_target(KeyLayer::Overlay, Some(OverlayKind::WorkspaceAdd)),
        PasteTarget::WorkspaceField
    );
    for overlay in [
        OverlayKind::Help,
        OverlayKind::TriageQueue,
        OverlayKind::ViewPicker,
        OverlayKind::WorktreeFinish,
    ] {
        assert_eq!(
            paste_target(KeyLayer::Overlay, Some(overlay)),
            PasteTarget::Drop,
            "{overlay:?} must swallow pastes"
        );
    }
    assert_eq!(paste_target(KeyLayer::Garage, None), PasteTarget::Drop);
}

#[test]
fn a_paste_in_the_workspace_overlay_lands_in_the_field() {
    // The WorkspaceField branch of handle_paste, end to end on the form.
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    store.open_overlay(OverlayKind::WorkspaceAdd);
    let mut form = WorkspaceAddForm::default();
    let state = store.state();
    assert_eq!(
        paste_target(state.layer, state.overlay),
        PasteTarget::WorkspaceField
    );
    form.insert_paste("/Users/me/dev/My\\ Proj\n");
    assert_eq!(form.input, "/Users/me/dev/My Proj");
}

#[test]
fn workspace_add_name_collides_against_existing_groups() {
    let mut store = store_with(vec![ws("proj")], vec![]);
    store.open_overlay(OverlayKind::WorkspaceAdd);
    let (mut sel, mut form) = (0, WorkspaceAddForm::default());
    form.input = "/x/proj".to_owned();
    let effects = overlay_key(&mut store, &mut sel, &mut form, plain(KeyCode::Enter));
    assert_eq!(
        effects,
        vec![Effect::AddWorkspace {
            name: "proj-2".to_owned(),
            dir: "/x/proj".to_owned(),
        }]
    );
}

// ── p10 views: d/D strip notices (design.md "detached <label>" /
// "moved <label> → <view>" — kept stable for e2e greps) ────────────────

#[test]
fn d_detaching_shows_the_detached_notice() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "two"));
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "d", 1000);
    assert_eq!(router.notices.current(1001), Some("detached two"));
}

#[test]
fn d_rejoining_shows_no_notice() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "two"));
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "d", 1000); // detach: notice shown, ttl 2000ms
    key_at(&mut router, &mut store, "d", 3500); // rejoin, well after that notice expired
    assert_eq!(router.notices.current(3600), None, "a rejoin raises no notice of its own");
}

#[test]
fn d_with_no_focused_session_is_a_silent_no_op() {
    let mut store = store_with(vec![ws("a")], vec![]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "d", 1000);
    assert_eq!(router.notices.current(1001), None);
}

#[test]
fn shift_d_opens_the_picker_overlay() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one")]);
    let mut router = GarageRouter::default();
    key_at(&mut router, &mut store, "D", 1000);
    assert_eq!(store.state().overlay, Some(OverlayKind::ViewPicker));
}

#[test]
fn picker_entries_exclude_the_focused_sessions_current_view() {
    // Exactly the reported repro shape: two sessions, `d` detaches the
    // focused one into its own solo view, then `D` must never offer that
    // solo view as a destination for itself.
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "one"));
    store.detach_focused(); // "one" -> its own solo view "one"; "two" stays in "main"
    assert_eq!(
        view_picker_entries(&store),
        ["main", "new group…"],
        "not [\"main\", \"one\", \"new group…\"]"
    );

    // Cross-check the other side of the same pair: focused back on
    // "main" (holding "two"), "main" itself must now be the excluded one.
    store.focus_view_of(&id("a", "two"));
    assert_eq!(view_picker_entries(&store), ["one", "new group…"]);
}

#[test]
fn picker_enter_on_an_existing_view_moves_and_shows_the_moved_notice() {
    let mut store =
        store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two"), si("a", "three")]);
    store.focus_session(&id("a", "two"));
    store.detach_focused(); // "two" -> solo view "two"
    store.focus_view_of(&id("a", "three"));
    store.open_view_picker();

    // Focused session "three" sits in "main" — its own current view, so
    // "main" is excluded. Entries are ["two", "new group…"]; select
    // "two" (index 0).
    let mut view_picker = ViewPickerState { selected: 0, ..Default::default() };
    let notice =
        handle_view_picker_key(&mut store, &plain(KeyCode::Enter), None, &mut view_picker);
    assert_eq!(notice.as_deref(), Some("moved three → two"));
    assert_eq!(store.state().overlay, None, "picker closes on selection");
    let view = store
        .state()
        .views_for("a")
        .into_iter()
        .find(|v| v.name == "two")
        .unwrap();
    assert_eq!(view.session_ids, [id("a", "two"), id("a", "three")]);
}

#[test]
fn picker_new_group_flow_creates_a_named_group_and_shows_the_moved_notice() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "one"));
    store.open_view_picker();

    // "one" and "two" are both still in "main" — the focused session's
    // own current view, so it's excluded, leaving just ["new group…"].
    let mut view_picker = ViewPickerState { selected: 0, ..Default::default() };
    assert_eq!(
        handle_view_picker_key(&mut store, &plain(KeyCode::Enter), None, &mut view_picker),
        None,
        "switches to the text-input sub-mode; nothing moved yet"
    );
    assert!(view_picker.new_group_input.is_some());

    for c in "backend".chars() {
        handle_view_picker_key(
            &mut store,
            &KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            Some(c),
            &mut view_picker,
        );
    }
    let notice =
        handle_view_picker_key(&mut store, &plain(KeyCode::Enter), None, &mut view_picker);
    assert_eq!(notice.as_deref(), Some("moved one → backend"));
    assert_eq!(store.state().focused_view_name("a"), "backend");
}

#[test]
fn picker_esc_cancels_from_either_mode_without_moving_anything() {
    let mut store = store_with(vec![ws("a")], vec![si("a", "one"), si("a", "two")]);
    store.focus_session(&id("a", "one"));
    store.open_view_picker();
    let mut view_picker = ViewPickerState::default();

    handle_view_picker_key(&mut store, &plain(KeyCode::Esc), None, &mut view_picker);
    assert_eq!(store.state().overlay, None);
    assert_eq!(store.state().views_for("a").len(), 1, "nothing moved");

    store.open_view_picker();
    view_picker.new_group_input = Some("partial".to_owned());
    handle_view_picker_key(&mut store, &plain(KeyCode::Esc), None, &mut view_picker);
    assert_eq!(store.state().overlay, None);
    assert!(view_picker.new_group_input.is_none());
}
