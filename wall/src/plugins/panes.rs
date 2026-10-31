//! The plugin pane overlay (p24 design section 6): a plugin's process runs as
//! a backend session and the wall attaches to it with the same PTY code as an
//! agent tile. While the overlay is open every key goes to the pane except the
//! reserved close chord. `tile` placement is shown as the overlay too.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use garage_protocol::plugins::PaneInstance;
use ratatui::layout::Rect;

use crate::ui::registry::TileRegistry;

/// Ctrl+Q twice within this closes the pane. Both presses are the wall's: the
/// program never sees Ctrl+Q.
pub const CLOSE_CHORD_WINDOW: Duration = Duration::from_millis(1500);

/// Longest pane title shown, in characters.
pub const TITLE_MAX: usize = 40;

/// What the pane's border says: the plugin id first, always, then the title.
/// Both are the plugin's words, so control and invisible characters are
/// dropped and the title is cut, which keeps a plugin from dressing its pane
/// up as something else.
pub fn label(plugin: &str, title: &str) -> String {
    let plugin = garage_protocol::widgets::sanitise(plugin);
    let title = garage_protocol::widgets::sanitise(title);
    let title = if title.chars().count() > TITLE_MAX {
        format!("{}…", title.chars().take(TITLE_MAX - 1).collect::<String>())
    } else {
        title
    };
    format!(" {plugin}: {title} ")
}

/// Default `popup` size when the manifest gives none.
const POPUP_DEFAULT: (u16, u16) = (60, 16);

#[derive(Default)]
pub struct PluginPaneState {
    instance: Option<PaneInstance>,
    first_close_press_ms: Option<i64>,
    /// The overlay's rect in the last frame (clicks outside it close it).
    pub rect: Option<Rect>,
}

impl PluginPaneState {
    pub fn is_open(&self) -> bool {
        self.instance.is_some()
    }

    pub fn instance(&self) -> Option<&PaneInstance> {
        self.instance.as_ref()
    }

    pub fn session(&self) -> Option<&str> {
        self.instance.as_ref().map(|i| i.session.as_str())
    }

    pub fn plugin(&self) -> Option<&str> {
        self.instance.as_ref().map(|i| i.plugin.as_str())
    }

    /// Show `pane`: pin its session in the registry so the tile code attaches
    /// it. Replaces any pane already open.
    pub fn open(&mut self, pane: PaneInstance, registry: &mut TileRegistry) {
        self.close(registry);
        registry.pin(&pane.session);
        self.instance = Some(pane);
        self.first_close_press_ms = None;
    }

    pub fn close(&mut self, registry: &mut TileRegistry) {
        if let Some(old) = self.instance.take() {
            registry.unpin(&old.session);
        }
        self.first_close_press_ms = None;
        self.rect = None;
    }

    /// Feed a key; true when it completes the close chord.
    pub fn close_chord(&mut self, key: &KeyEvent, now_ms: i64) -> bool {
        let is_chord =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q');
        if !is_chord {
            self.first_close_press_ms = None;
            return false;
        }
        match self.first_close_press_ms {
            Some(first) if now_ms - first <= CLOSE_CHORD_WINDOW.as_millis() as i64 => {
                self.first_close_press_ms = None;
                true
            }
            _ => {
                self.first_close_press_ms = Some(now_ms);
                false
            }
        }
    }

    /// True when the first Ctrl+Q is waiting for its second (the overlay frame says so).
    pub fn close_pending(&self, now_ms: i64) -> bool {
        self.first_close_press_ms
            .is_some_and(|f| now_ms - f <= CLOSE_CHORD_WINDOW.as_millis() as i64)
    }
}

/// Where the pane sits on a screen of `area`: `overlay` and `tile` take the
/// screen less a two-column and one-row margin; `popup` is a centred box of
/// the manifest's size (default 60x16), `pct` meaning percent of the screen.
pub fn pane_rect(area: Rect, pane: &PaneInstance) -> Rect {
    let fit = |want: u16, max: u16| want.clamp(4, max.max(4));
    match pane.placement.as_str() {
        "popup" => {
            let (mut cols, mut rows) = (
                pane.cols.unwrap_or(POPUP_DEFAULT.0),
                pane.rows.unwrap_or(POPUP_DEFAULT.1),
            );
            if pane.pct {
                cols = (u32::from(area.width) * u32::from(cols.min(100)) / 100) as u16;
                rows = (u32::from(area.height) * u32::from(rows.min(100)) / 100) as u16;
            }
            let (w, h) = (fit(cols, area.width), fit(rows, area.height));
            Rect::new(
                area.x + (area.width.saturating_sub(w)) / 2,
                area.y + (area.height.saturating_sub(h)) / 2,
                w.min(area.width),
                h.min(area.height),
            )
        }
        _ => {
            let (mx, my) = (2.min(area.width / 4), 1.min(area.height / 4));
            Rect::new(
                area.x + mx,
                area.y + my,
                area.width.saturating_sub(2 * mx),
                area.height.saturating_sub(2 * my),
            )
        }
    }
}

/// The PTY size inside the bordered pane.
pub fn inner(rect: Rect) -> (u16, u16) {
    (
        rect.width.saturating_sub(2).max(1),
        rect.height.saturating_sub(2).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_protocol::AttachSpec;

    fn pane(placement: &str, size: Option<(u16, u16, bool)>) -> PaneInstance {
        PaneInstance {
            plugin: "p".into(),
            pane: "d".into(),
            title: "Details".into(),
            placement: placement.into(),
            session: "plugin/p/d".into(),
            attach: AttachSpec::Tmux { socket: None },
            cols: size.map(|s| s.0),
            rows: size.map(|s| s.1),
            pct: size.is_some_and(|s| s.2),
        }
    }

    fn ctrl_q() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)
    }

    #[test]
    fn the_label_names_the_plugin_first_and_cleans_and_cuts_the_title() {
        assert_eq!(label("bt", "Details"), " bt: Details ");
        let nasty = format!("\u{1b}[2J{}\u{202e}x\n", "y".repeat(80));
        let l = label("bt", &nasty);
        assert!(l.starts_with(" bt: "), "{l:?}");
        assert!(
            !l.chars().any(|c| c.is_control() || c == '\u{202e}'),
            "{l:?}"
        );
        assert_eq!(l.chars().count(), " bt: ".len() + TITLE_MAX + 1, "{l:?}");
        assert!(l.trim_end().ends_with('…'));
        // A title cannot pose as another plugin: the real id is always first.
        assert!(label("evil", "core: Secure").starts_with(" evil: "));
    }

    #[test]
    fn ctrl_q_twice_in_the_window_closes_and_anything_else_resets_it() {
        let mut s = PluginPaneState::default();
        assert!(!s.close_chord(&ctrl_q(), 1000));
        assert!(s.close_pending(1000));
        assert!(s.close_chord(&ctrl_q(), 2000), "second press inside 1.5 s");
        assert!(!s.close_chord(&ctrl_q(), 3000), "and it starts over");
        assert!(!s.close_chord(&KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), 3100));
        assert!(
            !s.close_chord(&ctrl_q(), 3200),
            "another key in between cancelled the first press"
        );
        assert!(
            !s.close_chord(&ctrl_q(), 5000),
            "too late is a first press again"
        );
        assert!(
            !s.close_pending(9000),
            "a stale first press is not shown as pending"
        );
    }

    #[test]
    fn an_overlay_nearly_fills_the_screen_and_a_popup_is_centred_in_its_size() {
        let area = Rect::new(0, 0, 100, 30);
        assert_eq!(
            pane_rect(area, &pane("overlay", None)),
            Rect::new(2, 1, 96, 28)
        );
        assert_eq!(
            pane_rect(area, &pane("popup", None)),
            Rect::new(20, 7, 60, 16)
        );
        assert_eq!(
            pane_rect(area, &pane("popup", Some((40, 10, false)))),
            Rect::new(30, 10, 40, 10)
        );
        assert_eq!(
            pane_rect(area, &pane("popup", Some((50, 50, true)))),
            Rect::new(25, 7, 50, 15)
        );
        let tiny = Rect::new(0, 0, 30, 8);
        let r = pane_rect(tiny, &pane("popup", None));
        assert!(
            r.width <= 30 && r.height <= 8,
            "a popup never outgrows the screen: {r:?}"
        );
        assert_eq!(inner(Rect::new(0, 0, 60, 16)), (58, 14));
    }
}
