//! Frame drawing: the wall layout, tiles and overlays (moved out of
//! `runtime/mod.rs`, p22 task 1.6).

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Widget as _};

use crate::slots::{SlotClickRegion, SlotCtx, SlotId};
use crate::state::views::ViewSummary;
use crate::state::wall_state::{KeyLayer, OverlayKind};
use crate::ui::help::render_help;
use crate::ui::layout::{slot_geometry, tile_inner, wall_layout};
use crate::ui::rail::render_rail;
use crate::ui::strip::render_strip;
use crate::ui::theme::colors;
use crate::ui::tile::{Gutter, TileView, render_centered_lines, render_tile};
use crate::ui::triage::{render_triage, triage_queue_rows};
use crate::ui::view_picker::{picker_entries, render_view_picker};
use crate::ui::view_strip::render_view_strip;
use crate::ui::workspace_add::render_workspace_add;
use crate::ui::worktree_finish::render_worktree_finish;

use super::{App, NoticeKind};

impl App {
    pub(super) fn draw_frame(&mut self, frame: &mut ratatui::Frame, now_ms: i64) {
        // Animated slot items read this frame's clock; the loop reads back
        // which of them were drawn to decide whether to wake for animation.
        self.slots.begin_frame(now_ms);
        let area = frame.area();
        let (ids, maximized_index) = {
            let state = self.store.state();
            let ids = state.gridded_session_ids.clone();
            let maximized = state
                .maximized_session_id
                .as_deref()
                .and_then(|m| ids.iter().position(|id| id == m));
            (ids, maximized)
        };
        // Views (spec tui-views "View strip and group frame"): the strip
        // shows only at 2+ views; the frame only around a 2+-session
        // focused view — both precomputed here so `wall_layout` stays pure
        // geometry with no state dependency of its own.
        let focused_workspace = self.store.state().focused_workspace.clone();
        let views: Vec<ViewSummary> = focused_workspace
            .as_deref()
            .map(|w| self.store.state().views_for(w))
            .unwrap_or_default();
        let focused_view_name = focused_workspace
            .as_deref()
            .map(|w| self.store.state().focused_view_name(w).to_owned())
            .unwrap_or_default();
        let show_view_strip = views.len() >= 2;
        let framed = views
            .iter()
            .find(|v| v.name == focused_view_name)
            .is_some_and(|v| v.session_ids.len() >= 2);

        let layout = wall_layout(area, ids.len(), maximized_index, show_view_strip, framed);

        // PTY size tracks the tile (TIOCSWINSZ) on every layout pass —
        // startup, grid reshape, maximize both ways, terminal resize.
        for (i, id) in ids.iter().enumerate() {
            let (cols, rows) = tile_inner(layout.tiles[i]);
            self.registry.apply_size(id, cols, rows);
        }

        self.gutter_sessions.clear(); // refilled by the tiles drawn below
        let buf = frame.buffer_mut();
        let mut slot_clicks =
            render_rail(buf, layout.rail, self.store.state(), &self.slots, now_ms);

        if show_view_strip {
            let ranges = render_view_strip(buf, layout.view_strip, &views, &focused_view_name);
            self.view_strip_targets =
                views.iter().zip(ranges).map(|(v, r)| (r, v.name.clone())).collect();
        } else {
            self.view_strip_targets.clear();
        }
        if let Some(frame_rect) = layout.group_frame {
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(colors::FAINT))
                .render(frame_rect, buf);
        }

        if self.store.state().groups.is_empty() {
            // Zero workspaces: onboarding panel (never amber).
            render_centered_lines(
                buf,
                layout.grid,
                vec![
                    Line::styled(
                        "no workspaces yet — press w to add one",
                        Style::default().fg(colors::FG),
                    ),
                    Line::styled(
                        "every Claude session in it appears here live",
                        Style::default().fg(colors::FAINT),
                    ),
                ],
            );
        } else if ids.is_empty() {
            render_centered_lines(
                buf,
                layout.grid,
                vec![
                    Line::styled(
                        "no sessions in this workspace",
                        Style::default().fg(colors::FG),
                    ),
                    Line::styled(
                        "press n for a session, N for a worktree session",
                        Style::default().fg(colors::FAINT),
                    ),
                ],
            );
        } else {
            for (i, id) in ids.iter().enumerate() {
                // A maximized tile covers the whole grid: siblings stay
                // mounted (parsers keep reading, PTYs stay sized) but are
                // not painted and receive no input.
                if maximized_index.is_some() && maximized_index != Some(i) {
                    continue;
                }
                slot_clicks.extend(self.draw_tile(buf, layout.tiles[i], id, now_ms));
            }
        }

        if let Some(pet) = &self.pet {
            pet.publish_frame(&self.router.notices, now_ms);
        }
        let strip_cols = render_strip(
            buf,
            layout.strip,
            self.store.state(),
            &self.slots,
            self.router.notices.current_of(NoticeKind::User, now_ms),
        );
        self.badge_cols = strip_cols.badge;
        slot_clicks.extend(strip_cols.slots.into_iter().map(|(cols, target)| SlotClickRegion {
            rect: Rect::new(cols.start, layout.strip.y, cols.end - cols.start, 1),
            target,
            slot: SlotId::StripRight,
            session: None,
            workspace: None,
        }));
        self.slot_clicks = slot_clicks;
        self.last_strip_filler = strip_cols.filler_width;
        // What each slot had room for this frame: providers read it (via
        // `SlotCtx::geometry`) the next time they are asked.
        self.slots.set_geometry(slot_geometry(&layout, &strip_cols.filler));

        self.triage_rect = None;
        self.ws_add_rect = None;
        self.view_picker_rect = None;
        match self.store.state().overlay {
            Some(OverlayKind::Help) => render_help(buf, area),
            Some(OverlayKind::TriageQueue) => {
                let rows = triage_queue_rows(&self.store.state().sessions);
                self.triage_rect =
                    Some(render_triage(buf, area, &rows, self.queue_selection, now_ms));
            }
            Some(OverlayKind::WorkspaceAdd) => {
                self.ws_add_rect = Some(render_workspace_add(buf, area, &self.ws_form));
            }
            Some(OverlayKind::ViewPicker) => {
                let view_names: Vec<String> = views.iter().map(|v| v.name.clone()).collect();
                let entries = picker_entries(&view_names, &focused_view_name);
                self.view_picker_rect = Some(render_view_picker(
                    buf,
                    area,
                    &entries,
                    self.view_picker.selected,
                    self.view_picker.new_group_input.as_deref(),
                ));
            }
            Some(OverlayKind::WorktreeFinish) => {
                render_worktree_finish(buf, area, &self.wt_finish, now_ms);
            }
            None => {}
        }
        self.layout = Some(layout);
    }

    pub(super) fn draw_tile(
        &mut self,
        buf: &mut ratatui::buffer::Buffer,
        rect: Rect,
        id: &str,
        now_ms: i64,
    ) -> Vec<SlotClickRegion> {
        let state = self.store.state();
        let Some(session) = state.session_by_id(Some(id)) else {
            return Vec::new(); // refetch race; the next frame fixes it
        };
        let focused = state.focused_session_id.as_deref() == Some(id);
        let frozen = self.registry.frozen(id);
        // `tile.gutter` markers and the scrollback row the body's top sits on:
        // a frozen view knows it, a live tail view needs the polled tmux
        // history size (and draws no gutter until the first poll lands).
        let gutter_items = self.slots.items(
            SlotId::TileGutter,
            &SlotCtx { session: Some(session), ..SlotCtx::default() },
        );
        if !gutter_items.is_empty() {
            self.gutter_sessions.insert(id.to_owned());
        }
        let top_abs = frozen
            .map(|f| f.model.top_abs())
            .or_else(|| self.registry.history(id));
        let gutter = match (gutter_items.is_empty(), top_abs) {
            (false, Some(top_abs)) => Some(Gutter { items: &gutter_items, top_abs }),
            _ => None,
        };
        let view = TileView {
            session,
            focused,
            engaged: focused && state.layer == KeyLayer::Engaged,
            restoring: self.router.restoring().contains(id),
            dead_reason: self.registry.dead_reason(id),
            frozen_new_lines: frozen.map(|f| f.new_lines),
            now_ms,
            slots: &self.slots,
            gutter,
        };
        if let Some(frozen) = frozen {
            render_tile(buf, rect, &view, Some(frozen.parser.screen()))
        } else if let Some(client) = self.registry.client(id) {
            let parser = client.parser().lock().unwrap();
            render_tile(buf, rect, &view, Some(parser.screen()))
        } else {
            render_tile(buf, rect, &view, None)
        }
    }
}
