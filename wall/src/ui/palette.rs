//! The jump palette (`/`): one search box over every session, project and
//! recently closed conversation. Type a few letters of a project, a session
//! name or a conversation title; Enter jumps to it (or reopens a recent one).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};

use crate::ui::layout::{centered_rect, modal_width};
use crate::ui::theme::colors;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteTarget {
    /// Focus and engage this session id.
    Session(String),
    /// Focus this workspace.
    Workspace(String),
    /// Reopen this closed conversation (`claude --resume <id>`).
    Recent { claude_session_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub target: PaletteTarget,
    /// What is matched against and shown, e.g. `proj/auth-refactor`.
    pub label: String,
    /// Dim right-hand text, e.g. the status or "closed".
    pub detail: String,
}

impl PaletteItem {
    fn glyph(&self) -> &'static str {
        match self.target {
            PaletteTarget::Session(_) => "◆",
            PaletteTarget::Workspace(_) => "▣",
            PaletteTarget::Recent { .. } => "↺",
        }
    }
}

/// Subsequence fuzzy score of `query` in `text` (case-insensitive); `None`
/// when some query character is missing. Higher is better: consecutive runs
/// and word-start hits score more, long gaps cost.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut prev: Option<usize> = None;
    for qc in q {
        let found = (ti..t.len()).find(|&i| t[i] == qc)?;
        score += 10;
        if prev.is_some_and(|p| found == p + 1) {
            score += 15; // consecutive
        }
        if found == 0 || matches!(t[found - 1], '/' | '-' | '_' | ' ' | '.') {
            score += 12; // word start
        }
        score -= i32::try_from(found - prev.map_or(0, |p| p + 1)).unwrap_or(0).min(8);
        prev = Some(found);
        ti = found + 1;
    }
    Some(score)
}

/// Indices of `items` matching `query`, best first (stable for ties, so the
/// caller's own order — live sessions before recents — breaks them).
pub fn filter(items: &[PaletteItem], query: &str) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| fuzzy_score(query, &it.label).map(|s| (s, i)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

fn fit(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = chars[..width - 1].iter().collect();
    out.push('…');
    out
}

const MAX_ROWS: usize = 10;

pub fn render_palette(
    buf: &mut Buffer,
    area: Rect,
    items: &[PaletteItem],
    matches: &[usize],
    selected: usize,
    query: &str,
) -> Rect {
    let width = modal_width(area.width, 36, 90);
    let shown = matches.len().clamp(1, MAX_ROWS) as u16;
    let rect = centered_rect(area, width, (shown + 7).min(area.height));
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colors::DIM))
        .title(Line::styled(" jump ", Style::default().fg(colors::FG)));
    let inner = block.inner(rect);
    block.render(rect, buf);
    let padded = Rect {
        x: inner.x + 2.min(inner.width),
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(4),
        height: inner.height.saturating_sub(2),
    };
    let w = usize::from(padded.width);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("/ ", Style::default().fg(colors::AMBER)),
            Span::styled(query.to_owned(), Style::default().fg(colors::FG)),
            Span::styled("▏", Style::default().fg(colors::FG)),
        ]),
        Line::raw(""),
    ];
    if matches.is_empty() {
        lines.push(Line::styled("no match", Style::default().fg(colors::DIM)));
    } else {
        let selected = selected.min(matches.len() - 1);
        // Keep the selection inside the visible window.
        let start = selected.saturating_sub(MAX_ROWS - 1);
        for (row, &idx) in matches.iter().enumerate().skip(start).take(MAX_ROWS) {
            let it = &items[idx];
            let on = row == selected;
            let mut style = Style::default().fg(if on { colors::FG } else { colors::DIM });
            if on {
                style = style.add_modifier(Modifier::BOLD);
            }
            let detail = format!("  {}", it.detail);
            let label_w = w.saturating_sub(4 + detail.chars().count());
            lines.push(Line::from(vec![
                Span::styled(if on { "▸ " } else { "  " }, Style::default().fg(colors::FG)),
                Span::styled(format!("{} ", it.glyph()), Style::default().fg(colors::AMBER)),
                Span::styled(fit(&it.label, label_w), style),
                Span::styled(detail, Style::default().fg(colors::FAINT)),
            ]));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "↑/↓ move · Enter go · Esc close",
        Style::default().fg(colors::FAINT),
    ));
    Paragraph::new(lines).render(padded, buf);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str) -> PaletteItem {
        PaletteItem {
            target: PaletteTarget::Workspace(label.to_owned()),
            label: label.to_owned(),
            detail: String::new(),
        }
    }

    #[test]
    fn empty_query_matches_everything_in_order() {
        let items = vec![item("a/one"), item("b/two")];
        assert_eq!(filter(&items, ""), vec![0, 1]);
    }

    #[test]
    fn subsequence_matches_and_misses() {
        assert!(fuzzy_score("fgw", "figma-to-webflow").is_some());
        assert!(fuzzy_score("xyz", "figma-to-webflow").is_none());
    }

    #[test]
    fn word_start_and_consecutive_hits_rank_higher() {
        let items = vec![item("proj/xauthx"), item("proj/auth-refactor")];
        // "auth" is a clean word-start run in the second label.
        assert_eq!(filter(&items, "auth")[0], 1);
    }

    #[test]
    fn case_is_ignored() {
        assert!(fuzzy_score("FIG", "figma").is_some());
    }
}
