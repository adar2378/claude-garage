//! Frame drawing: the wall layout, tiles and overlays (moved out of
//! `runtime/mod.rs`, p22 task 1.6).

use super::*;

impl App {
    pub(super) fn draw_frame(&mut self, frame: &mut ratatui::Frame, now_ms: i64) {
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

        let buf = frame.buffer_mut();
        render_rail(buf, layout.rail, self.store.state(), now_ms);

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
                self.draw_tile(buf, layout.tiles[i], id, now_ms);
            }
        }

        let strip_cols = render_strip(
            buf,
            layout.strip,
            self.store.state(),
            self.router.notices.current_of(NoticeKind::User, now_ms),
            self.pet_render.clone().map(|mut p| {
                p.say = self
                    .router
                    .notices
                    .current_of(NoticeKind::Pet, now_ms)
                    .map(str::to_owned);
                p
            }),
        );
        self.badge_cols = strip_cols.badge;
        self.pet_cols = strip_cols.pet;
        self.last_strip_filler = strip_cols.filler_width;

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

    pub(super) fn draw_tile(&self, buf: &mut ratatui::buffer::Buffer, rect: Rect, id: &str, now_ms: i64) {
        let state = self.store.state();
        let Some(session) = state.session_by_id(Some(id)) else {
            return; // refetch race; the next frame fixes it
        };
        let focused = state.focused_session_id.as_deref() == Some(id);
        let frozen = self.registry.frozen(id);
        let view = TileView {
            session,
            focused,
            engaged: focused && state.layer == KeyLayer::Engaged,
            restoring: self.router.restoring().contains(id),
            dead_reason: self.registry.dead_reason(id),
            frozen_new_lines: frozen.map(|f| f.new_lines),
            now_ms,
        };
        if let Some(frozen) = frozen {
            render_tile(buf, rect, &view, Some(frozen.parser.screen()));
        } else if let Some(client) = self.registry.client(id) {
            let parser = client.parser().lock().unwrap();
            render_tile(buf, rect, &view, Some(parser.screen()));
        } else {
            render_tile(buf, rect, &view, None);
        }
    }
}
