//! Overlay layer: key routing for the help, triage, add-workspace, view
//! picker and worktree-finish overlays, plus the view picker and worktree
//! finish helpers (moved out of `runtime/mod.rs`, p22 task 1.4).

use super::*;

/// Overlay-layer key routing (task 4.3/4.4): the overlay layer consumes
/// everything, keyed by [`OverlayKind`]. Help: Esc/`?`/`q` close. Triage
/// queue: `j`/`k` (and arrows) move the selection with wrap, Enter closes
/// and jump-engages the selected session, Esc closes. Add-workspace: the
/// text field owns the keys — Enter submits (validation via the injected
/// `dir_exists`, so this routes pure in tests), Ctrl+O opens the native
/// folder picker (ignored while busy or already picking), Esc cancels and
/// resets the form. Returned effects are the caller's IO (the PUT / the
/// picker call).
pub fn handle_overlay_key(
    store: &mut WallStore,
    key: &KeyEvent,
    queue_selection: &mut usize,
    form: &mut WorkspaceAddForm,
    view_picker: &mut ViewPickerState,
    home: &str,
    dir_exists: &dyn Fn(&str) -> bool,
) -> (Vec<Effect>, Option<String>) {
    let Some(kind) = store.state().overlay else {
        return (Vec::new(), None);
    };
    let mut effects = Vec::new();
    let mut notice = None;
    let plain_char = match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(c)
        }
        _ => None,
    };
    match kind {
        // Help and the triage queue: pure key → action overlays, resolved
        // through the keymap (the text-input overlays below own their keys).
        OverlayKind::Help | OverlayKind::TriageQueue => {
            let action = overlay_key_string(key)
                .and_then(|k| Keymap::shared_defaults().resolve(Layer::Overlay(kind), &k))
                .map(|r| r.action);
            let len = triage_queue_rows(&store.state().sessions).len();
            match action {
                Some(ids::OVERLAY_CLOSE) => store.close_overlay(),
                Some(ids::OVERLAY_SELECT_NEXT) => {
                    *queue_selection = wrap_selection(*queue_selection, 1, len);
                }
                Some(ids::OVERLAY_SELECT_PREV) => {
                    *queue_selection = wrap_selection(*queue_selection, -1, len);
                }
                Some(ids::JUMP_BLOCKED) => {
                    // Close first, then jump — jump_to_session refuses stale
                    // rows (gone / no longer blocked), same as the Dart flow.
                    store.close_overlay();
                    let target = triage_queue_rows(&store.state().sessions)
                        .get((*queue_selection).min(len.saturating_sub(1)))
                        .map(|s| s.id.clone());
                    if let Some(id) = target {
                        store.jump_to_session(&id);
                    }
                }
                _ => {}
            }
        }
        OverlayKind::WorkspaceAdd => match key.code {
            KeyCode::Esc => {
                form.reset();
                store.close_overlay();
            }
            KeyCode::Backspace => form.backspace(),
            KeyCode::Enter => {
                let existing: Vec<String> = store
                    .state()
                    .workspaces
                    .iter()
                    .map(|w| w.name.clone())
                    .chain(store.state().groups.iter().map(|g| g.name.clone()))
                    .collect();
                if let Some((name, dir)) = form.submit(home, &existing, dir_exists) {
                    effects.push(Effect::AddWorkspace { name, dir });
                }
            }
            // Before the plain-char arm: Ctrl+O must never type an `o`.
            KeyCode::Char('o' | 'O') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if form.start_pick() {
                    effects.push(Effect::PickDirectory);
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                form.insert_char(c);
            }
            _ => {}
        },
        OverlayKind::ViewPicker => {
            notice = handle_view_picker_key(store, key, plain_char, view_picker);
        }
        // Routed by `App::handle_key` to [`handle_worktree_finish_key`]: the
        // armed discard needs the clock, which this router never sees.
        OverlayKind::WorktreeFinish => {}
    }
    (effects, notice)
}

/// The view-picker's selectable rows for the CURRENTLY focused
/// workspace/session (spec tui-views "Move to a group"): every other view of
/// the workspace plus the trailing "new group…" row — the focused session's
/// own current view is excluded (see [`picker_entries`]). Shared by the key,
/// click, and render paths so all three can never disagree about what's
/// selectable — the row a keypress or click resolves to is always the same
/// row the modal draws.
pub(super) fn view_picker_entries(store: &WallStore) -> Vec<String> {
    let Some(workspace) = store.state().focused_workspace.clone() else {
        return picker_entries(&[], "");
    };
    let current_view = store.state().focused_view_name(&workspace).to_owned();
    let view_names: Vec<String> =
        store.state().views_for(&workspace).into_iter().map(|v| v.name).collect();
    picker_entries(&view_names, &current_view)
}

/// `D` picker key routing (spec tui-views "Move to a group"): `j`/`k`/arrows
/// move the selection, Enter on a view moves the focused session there
/// (closing the overlay and returning the "moved <label> → <view>" strip
/// notice), Enter on the trailing "new group…" row switches to its
/// text-input sub-mode (reusing the workspace_add field pattern) whose own
/// Enter creates/moves into that name; Esc cancels from either sub-mode.
pub(super) fn handle_view_picker_key(
    store: &mut WallStore,
    key: &KeyEvent,
    plain_char: Option<char>,
    view_picker: &mut ViewPickerState,
) -> Option<String> {
    let entries = view_picker_entries(store);

    if view_picker.new_group_input.is_some() {
        match key.code {
            KeyCode::Esc => {
                view_picker.reset();
                store.close_overlay();
            }
            KeyCode::Backspace => view_picker.backspace(),
            KeyCode::Enter => {
                let name = view_picker.new_group_input.clone().unwrap_or_default();
                let trimmed = name.trim().to_owned();
                view_picker.reset();
                if trimmed.is_empty() {
                    store.close_overlay();
                } else {
                    return move_to_view_and_notice(store, &trimmed);
                }
            }
            KeyCode::Char(c) if plain_char == Some(c) => view_picker.insert_char(c),
            _ => {}
        }
        return None;
    }

    // The trailing "new group…" row is always the last entry (`picker_entries`
    // guarantees it — never empty even with zero other views).
    let entries_len = entries.len();
    match (key.code, plain_char) {
        (KeyCode::Esc, _) => {
            view_picker.reset();
            store.close_overlay();
        }
        (KeyCode::Down, _) | (_, Some('j')) => {
            view_picker.selected = wrap_selection(view_picker.selected, 1, entries_len);
        }
        (KeyCode::Up, _) | (_, Some('k')) => {
            view_picker.selected = wrap_selection(view_picker.selected, -1, entries_len);
        }
        (KeyCode::Enter, _) => {
            let sel = view_picker.selected.min(entries_len.saturating_sub(1));
            if sel == entries_len - 1 {
                view_picker.new_group_input = Some(String::new());
            } else if let Some(name) = entries.get(sel).cloned() {
                view_picker.reset();
                return move_to_view_and_notice(store, &name);
            }
        }
        _ => {}
    }
    None
}

/// Close the picker, move the focused session into `view_name`, and return
/// the strip notice on success (spec tui-views design.md: "moved <label> →
/// <view>" — kept stable for e2e greps).
pub(super) fn move_to_view_and_notice(store: &mut WallStore, view_name: &str) -> Option<String> {
    let label = store
        .state()
        .session_by_id(store.state().focused_session_id.as_deref())
        .map(|s| s.label.clone());
    store.close_overlay();
    if store.move_focused_to_view(view_name) {
        label.map(|label| format!("moved {label} → {view_name}"))
    } else {
        None
    }
}

/// Finish-overlay key routing (spec tui-worktree-finish): `m` merges, `d`
/// arms and a second `d` within the window discards, `k`/Esc keeps (no
/// request; the overlay closes once no record waits). Everything else is
/// consumed — the overlay is modal — and disarms a pending discard. While a
/// request is in flight every choice is ignored.
pub(super) fn handle_worktree_finish_key(
    store: &mut WallStore,
    key: &KeyEvent,
    finish: &mut WorktreeFinishState,
    now_ms: i64,
) -> Vec<Effect> {
    let plain_char = match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(c)
        }
        _ => None,
    };
    let choice = match (key.code, plain_char) {
        (KeyCode::Esc, _) | (_, Some('k')) => finish.keep(),
        (_, Some('m')) => finish.merge(),
        (_, Some('d')) => finish.discard(now_ms),
        _ => {
            finish.other_key();
            FinishChoice::Nothing
        }
    };
    match choice {
        FinishChoice::Run(record, action) => vec![Effect::FinishWorktree { record, action }],
        FinishChoice::Kept { close: true } => {
            store.close_overlay();
            Vec::new()
        }
        FinishChoice::Kept { close: false } | FinishChoice::Nothing => Vec::new(),
    }
}

/// Queue a closed session's worktree record and bring the finish overlay up
/// (spec tui-worktree-finish). The DELETE settles asynchronously, so the
/// wall may have moved on: an engaged tile is disengaged and any other
/// overlay is closed — the record is the only handle on the worktree and
/// must not wait behind them. Returns true when another overlay was
/// displaced (the caller resets that overlay's state).
pub(super) fn show_worktree_finish(
    store: &mut WallStore,
    finish: &mut WorktreeFinishState,
    record: WorktreeRecord,
) -> bool {
    finish.push(record);
    let mut displaced = false;
    match (store.state().layer, store.state().overlay) {
        (KeyLayer::Overlay, Some(OverlayKind::WorktreeFinish)) => return false,
        (KeyLayer::Overlay, _) => {
            store.close_overlay();
            displaced = true;
        }
        (KeyLayer::Engaged, _) => store.disengage(),
        (KeyLayer::Garage, _) => {}
    }
    store.open_overlay(OverlayKind::WorktreeFinish);
    displaced
}

/// Apply a finish call's settle: success returns the strip notice and
/// closes the overlay once no record waits; a failure keeps the overlay
/// open with the daemon's message inline (no notice).
pub(super) fn worktree_finish_settled(
    store: &mut WallStore,
    finish: &mut WorktreeFinishState,
    record: &WorktreeRecord,
    action: FinishAction,
    error: Option<String>,
) -> Option<String> {
    let failed = error.is_some();
    let close = finish.settled(error);
    if close && store.state().overlay == Some(OverlayKind::WorktreeFinish) {
        store.close_overlay();
    }
    (!failed).then(|| finish_notice(record, action))
}

impl App {
    /// A click inside the `D` view-picker overlay (task 3.3 "picker overlay
    /// clicks"): outside the modal is Esc; inside, while the "new group…"
    /// text field owns the keys, clicks do nothing (same as
    /// `workspace_add`'s field); otherwise a row click selects — the
    /// trailing "new group…" row switches to that sub-mode, any other row
    /// moves the focused session there and shows the "moved" notice.
    pub(super) fn pick_view_entry(&mut self, i: usize) {
        let entries = view_picker_entries(&self.store);
        let entries_len = entries.len();
        if i == entries_len - 1 {
            self.view_picker.selected = i;
            self.view_picker.new_group_input = Some(String::new());
            return;
        }
        let Some(name) = entries.get(i).cloned() else {
            return;
        };
        let label = self
            .store
            .state()
            .session_by_id(self.store.state().focused_session_id.as_deref())
            .map(|s| s.label.clone());
        self.store.close_overlay();
        if self.store.move_focused_to_view(&name) {
            if let Some(label) = label {
                self.router.notices.show(format!("moved {label} → {name}"), 2000, now_ms());
            }
        }
        self.view_picker.reset();
    }
}
