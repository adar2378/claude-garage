//! Frame drawing: the wall layout, tiles and overlays (moved out of
//! `runtime/mod.rs`, p22 task 1.6).

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Widget as _};

use crate::slots::{SlotClickRegion, SlotCtx, SlotId};
use crate::state::views::ViewSummary;
use crate::state::wall_state::{KeyLayer, OverlayKind};
use crate::ui::help::render_help_with;
use crate::ui::layout::{slot_geometry, tile_inner, wall_layout};
use crate::ui::rail::render_rail_fading;
use crate::ui::strip::render_strip;
use crate::ui::theme::colors;
use crate::ui::tile::{Gutter, TileView, render_centered_lines, render_tile};
use crate::ui::triage::{render_triage, triage_queue_rows};
use crate::ui::view_picker::{picker_entries, render_view_picker};
use crate::ui::view_strip::render_view_strip;
use crate::ui::workspace_add::render_workspace_add;
use crate::ui::worktree_finish::render_worktree_finish;

use super::App;

impl App {
    pub(super) fn draw_frame(&mut self, frame: &mut ratatui::Frame, now_ms: i64) {
        // Animated slot items read this frame's clock; the loop reads back
        // which of them were drawn to decide whether to wake for animation.
        self.slots.begin_frame(now_ms);
        let area = frame.area();
        // Motion (p23): advance the clock, then start the dissolve of any
        // tile whose session just left the daemon's list.
        self.motion.begin_frame(now_ms);
        {
            let state = self.store.state();
            self.motion
                .closed(|id| state.session_by_id(Some(id)).is_some());
        }
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
        let mut slot_clicks = render_rail_fading(
            buf,
            layout.rail,
            self.store.state(),
            &self.slots,
            now_ms,
            self.motion.enabled(),
        );

        if show_view_strip {
            let ranges = render_view_strip(buf, layout.view_strip, &views, &focused_view_name);
            self.view_strip_targets = views
                .iter()
                .zip(ranges)
                .map(|(v, r)| (r, v.name.clone()))
                .collect();
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
                let (attached, blocked, spinning) = self.tile_motion_flags(id);
                self.motion
                    .tile_drawn(id, layout.tiles[i], attached, blocked);
                if spinning {
                    self.motion.spinner_shown();
                }
                slot_clicks.extend(self.draw_tile(buf, layout.tiles[i], id, now_ms, spinning));
            }
        }
        // Effects run over the finished tiles, under the strip and overlays.
        self.motion.apply(buf);
        if self
            .motion
            .strip_spinner_active(self.store.state().daemon_live, now_ms)
        {
            self.motion.spinner_shown();
        }

        let strip_cols = render_strip(
            buf,
            layout.strip,
            self.store.state(),
            &self.slots,
            self.router.notices.current(now_ms),
        );
        self.badge_cols = strip_cols.badge;
        slot_clicks.extend(
            strip_cols
                .slots
                .into_iter()
                .map(|(cols, target)| SlotClickRegion {
                    rect: Rect::new(cols.start, layout.strip.y, cols.end - cols.start, 1),
                    target,
                    slot: SlotId::StripRight,
                    session: None,
                    workspace: None,
                }),
        );
        self.slot_clicks = slot_clicks;
        // What each slot had room for this frame: providers read it (via
        // `SlotCtx::geometry`) the next time they are asked.
        self.slots
            .set_geometry(slot_geometry(&layout, &strip_cols.filler));

        self.triage_rect = None;
        self.ws_add_rect = None;
        self.view_picker_rect = None;
        match self.store.state().overlay {
            Some(OverlayKind::Help) => render_help_with(buf, area, &self.plugin_help_rows()),
            Some(OverlayKind::TriageQueue) => {
                let rows = triage_queue_rows(&self.store.state().sessions);
                self.triage_rect = Some(render_triage(
                    buf,
                    area,
                    &rows,
                    self.queue_selection,
                    now_ms,
                ));
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
            Some(OverlayKind::PluginPane) => self.draw_plugin_pane(buf, area, now_ms),
            None => {}
        }
        self.layout = Some(layout);
        self.report_slot_geometry();
        // The strip (and its notice) is painted: a pending plugin arm's
        // prompt has now been on screen.
        self.router.mark_prompt_drawn(now_ms);
    }

    /// The help overlay's "Plugins" rows: per plugin its actions with the
    /// key each one really has, then the keys that did not take effect.
    fn plugin_help_rows(&self) -> Vec<(String, String)> {
        let store = self.plugins.borrow();
        let mut rows = Vec::new();
        for p in store
            .list()
            .plugins
            .iter()
            .filter(|p| p.active && !p.actions.is_empty())
        {
            rows.push((p.name.clone(), String::new()));
            for a in &p.actions {
                let key = crate::actions::ActionId::lookup(&a.id)
                    .and_then(|id| self.keymap.key_for(id))
                    .unwrap_or_default();
                rows.push((key, a.title.clone()));
            }
            for c in self
                .plugin_key_conflicts
                .iter()
                .filter(|c| c.plugin == p.id)
            {
                rows.push((
                    format!("! {}", c.key),
                    format!("{} ({})", c.action.as_str(), c.reason),
                ));
            }
        }
        rows
    }

    /// Tell the daemon how much room each slot has, when it changed (and a
    /// plugin is active to care): run contexts carry it as `ui.slots`.
    fn report_slot_geometry(&mut self) {
        if !self.plugins.borrow().has_active() || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let geometry = self.slots.geometry();
        let Some(body) = self.geometry.due(&geometry, std::time::Instant::now()) else {
            return;
        };
        let base_url = self.base_url.clone();
        let tx = self.tx.clone();
        drop(tokio::task::spawn_blocking(move || {
            let reported = crate::api::client::GarageClient::new(Some(base_url))
                .put_ui_slots(&body)
                .map_err(|e| e.to_string());
            let _ = tx.send(super::AppEvent::SlotsReported(reported));
        }));
    }

    /// The plugin pane overlay: a bordered PTY, sized to the screen (or the
    /// popup's own size), with the close chord named in its frame.
    fn draw_plugin_pane(&mut self, buf: &mut ratatui::buffer::Buffer, area: Rect, now_ms: i64) {
        use tui_term::widget::{Cursor, PseudoTerminal};
        let Some(pane) = self.pane.instance().cloned() else {
            return;
        };
        let rect = crate::plugins::panes::pane_rect(area, &pane);
        self.pane.rect = Some(rect);
        let (cols, rows) = crate::plugins::panes::inner(rect);
        self.registry.apply_size(&pane.session, cols, rows);
        ratatui::widgets::Clear.render(rect, buf);
        let hint = if self.pane.close_pending(now_ms) {
            " ctrl+q again to close "
        } else {
            " ctrl+q ctrl+q closes "
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(colors::PLUGIN))
            .title(crate::plugins::panes::label(&pane.plugin, &pane.title))
            .title_bottom(Line::styled(hint, Style::default().fg(colors::FAINT)));
        if let Some(client) = self.registry.client(&pane.session) {
            let parser = client.parser().lock().unwrap();
            let cursor = Cursor::default()
                .symbol(" ")
                .style(Style::default().add_modifier(ratatui::style::Modifier::REVERSED));
            PseudoTerminal::new(parser.screen())
                .block(block)
                .cursor(cursor)
                .render(rect, buf);
        } else {
            let inner = block.inner(rect);
            block.render(rect, buf);
            let text = match self.registry.dead_reason(&pane.session) {
                Some(reason) => reason.to_owned(),
                None => "… attaching".to_owned(),
            };
            render_centered_lines(
                buf,
                inner,
                vec![Line::styled(text, Style::default().fg(colors::DIM))],
            );
        }
    }

    /// `(attached, needs-input, spinner showing)` for one tile — what the
    /// motion layer keys its edges and its redraw clock on.
    fn tile_motion_flags(&self, id: &str) -> (bool, bool, bool) {
        let state = self.store.state();
        let Some(session) = state.session_by_id(Some(id)) else {
            return (false, false, false);
        };
        let has_screen = self.registry.frozen(id).is_some() || self.registry.client(id).is_some();
        let dead = self.registry.dead_reason(id).is_some();
        let attached = session.live() && !dead && has_screen;
        let attaching = session.live() && !dead && !has_screen;
        let restoring = !session.live() && self.router.restoring().contains(id);
        (attached, session.needs_input(), attaching || restoring)
    }

    pub(super) fn draw_tile(
        &mut self,
        buf: &mut ratatui::buffer::Buffer,
        rect: Rect,
        id: &str,
        now_ms: i64,
        spinning: bool,
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
        let mut gutter_items = self.slots.items(
            SlotId::TileGutter,
            &SlotCtx {
                session: Some(session),
                ..SlotCtx::default()
            },
        );
        if !gutter_items.is_empty() {
            self.gutter_sessions.insert(id.to_owned());
        }
        // A marker is anchored to the scrollback row the daemon measured when
        // the turn ended; one that fell out of the kept history is dropped.
        if let (Some(history), true) = (
            self.registry.history(id),
            gutter_items.iter().any(|i| i.gutter_row.is_some()),
        ) {
            let limit = self.registry.history_limit(id);
            gutter_items.retain(|i| {
                i.gutter_row
                    .is_none_or(|row| crate::ui::tile::gutter_in_history(row, history, limit))
            });
        }
        let top_abs = frozen
            .map(|f| f.model.top_abs())
            .or_else(|| self.registry.history(id));
        let gutter = match (gutter_items.is_empty(), top_abs) {
            (false, Some(top_abs)) => Some(Gutter {
                items: &gutter_items,
                top_abs,
            }),
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
            fade_done: self.motion.enabled(),
            spinner: if spinning {
                self.motion.tile_spinner(now_ms)
            } else {
                None
            },
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
