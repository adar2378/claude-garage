//! Status vocabulary for the wall (port of `tui/lib/ui/theme.dart`) —
//! glyphs, colors, elapsed formatting.
//!
//! The visual contract is the p7 UX mockup's salience ladder (mirrored from
//! `ui/src/lib/status.js` semantics): amber is EXCLUSIVELY needs-input — the
//! only loud state — `done` is green and fades 2 minutes after the
//! transition, `working`/`idle` are deliberately colourless neutrals.
//!
//! Everything here is pure (time is injected) so it unit-tests without a
//! terminal.

use ratatui::style::Color;

/// Glyph set from the mockup: ● needs-input, ◐ working, ✓ done, ○ idle,
/// ⟳ restorable.
pub fn glyph_for(status: &str) -> &'static str {
    match status {
        "needs-input" => "●",
        "working" => "◐",
        "done" => "✓",
        "restorable" => "⟳",
        _ => "○", // idle and anything unknown
    }
}

/// `done` stops being highlighted this long after the transition
/// (spec tui-wall: "Done fades").
pub const DONE_FADE_MS: i64 = 2 * 60 * 1000;

/// True when a `done` session's highlight has expired. Unknown `since` never
/// fades (better to over-highlight than to hide a fresh completion).
pub fn done_faded(since_ms: Option<i64>, now_ms: i64) -> bool {
    since_ms.is_some_and(|since| now_ms - since > DONE_FADE_MS)
}

/// The wall palette. Amber is reserved: nothing but needs-input may use it.
pub mod colors {
    use super::Color;

    /// Needs-input — the only hue that shouts.
    pub const AMBER: Color = Color::Rgb(255, 179, 64);
    /// Fresh `done`.
    pub const GREEN: Color = Color::Green;
    /// Normal foreground.
    pub const FG: Color = Color::White;
    /// Working/idle neutral.
    pub const DIM: Color = Color::Gray;
    /// De-emphasized (idle glyphs, non-gridded rail rows, faded done).
    pub const FAINT: Color = Color::DarkGray;
    /// Validation errors (never amber — amber is reserved for needs-input).
    pub const ERROR: Color = Color::Red;
    /// Context-meter "compact or restart soon" threshold (spec
    /// tui-context-meters "Tile context meter", ≥80%) — the palette's red,
    /// same value as [`ERROR`]; kept as its own name since the two convey
    /// unrelated things. NEVER amber (amber stays exclusive to needs-input).
    pub const CTX_HOT: Color = Color::Red;
}

/// Status → color per the salience ladder. `since_ms`/`now_ms` drive the
/// done-fade.
pub fn status_color(status: &str, since_ms: Option<i64>, now_ms: i64) -> Color {
    match status {
        "needs-input" => colors::AMBER,
        "done" => {
            if done_faded(since_ms, now_ms) {
                colors::FAINT
            } else {
                colors::GREEN
            }
        }
        "working" => colors::DIM,
        _ => colors::FAINT, // idle, restorable
    }
}

/// Elapsed-time formatting, ported from `ui/src/lib/elapsed.js`
/// formatElapsed(): `None` → "—", under an hour → "M:SS", under a day →
/// "Hh MMm", else "Nd". Clamps negative deltas to 0 (clock skew reads as
/// "just now").
pub fn format_elapsed(since_ms: Option<i64>, now_ms: i64) -> String {
    let Some(since) = since_ms else {
        return "—".to_owned();
    };
    let total_seconds = (now_ms - since).max(0) / 1000;
    if total_seconds < 3600 {
        return format!("{}:{:02}", total_seconds / 60, total_seconds % 60);
    }
    if total_seconds < 86400 {
        return format!("{}h {:02}m", total_seconds / 3600, (total_seconds % 3600) / 60);
    }
    format!("{}d", total_seconds / 86400)
}

/// The tile/rail timer: waiting time for needs-input, working time for
/// working — the two states where "how long" matters. `None` for the rest.
pub fn elapsed_for(status: &str, since_ms: Option<i64>, now_ms: i64) -> Option<String> {
    if status != "needs-input" && status != "working" {
        return None;
    }
    Some(format_elapsed(since_ms, now_ms))
}

#[cfg(test)]
mod tests {
    //! Port of `tui/test/theme_test.dart` — glyphs, done-fade, the salience
    //! color ladder, elapsed formatting.
    use super::*;

    #[test]
    fn glyphs_match_the_mockup_set() {
        assert_eq!(glyph_for("needs-input"), "●");
        assert_eq!(glyph_for("working"), "◐");
        assert_eq!(glyph_for("done"), "✓");
        assert_eq!(glyph_for("idle"), "○");
        assert_eq!(glyph_for("restorable"), "⟳");
        assert_eq!(glyph_for("unknown-status"), "○");
    }

    #[test]
    fn done_fades_after_two_minutes_unknown_since_never_fades() {
        assert!(!done_faded(Some(1000), 1000 + DONE_FADE_MS));
        assert!(done_faded(Some(1000), 1000 + DONE_FADE_MS + 1));
        assert!(!done_faded(None, i64::MAX));
    }

    #[test]
    fn ctx_hot_is_the_palette_red_never_amber() {
        assert_eq!(colors::CTX_HOT, Color::Red);
        assert_ne!(colors::CTX_HOT, colors::AMBER);
    }

    #[test]
    fn amber_is_exclusive_to_needs_input() {
        assert_eq!(status_color("needs-input", None, 0), colors::AMBER);
        for status in ["working", "done", "idle", "restorable"] {
            assert_ne!(status_color(status, Some(0), 0), colors::AMBER, "{status}");
        }
    }

    #[test]
    fn done_is_green_until_the_fade_then_faint() {
        assert_eq!(status_color("done", Some(1000), 2000), colors::GREEN);
        assert_eq!(
            status_color("done", Some(1000), 1000 + DONE_FADE_MS + 1),
            colors::FAINT
        );
    }

    #[test]
    fn neutrals_for_the_quiet_states() {
        assert_eq!(status_color("working", None, 0), colors::DIM);
        assert_eq!(status_color("idle", None, 0), colors::FAINT);
        assert_eq!(status_color("restorable", None, 0), colors::FAINT);
    }

    #[test]
    fn format_elapsed_matches_the_web_ui_buckets() {
        assert_eq!(format_elapsed(None, 0), "—");
        assert_eq!(format_elapsed(Some(0), 0), "0:00");
        assert_eq!(format_elapsed(Some(0), 65_000), "1:05");
        assert_eq!(format_elapsed(Some(0), 3_599_000), "59:59");
        assert_eq!(format_elapsed(Some(0), 3_600_000), "1h 00m");
        assert_eq!(format_elapsed(Some(0), 5_400_000), "1h 30m");
        assert_eq!(format_elapsed(Some(0), 86_400_000), "1d");
        assert_eq!(format_elapsed(Some(0), 3 * 86_400_000), "3d");
    }

    #[test]
    fn negative_deltas_clamp_to_just_now() {
        assert_eq!(format_elapsed(Some(10_000), 5_000), "0:00");
    }

    #[test]
    fn elapsed_only_for_needs_input_and_working() {
        assert_eq!(elapsed_for("needs-input", Some(0), 60_000).as_deref(), Some("1:00"));
        assert_eq!(elapsed_for("working", Some(0), 60_000).as_deref(), Some("1:00"));
        assert_eq!(elapsed_for("done", Some(0), 60_000), None);
        assert_eq!(elapsed_for("idle", Some(0), 60_000), None);
        assert_eq!(elapsed_for("restorable", None, 60_000), None);
    }
}
