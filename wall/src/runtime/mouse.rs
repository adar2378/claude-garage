//! Mouse handling: clicks resolve to actions through `hit_targets::action_at`,
//! wheel events drive the scrollback peek (moved out of `runtime/mod.rs`,
//! p22 task 1.5).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::input::links::url_at;
use crate::state::wall_state::{KeyLayer, OverlayKind, WallSession};
use crate::ui::hit_targets::{self, action_at, ClickMap};
use crate::ui::scroll::WHEEL_LINES;

use super::router::plugin_gate::ClickConfirm;
use super::{App, now_ms, view_picker_entries};

impl App {
    /// True when the event raised a quit.
    pub(super) fn handle_mouse(&mut self, mouse: &MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                return self.handle_click(mouse.column, mouse.row);
            }
            MouseEventKind::ScrollUp => self.handle_wheel(mouse.column, mouse.row, true),
            MouseEventKind::ScrollDown => self.handle_wheel(mouse.column, mouse.row, false),
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
        {
            if let Some((id, rect)) = self.tile_at(col, row) {
                let inner = Rect {
                    x: rect.x + 1,
                    y: rect.y + 1,
                    width: rect.width.saturating_sub(2),
                    height: rect.height.saturating_sub(2),
                };
                if inner.contains((col, row).into()) {
                    if let Some(line) = self.registry.row_text(&id, row - inner.y) {
                        if let Some(url) = url_at(&line, usize::from(col - inner.x)) {
                            // A click like any other: it cancels a pending
                            // plugin arm before it does its own thing.
                            if self.router.plugin_origin_arm_pending() {
                                self.router.disarm_plugin();
                            }
                            let _ = std::process::Command::new("open").arg(&url).spawn();
                            self.router.notices.show(format!("opened {url}"), 2000, now_ms());
                            return false;
                        }
                    }
                }
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
