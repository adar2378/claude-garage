//! The `p` plugins overlay: every installed plugin with its source, whether
//! it is on, and the permissions it asked for. `j`/`k`/arrows move, Space
//! turns the selected plugin on or off. Removing is the CLI's job, so the
//! footer names the exact command instead of offering it. The list is read
//! from the plugin store on every frame, so the daemon's `plugins` event
//! refreshes an open overlay with no state of its own to keep in step; this
//! module is pure content + geometry, like `view_picker`.

use garage_protocol::plugins::{PluginList, PluginSummary};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::ui::layout::{centered_rect, modal_width};
use crate::ui::theme::colors;
use crate::ui::triage::fit;
use crate::ui::worktree_finish::wrap_rows;

/// The most rows an inline error takes before it is cut with an ellipsis.
const MAX_ERROR_ROWS: usize = 4;

/// The overlay's own state: the selection, the plugin a toggle is in flight
/// for, and the last toggle's error. Reset on every open.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct PluginsOverlayState {
    pub selected: usize,
    /// `Some(id)` while the daemon is answering that plugin's enable/disable;
    /// Space is ignored until it settles.
    pub busy: Option<String>,
    /// The last toggle's failure, shown in the footer until the next action.
    pub error: Option<String>,
}

impl PluginsOverlayState {
    pub fn reset(&mut self) {
        *self = PluginsOverlayState::default();
    }

    /// Pull the selection back inside a list that got shorter.
    pub fn clamp(&mut self, len: usize) {
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    /// The selected plugin of `list`, if any.
    pub fn current<'a>(&self, list: &'a PluginList) -> Option<&'a PluginSummary> {
        list.plugins
            .get(self.selected.min(list.plugins.len().saturating_sub(1)))
    }
}

/// What the footer says about removing `plugin`: the CLI command, or that a
/// bundled plugin can only be turned off.
pub fn remove_hint(plugin: &PluginSummary) -> String {
    if plugin.source == "bundled" {
        "bundled: on/off only".to_owned()
    } else {
        format!("remove: claude-garage plugin remove {}", plugin.id)
    }
}

/// The command an empty overlay points at.
pub const INSTALL_HINT: &str = "claude-garage plugin install owner/repo";

fn rule(width: usize) -> Line<'static> {
    Line::styled("─".repeat(width), Style::default().fg(colors::FAINT))
}

fn plugin_row(
    plugin: &PluginSummary,
    selected: bool,
    name_w: usize,
    width: usize,
) -> Line<'static> {
    let (glyph, glyph_color) = match (plugin.enabled, plugin.active) {
        (true, true) => ("●", colors::GREEN),
        // On, but not running: not approved, or it failed to load.
        (true, false) => ("◐", colors::AMBER),
        (false, _) => ("○", colors::FAINT),
    };
    let mut text = Style::default().fg(if selected { colors::FG } else { colors::DIM });
    if selected {
        text = text.add_modifier(Modifier::BOLD);
    }
    let source = if plugin.source.is_empty() {
        "-"
    } else {
        plugin.source.as_str()
    };
    let state = if plugin.enabled { "on" } else { "off" };
    let permissions = plugin.permissions.join(", ");
    let fixed = 2 + 2 + name_w + 1 + 8 + 1 + 3 + 2;
    Line::from(vec![
        Span::styled(
            if selected { "▸ " } else { "  " },
            Style::default().fg(colors::FG),
        ),
        Span::styled(format!("{glyph} "), Style::default().fg(glyph_color)),
        Span::styled(format!("{:<name_w$} ", fit(&plugin.id, name_w)), text),
        Span::styled(format!("{source:<8} "), text),
        Span::styled(format!("{state:<3}  "), text),
        Span::styled(
            fit(&permissions, width.saturating_sub(fixed)),
            Style::default().fg(colors::FAINT),
        ),
    ])
}

/// The modal's content lines at `width` cells: the rows, then the footer
/// (key hints with the selected plugin's remove command, the in-flight
/// toggle, the last error). An empty list renders the install hint instead.
pub fn plugins_lines(
    list: &PluginList,
    state: &PluginsOverlayState,
    width: usize,
) -> Vec<Line<'static>> {
    let faint = Style::default().fg(colors::FAINT);
    let mut lines = Vec::new();
    if list.plugins.is_empty() {
        lines.push(Line::styled(
            "No plugins installed",
            Style::default().fg(colors::DIM),
        ));
        lines.push(Line::raw(""));
        for row in wrap_rows(
            &format!("Install one with: {INSTALL_HINT}  (or `claude-garage plugin link <folder>`)"),
            width,
            4,
        ) {
            lines.push(Line::styled(row, faint));
        }
        return lines;
    }
    let selected = state.selected.min(list.plugins.len() - 1);
    let name_w = list
        .plugins
        .iter()
        .map(|p| p.id.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 20);
    let dim = Style::default().fg(colors::FAINT);
    lines.push(Line::styled(
        format!(
            "    {:<name_w$} {:<8} {:<3}  permissions",
            "name", "source", "state"
        ),
        dim,
    ));
    for (i, plugin) in list.plugins.iter().enumerate() {
        lines.push(plugin_row(plugin, i == selected, name_w, width));
    }
    lines.push(rule(width));
    lines.push(Line::styled(fit("j/k move · Space on/off", width), faint));
    // Never split: a command wider than the box is clipped by the edge.
    lines.push(Line::styled(remove_hint(&list.plugins[selected]), faint));
    if let Some(id) = &state.busy {
        lines.push(Line::styled(format!("updating {id}…"), faint));
    }
    if let Some(error) = &state.error {
        for row in wrap_rows(error, width, MAX_ERROR_ROWS) {
            lines.push(Line::styled(row, Style::default().fg(colors::ERROR)));
        }
    }
    lines
}

/// The modal box rect for the content `lines` would have at this `area`.
pub fn plugins_modal_rect(area: Rect, list: &PluginList, state: &PluginsOverlayState) -> Rect {
    let width = modal_width(area.width, 40, 78);
    let content_width = usize::from(width.saturating_sub(6));
    let rows = plugins_lines(list, state, content_width).len() as u16;
    centered_rect(area, width, (rows + 4).min(area.height))
}

/// Render the overlay; returns the modal rect (an outside click closes it).
pub fn render_plugins_overlay(
    buf: &mut Buffer,
    area: Rect,
    list: &PluginList,
    state: &PluginsOverlayState,
) -> Rect {
    let rect = plugins_modal_rect(area, list, state);
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(" Plugins ", Style::default().fg(colors::FG)))
        .title(
            Line::styled(" p / Esc to close ", Style::default().fg(colors::FAINT)).right_aligned(),
        );
    let inner = block.inner(rect);
    block.render(rect, buf);
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    Paragraph::new(plugins_lines(list, state, usize::from(padded.width))).render(padded, buf);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn list(v: serde_json::Value) -> PluginList {
        serde_json::from_value(json!({ "plugins": v })).unwrap()
    }

    fn sample() -> PluginList {
        list(json!([
            {"id": "usage-bar", "name": "Usage bar", "enabled": true, "active": true,
             "source": "bundled", "permissions": ["widgets", "notices"]},
            {"id": "pet-extras", "name": "Pet extras", "enabled": false, "active": false,
             "source": "local", "permissions": ["ui.invoke"]},
        ]))
    }

    fn text(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn rows_show_state_source_and_permissions_and_the_footer_follows_the_selection() {
        let mut state = PluginsOverlayState::default();
        let rows = text(&plugins_lines(&sample(), &state, 90));
        assert!(rows[1].starts_with("▸ ● usage-bar"), "{:?}", rows[1]);
        assert!(rows[1].contains("bundled  on"), "{:?}", rows[1]);
        assert!(rows[1].ends_with("widgets, notices"), "{:?}", rows[1]);
        assert!(rows[2].starts_with("  ○ pet-extras"), "{:?}", rows[2]);
        assert!(rows[2].contains("local    off"), "{:?}", rows[2]);
        let n = rows.len();
        assert_eq!(rows[n - 2], "j/k move · Space on/off");
        assert_eq!(
            rows[n - 1],
            "bundled: on/off only",
            "a bundled plugin offers no remove command"
        );
        state.selected = 1;
        assert_eq!(
            text(&plugins_lines(&sample(), &state, 40)).last().unwrap(),
            "remove: claude-garage plugin remove pet-extras",
            "the command stays on one line, however narrow"
        );
    }

    #[test]
    fn an_empty_list_says_so_and_names_the_install_command() {
        let rows = text(&plugins_lines(
            &list(json!([])),
            &PluginsOverlayState::default(),
            70,
        ));
        assert_eq!(rows[0], "No plugins installed");
        assert!(rows.iter().any(|r| r.contains(INSTALL_HINT)));
    }

    #[test]
    fn an_error_and_a_toggle_in_flight_are_shown_under_the_hint() {
        let state = PluginsOverlayState {
            selected: 0,
            busy: Some("usage-bar".into()),
            error: Some("plugin usage-bar failed to load".into()),
        };
        let rows = text(&plugins_lines(&sample(), &state, 90));
        let n = rows.len();
        assert_eq!(rows[n - 2], "updating usage-bar…");
        assert_eq!(rows[n - 1], "plugin usage-bar failed to load");
    }

    #[test]
    fn a_stale_selection_clamps_to_the_last_plugin() {
        let mut state = PluginsOverlayState {
            selected: 9,
            ..Default::default()
        };
        assert_eq!(state.current(&sample()).unwrap().id, "pet-extras");
        state.clamp(1);
        assert_eq!(state.selected, 0);
        state.clamp(0);
        assert_eq!(state.selected, 0);
        assert!(state.current(&list(json!([]))).is_none());
    }
}
