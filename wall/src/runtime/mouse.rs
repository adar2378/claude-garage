//! Mouse handling: clicks resolve to actions through `hit_targets::action_at`,
//! wheel events drive the scrollback peek (moved out of `runtime/mod.rs`,
//! p22 task 1.5).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::input::links::url_at;
use crate::input::selection::{Selection, local_cell, text_in};
use crate::state::wall_state::RailPrefs;
use crate::state::wall_state::{KeyLayer, OverlayKind, WallSession};
use crate::ui::hit_targets::{self, ClickMap, action_at};
use crate::ui::scroll::WHEEL_LINES;

use super::router::plugin_gate::ClickConfirm;
use super::{App, now_ms, view_picker_entries};

/// What a held left button is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Drag {
    /// Moving the rail's divider.
    Rail,
    /// Selecting text in a tile; `inner` is the tile's terminal area.
    Select {
        session: String,
        inner: Rect,
        anchor: (u16, u16),
    },
}

/// A tile rect minus its 1-cell border: the terminal area.
fn inner_of(rect: Rect) -> Rect {
    Rect {
        x: rect.x + 1,
        y: rect.y + 1,
        width: rect.width.saturating_sub(2),
        height: rect.height.saturating_sub(2),
    }
}

impl App {
    /// The rail's divider column, when the rail is showing.
    fn on_rail_divider(&self, col: u16, row: u16) -> bool {
        self.layout.as_ref().is_some_and(|l| {
            l.rail.width > 0
                && col == l.rail.x + l.rail.width - 1
                && row >= l.rail.y
                && row < l.rail.y + l.rail.height
        })
    }

    /// A left press: starts a divider drag or the anchor of a selection.
    /// Returns true when the press was the divider's (nothing else sees it).
    fn begin_drag(&mut self, col: u16, row: u16) -> bool {
        self.selection = None;
        self.drag = None;
        if self.store.state().overlay.is_some() {
            return false;
        }
        if self.on_rail_divider(col, row) {
            self.drag = Some(Drag::Rail);
            return true;
        }
        let in_view_strip = self
            .layout
            .as_ref()
            .is_some_and(|l| l.view_strip.height > 0 && row == l.view_strip.y);
        if !in_view_strip && let Some((id, rect)) = self.tile_at(col, row) {
            let inner = inner_of(rect);
            if inner.contains((col, row).into()) {
                self.drag = Some(Drag::Select {
                    session: id,
                    inner,
                    anchor: local_cell(inner, col, row),
                });
            }
        }
        false
    }

    fn drag_to(&mut self, col: u16, row: u16) {
        match self.drag.clone() {
            Some(Drag::Rail) => {
                let rail_x = self.layout.as_ref().map_or(0, |l| l.rail.x);
                let width = col.saturating_add(1).saturating_sub(rail_x);
                self.store.set_rail(RailPrefs {
                    hidden: false,
                    width,
                });
            }
            Some(Drag::Select {
                session,
                inner,
                anchor,
            }) => {
                let head = local_cell(inner, col, row);
                self.selection = (head != anchor).then_some(Selection {
                    session,
                    anchor,
                    head,
                });
            }
            None => {}
        }
    }

    /// The button came up: a finished selection is copied right away.
    fn end_drag(&mut self) {
        if matches!(self.drag.take(), Some(Drag::Select { .. })) {
            self.copy_selection();
        }
    }

    /// Queue the highlighted text for the clipboard. The highlight stays so
    /// the user sees what was taken. True when there was text to copy.
    pub(super) fn copy_selection(&mut self) -> bool {
        let Some(sel) = self.selection.clone().filter(|s| !s.is_empty()) else {
            return false;
        };
        let (start, end) = sel.ordered();
        let text = self
            .registry
            .with_screen(&sel.session, |screen| text_in(screen, start, end))
            .unwrap_or_default();
        if text.trim().is_empty() {
            return false;
        }
        let chars = text.chars().count();
        self.copies.push(text);
        self.router
            .notices
            .show(format!("copied {chars} chars"), 1500, now_ms());
        true
    }

    /// Ctrl+C while text is highlighted copies it instead of interrupting
    /// the session; any other key just drops the highlight. True = the key
    /// was consumed.
    pub(super) fn selection_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        if self.selection.is_none() {
            return false;
        }
        let ctrl_c = key.modifiers == crossterm::event::KeyModifiers::CONTROL
            && key.code == crossterm::event::KeyCode::Char('c');
        if ctrl_c && self.copy_selection() {
            self.selection = None;
            return true;
        }
        self.selection = None;
        false
    }

    /// True when the event raised a quit.
    pub(super) fn handle_mouse(&mut self, mouse: &MouseEvent) -> bool {
        // A plugin pane is modal: a click outside it closes it, nothing
        // reaches the wall underneath.
        if self.store.state().overlay == Some(OverlayKind::PluginPane) {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && !self
                    .pane
                    .rect
                    .is_some_and(|r| r.contains((mouse.column, mouse.row).into()))
            {
                self.close_plugin_pane();
            }
            return false;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.begin_drag(mouse.column, mouse.row) {
                    return false;
                }
                return self.handle_click(mouse.column, mouse.row);
            }
            MouseEventKind::Drag(MouseButton::Left) => self.drag_to(mouse.column, mouse.row),
            MouseEventKind::Up(MouseButton::Left) => self.end_drag(),
            MouseEventKind::ScrollUp => {
                self.selection = None; // the text moves under the highlight
                self.handle_wheel(mouse.column, mouse.row, true);
            }
            MouseEventKind::ScrollDown => {
                self.selection = None;
                self.handle_wheel(mouse.column, mouse.row, false);
            }
            _ => {}
        }
        false
    }

    /// The session id under a grid point, honoring maximize (a maximized
    /// tile covers the whole grid, so every grid click/wheel targets it).
    pub(super) fn tile_at(&self, col: u16, row: u16) -> Option<(String, Rect)> {
        hit_targets::tile_at(self.layout.as_ref()?, self.store.state(), col, row)
    }

    /// True when the click raised a quit.
    pub(super) fn handle_click(&mut self, col: u16, row: u16) -> bool {
        let Some(layout) = self.layout.clone() else {
            return false;
        };
        // Click on a URL in a tile's grid text opens it (spec
        // tui-key-routing "Click opens links") — our mouse capture starves
        // the outer terminal of clicks, so the wall must be the linkifier.
        // Checked before focus/engage: a click ON a link means "open this",
        // not "type here". Not an action; everything else resolves below.
        if self.store.state().overlay.is_none()
            && !(layout.view_strip.height > 0 && row == layout.view_strip.y)
            && let Some((id, rect)) = self.tile_at(col, row)
        {
            let inner = Rect {
                x: rect.x + 1,
                y: rect.y + 1,
                width: rect.width.saturating_sub(2),
                height: rect.height.saturating_sub(2),
            };
            if inner.contains((col, row).into())
                && let Some(line) = self.registry.row_text(&id, row - inner.y)
                && let Some(url) = url_at(&line, usize::from(col - inner.x))
            {
                // A click like any other: it cancels a pending
                // plugin arm before it does its own thing.
                if self.router.plugin_origin_arm_pending() {
                    self.router.disarm_plugin();
                }
                let _ = open_url(&url);
                self.router
                    .notices
                    .show(format!("opened {url}"), 2000, now_ms());
                return false;
            }
        }
        let picker_entries = if self.store.state().overlay == Some(OverlayKind::ViewPicker) {
            view_picker_entries(&self.store).len()
        } else {
            0
        };
        let map = ClickMap {
            layout: &layout,
            view_strip_targets: &self.view_strip_targets,
            triage_rect: self.triage_rect,
            ws_add_rect: self.ws_add_rect,
            view_picker_rect: self.view_picker_rect,
            view_picker_entries: picker_entries,
            view_picker_typing: self.view_picker.new_group_input.is_some(),
            badge_cols: self.badge_cols.as_ref(),
            slot_clicks: &self.slot_clicks,
            slots: &self.slots,
        };
        let resolved = action_at(&map, self.store.state(), col, row);
        // A pending plugin arm is confirmed only by a click on the very
        // region that raised it (and never at all for an API request);
        // every other click, on a core target or empty space, cancels it
        // and then lands as usual.
        let now = now_ms();
        self.router.refresh_plugin_arm(now);
        if self.router.plugin_origin_arm_pending() {
            let clicked = resolved.as_ref().and_then(|(_, ctx)| ctx.region.as_ref());
            match self.router.plugin_confirm_click(clicked, now) {
                ClickConfirm::Run(action) => {
                    let (id, ctx) = *action;
                    let effects = self.dispatch_action(id, &ctx);
                    return self.run_effects(effects);
                }
                // A hasty click on the armed region is swallowed whole.
                ClickConfirm::Swallow => return false,
                ClickConfirm::Cancelled => {}
            }
        }
        if let Some((id, ctx)) = resolved {
            let effects = self.dispatch_action(id, &ctx);
            return self.run_effects(effects);
        }
        false
    }

    /// Wheel over a tile's terminal area. A frozen view owns the wheel
    /// regardless of engagement; an engaged LIVE tile forwards the SGR wheel
    /// sequence with tile-local 1-based coords (Claude Code enables mouse
    /// reporting and scrolls its own alternate screen); an unengaged live
    /// tile opens the frozen peek on wheel-up WITHOUT engaging.
    pub(super) fn handle_wheel(&mut self, col: u16, row: u16, up: bool) {
        if self.store.state().overlay.is_some() {
            return;
        }
        let Some((id, rect)) = self.tile_at(col, row) else {
            return;
        };
        // Tile-local coords inside the border; border wheel does nothing
        // (the Dart WheelRegion wrapped only the terminal child).
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        if !inner.contains((col, row).into()) {
            return;
        }
        if self.registry.frozen(&id).is_some() {
            self.scroll_tile(&id, if up { WHEEL_LINES } else { -WHEEL_LINES });
            return;
        }
        let live = self
            .store
            .state()
            .session_by_id(Some(&id))
            .is_some_and(WallSession::live);
        let engaged_here = self.store.state().layer == KeyLayer::Engaged
            && self.store.state().focused_session_id.as_deref() == Some(id.as_str());
        if engaged_here && live {
            let seq = format!(
                "\x1b[<{};{};{}M",
                if up { 64 } else { 65 },
                col - inner.x + 1,
                row - inner.y + 1
            );
            self.registry.write(&id, seq.as_bytes());
            return;
        }
        if live && up {
            self.scroll_tile(&id, WHEEL_LINES); // frozen peek, layer untouched
        }
    }
}

/// Hand `url` to the OS opener: `open` (macOS), `xdg-open` (Linux and other
/// Unix), `rundll32 url.dll,FileProtocolHandler` (Windows; no shell, so the
/// URL is never parsed as a command line).
fn open_url(url: &str) -> std::io::Result<std::process::Child> {
    let argv = opener_argv(url);
    std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .spawn()
}

fn opener_argv(url: &str) -> Vec<String> {
    let v: &[&str] = if cfg!(target_os = "macos") {
        &["open"]
    } else if cfg!(windows) {
        &["rundll32", "url.dll,FileProtocolHandler"]
    } else {
        &["xdg-open"]
    };
    v.iter()
        .map(|s| s.to_string())
        .chain([url.to_string()])
        .collect()
}

#[cfg(test)]
mod opener_tests {
    use super::opener_argv;

    #[test]
    fn the_url_is_the_last_single_argument() {
        let argv = opener_argv("https://example.com/a b&c");
        assert_eq!(argv.last().unwrap(), "https://example.com/a b&c");
        assert!(argv.len() >= 2);
    }
}
