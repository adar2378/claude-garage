//! Plugin widget wire types (p24 plugin host, design section 5): what a plugin
//! pushes with `PUT /api/plugins/{id}/widgets/{widget}` and what the TUI later
//! reads back. Daemon and TUI share these types; the daemon also runs
//! [`validate_put`] on every push, so one place owns the limits.
//!
//! Field names are `snake_case` (the plugin-facing API, like the manifest),
//! unlike the camelCase core API.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

/// The limits of design section 5. They are also published by
/// `GET /api/plugin/features` under `limits`.
pub mod limits {
    pub const ITEMS_PER_SCOPE: usize = 16;
    pub const SEGMENTS_PER_ITEM: usize = 16;
    pub const COLS_PER_ITEM: usize = 64;
    pub const CLICK_ARGS_BYTES: usize = 512;
    pub const BODY_BYTES: usize = 16 * 1024;
    pub const SCOPES_PER_WIDGET: usize = 256;
    pub const GUTTER_ITEMS_PER_SCOPE: usize = 64;
    pub const EXTRA_ROWS_PER_WORKSPACE: usize = 8;
    pub const FRAMES_PER_ITEM: usize = 16;
    pub const FRAME_MS_MIN: u32 = 100;
    pub const FRAME_MS_MAX: u32 = 2000;
    pub const FRAME_MS_DEFAULT: u32 = 250;
    pub const PUTS_PER_SEC: u32 = 10;
    pub const PUT_BURST: u32 = 20;
    /// Longest `click.action` string.
    pub const ACTION_LEN: usize = 96;
}

/// Where a widget draws. The scope key of its content depends on the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WidgetSlot {
    #[serde(rename = "tile.title.right")]
    TileTitleRight,
    #[serde(rename = "tile.gutter")]
    TileGutter,
    #[serde(rename = "rail.row")]
    RailRow,
    #[serde(rename = "rail.extra_rows")]
    RailExtraRows,
    #[serde(rename = "rail.header")]
    RailHeader,
    #[serde(rename = "strip.right")]
    StripRight,
}

/// What a scope key names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Session,
    Workspace,
    Global,
}

impl WidgetSlot {
    pub const ALL: [WidgetSlot; 6] = [
        WidgetSlot::TileTitleRight,
        WidgetSlot::TileGutter,
        WidgetSlot::RailRow,
        WidgetSlot::RailExtraRows,
        WidgetSlot::RailHeader,
        WidgetSlot::StripRight,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            WidgetSlot::TileTitleRight => "tile.title.right",
            WidgetSlot::TileGutter => "tile.gutter",
            WidgetSlot::RailRow => "rail.row",
            WidgetSlot::RailExtraRows => "rail.extra_rows",
            WidgetSlot::RailHeader => "rail.header",
            WidgetSlot::StripRight => "strip.right",
        }
    }

    pub fn scope(self) -> ScopeKind {
        match self {
            WidgetSlot::TileTitleRight | WidgetSlot::TileGutter | WidgetSlot::RailRow => ScopeKind::Session,
            WidgetSlot::RailHeader | WidgetSlot::RailExtraRows => ScopeKind::Workspace,
            WidgetSlot::StripRight => ScopeKind::Global,
        }
    }

    fn item_limit(self) -> (usize, &'static str) {
        match self {
            WidgetSlot::TileGutter => (limits::GUTTER_ITEMS_PER_SCOPE, "64 gutter items per scope"),
            WidgetSlot::RailExtraRows => (limits::EXTRA_ROWS_PER_WORKSPACE, "8 extra rows per workspace"),
            _ => (limits::ITEMS_PER_SCOPE, "16 items per scope"),
        }
    }
}

/// The scope key of a slot with no scope (`strip.right`).
pub const GLOBAL_SCOPE: &str = "*";

/// `fg`/`bg`: a theme token or `#rrggbb`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dim: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

pub const THEME_TOKENS: [&str; 6] = ["text", "dim", "accent", "ok", "warn", "error"];

fn color_ok(c: &str) -> bool {
    THEME_TOKENS.contains(&c)
        || (c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit()))
}

/// What a click on a segment (or a whole row) runs. `action` is a core id,
/// `plugin:action`, or a bare local id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Click {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

impl Click {
    /// Core ids are dotted; local ids may not contain a dot (manifest rule).
    pub fn is_core(&self) -> bool {
        is_core_action_id(&self.action)
    }
}

/// Core ids are dotted and have no colon; plugin ids are `plugin:action`;
/// bare local ids contain neither (manifest rule).
pub fn is_core_action_id(id: &str) -> bool {
    id.contains('.') && !id.contains(':')
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click: Option<Click>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Placement {
    Flow,
    /// Strip only: drawn at column `x`, after the flow items.
    Filler {
        x: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
    },
}

impl Default for Placement {
    fn default() -> Self {
        Placement::Flow
    }
}

/// One thing a widget shows. `segments` and `frames` are mutually exclusive;
/// the gutter fields (`row` / `turn` / `kind`) apply to `tile.gutter`, and
/// `row_style` / `click` to `rail.extra_rows` rows (and gutter markers).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WidgetItem {
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub placement: Placement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<Segment>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<Vec<Segment>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_style: Option<Style>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click: Option<Click>,
}

/// Body of `PUT /api/plugins/{id}/widgets/{widget}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WidgetPut {
    pub scopes: BTreeMap<String, Vec<WidgetItem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u32>,
}

impl Placement {
    fn is_filler(&self) -> bool {
        matches!(self, Placement::Filler { .. })
    }
}

/// A rejected push: `status` is 413 for a size or count limit and 422 for a
/// shape problem; `message` names the limit or field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for LimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

fn too_big(message: impl Into<String>) -> LimitError {
    LimitError { status: 413, message: message.into() }
}

fn bad(message: impl Into<String>) -> LimitError {
    LimitError { status: 422, message: message.into() }
}

/// The request body size limit (checked before parsing).
pub fn check_body_len(len: usize) -> Result<(), LimitError> {
    if len > limits::BODY_BYTES {
        return Err(too_big(format!("body is {len} bytes; limit is {} bytes (16 KiB)", limits::BODY_BYTES)));
    }
    Ok(())
}

/// Control characters and newlines are dropped.
pub fn sanitise(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

fn check_style(path: &str, style: &Style) -> Result<(), LimitError> {
    for (name, c) in [("fg", &style.fg), ("bg", &style.bg)] {
        if let Some(c) = c {
            if !color_ok(c) {
                return Err(bad(format!("{path}.{name}: {c:?} is neither a theme token nor #rrggbb")));
            }
        }
    }
    Ok(())
}

fn check_click(path: &str, click: &Click) -> Result<(), LimitError> {
    let a = &click.action;
    if a.is_empty() || a.len() > limits::ACTION_LEN || a.chars().any(char::is_control) {
        return Err(bad(format!("{path}.action: must be 1..={} characters, no control characters", limits::ACTION_LEN)));
    }
    if let Some(args) = &click.args {
        let n = serde_json::to_vec(args).map(|v| v.len()).unwrap_or(usize::MAX);
        if n > limits::CLICK_ARGS_BYTES {
            return Err(too_big(format!("{path}.args: {n} bytes; limit is {} bytes", limits::CLICK_ARGS_BYTES)));
        }
    }
    Ok(())
}

fn clean_segments(path: &str, segs: &mut [Segment]) -> Result<(), LimitError> {
    if segs.len() > limits::SEGMENTS_PER_ITEM {
        return Err(too_big(format!("{path}: {} segments; limit is {} per item", segs.len(), limits::SEGMENTS_PER_ITEM)));
    }
    let mut cols = 0;
    for (i, s) in segs.iter_mut().enumerate() {
        s.text = sanitise(&s.text);
        cols += UnicodeWidthStr::width(s.text.as_str());
        if let Some(st) = &s.style {
            check_style(&format!("{path}[{i}].style"), st)?;
        }
        if let Some(c) = &s.click {
            check_click(&format!("{path}[{i}].click"), c)?;
        }
    }
    if cols > limits::COLS_PER_ITEM {
        return Err(too_big(format!("{path}: {cols} display columns of text; limit is {} per item", limits::COLS_PER_ITEM)));
    }
    Ok(())
}

/// Validate and sanitise a push for a widget in `slot`. Returns the cleaned
/// copy (text sanitised, `frame_ms` clamped) that the store keeps.
pub fn validate_put(slot: WidgetSlot, mut put: WidgetPut) -> Result<WidgetPut, LimitError> {
    if put.scopes.len() > limits::SCOPES_PER_WIDGET {
        return Err(too_big(format!("{} scopes; limit is {} per widget", put.scopes.len(), limits::SCOPES_PER_WIDGET)));
    }
    if let Some(t) = put.ttl_secs {
        if t == 0 {
            return Err(bad("ttl_secs: must be at least 1"));
        }
    }
    let (max_items, limit_name) = slot.item_limit();
    for (scope, items) in put.scopes.iter_mut() {
        if scope.is_empty() {
            return Err(bad("scopes: empty scope key"));
        }
        if slot.scope() == ScopeKind::Global && scope != GLOBAL_SCOPE {
            return Err(bad(format!("scopes[{scope:?}]: {} has no scope; use the key \"*\"", slot.as_str())));
        }
        if items.len() > max_items {
            return Err(too_big(format!("scopes[{scope:?}]: {} items; limit is {limit_name}", items.len())));
        }
        for (i, item) in items.iter_mut().enumerate() {
            let at = format!("scopes[{scope:?}][{i}]");
            if item.placement.is_filler() && slot != WidgetSlot::StripRight {
                return Err(bad(format!("{at}.placement: filler is only valid in strip.right")));
            }
            if let Placement::Filler { caption: Some(c), .. } = &mut item.placement {
                *c = sanitise(c);
            }
            if slot == WidgetSlot::TileGutter && item.row.is_none() && item.turn.is_none() {
                return Err(bad(format!("{at}: a tile.gutter item needs `row` or `turn`")));
            }
            match (&mut item.segments, &mut item.frames) {
                (Some(_), Some(_)) => return Err(bad(format!("{at}: `segments` and `frames` are mutually exclusive"))),
                (Some(segs), None) => clean_segments(&format!("{at}.segments"), segs)?,
                (None, Some(frames)) => {
                    if frames.is_empty() {
                        return Err(bad(format!("{at}.frames: needs at least one frame")));
                    }
                    if frames.len() > limits::FRAMES_PER_ITEM {
                        return Err(too_big(format!(
                            "{at}.frames: {} frames; limit is {}",
                            frames.len(),
                            limits::FRAMES_PER_ITEM
                        )));
                    }
                    for (f, frame) in frames.iter_mut().enumerate() {
                        clean_segments(&format!("{at}.frames[{f}]"), frame)?;
                    }
                    item.frame_ms = Some(
                        item.frame_ms.unwrap_or(limits::FRAME_MS_DEFAULT).clamp(limits::FRAME_MS_MIN, limits::FRAME_MS_MAX),
                    );
                }
                (None, None) if slot == WidgetSlot::TileGutter => {}
                (None, None) => return Err(bad(format!("{at}: needs `segments` or `frames`"))),
            }
            if item.segments.is_some() {
                item.frame_ms = None;
            }
            if let Some(st) = &item.row_style {
                check_style(&format!("{at}.row_style"), st)?;
            }
            if let Some(c) = &item.click {
                check_click(&format!("{at}.click"), c)?;
            }
            if let Some(k) = &mut item.kind {
                *k = sanitise(k);
            }
        }
    }
    Ok(put)
}

/// Every click action in a push, for the daemon's permission check.
pub fn click_actions(put: &WidgetPut) -> Vec<&Click> {
    let mut out = Vec::new();
    for items in put.scopes.values() {
        for item in items {
            out.extend(item.click.iter());
            let frames: Vec<&Vec<Segment>> =
                item.segments.iter().chain(item.frames.iter().flatten()).collect();
            for f in frames {
                out.extend(f.iter().filter_map(|s| s.click.as_ref()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn put(v: Value) -> WidgetPut {
        serde_json::from_value(v).unwrap()
    }

    fn seg(text: &str) -> Value {
        json!({"text": text})
    }

    #[test]
    fn the_design_example_round_trips_and_defaults_to_flow() {
        let v = json!({
            "scopes": {"*": [{"priority": 10, "placement": {"kind": "flow"}, "segments": [
                {"text": "5h ", "style": {"fg": "dim"}},
                {"text": "42%", "style": {"fg": "warn", "bold": true},
                 "click": {"action": "usage-bar:details", "args": {"window": "5h"}}}
            ]}]},
            "ttl_secs": 120
        });
        let p: WidgetPut = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&p).unwrap(), v);
        let bare: WidgetItem = serde_json::from_value(json!({"segments": []})).unwrap();
        assert_eq!(bare.placement, Placement::Flow);
        assert_eq!(bare.priority, 0);
    }

    #[test]
    fn filler_animated_gutter_and_extra_row_forms_round_trip() {
        let filler: WidgetItem =
            serde_json::from_value(json!({"placement": {"kind": "filler", "x": 2, "caption": "hi"}, "segments": []})).unwrap();
        assert_eq!(filler.placement, Placement::Filler { x: 2, caption: Some("hi".into()) });
        let item: WidgetItem = serde_json::from_value(json!({
            "frames": [[{"text": "a"}], [{"text": "b"}]], "frame_ms": 250,
            "row": 120, "kind": "marker", "row_style": {"bg": "#112233"}, "click": {"action": "x:y"}
        }))
        .unwrap();
        assert_eq!(item.frames.as_ref().unwrap().len(), 2);
        assert_eq!(item.row, Some(120));
        let back: WidgetItem = serde_json::from_value(serde_json::to_value(&item).unwrap()).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn text_is_sanitised_and_measured_in_display_columns() {
        let p = validate_put(
            WidgetSlot::StripRight,
            put(json!({"scopes": {"*": [{"segments": [seg("a\nb\u{7}c")]}]}})),
        )
        .unwrap();
        assert_eq!(p.scopes["*"][0].segments.as_ref().unwrap()[0].text, "abc");
        // 33 double-width characters = 66 columns, but only 33 chars.
        let wide = "漢".repeat(33);
        let e = validate_put(WidgetSlot::StripRight, put(json!({"scopes": {"*": [{"segments": [seg(&wide)]}]}}))).unwrap_err();
        assert_eq!(e.status, 413);
        assert!(e.message.contains("64"), "{}", e.message);
        let ok = "漢".repeat(32);
        assert!(validate_put(WidgetSlot::StripRight, put(json!({"scopes": {"*": [{"segments": [seg(&ok)]}]}}))).is_ok());
    }

    #[test]
    fn each_limit_is_rejected_naming_itself() {
        let strip = |items: Value| put(json!({"scopes": {"*": items}}));
        let item = json!({"segments": [seg("x")]});
        let err = |slot, p| validate_put(slot, p).unwrap_err();

        let e = err(WidgetSlot::StripRight, strip(Value::Array(vec![item.clone(); 17])));
        assert_eq!((e.status, e.message.contains("16 items")), (413, true), "{e}");
        let many = json!({"segments": vec![seg("x"); 17]});
        assert!(err(WidgetSlot::StripRight, strip(json!([many]))).message.contains("17 segments"));
        let args = json!({"segments": [{"text": "x", "click": {"action": "a:b", "args": "y".repeat(600)}}]});
        let e = err(WidgetSlot::StripRight, strip(json!([args])));
        assert_eq!(e.status, 413);
        assert!(e.message.contains("512"), "{e}");
        let mut scopes = serde_json::Map::new();
        for i in 0..257 {
            scopes.insert(format!("s{i}"), json!([]));
        }
        let e = err(WidgetSlot::RailRow, put(json!({"scopes": scopes})));
        assert!(e.message.contains("256"), "{e}");
        assert_eq!(check_body_len(16 * 1024), Ok(()));
        assert_eq!(check_body_len(16 * 1024 + 1).unwrap_err().status, 413);
    }

    #[test]
    fn slot_specific_limits_scopes_and_filler_rules() {
        let rows = |n| put(json!({"scopes": {"w": vec![json!({"segments": [seg("r")]}); n]}}));
        assert!(validate_put(WidgetSlot::RailExtraRows, rows(8)).is_ok());
        assert!(validate_put(WidgetSlot::RailExtraRows, rows(9)).unwrap_err().message.contains("8 extra rows"));
        let gutter = |n| put(json!({"scopes": {"s": (0..n).map(|r| json!({"row": r, "kind": "marker"})).collect::<Vec<_>>()}}));
        assert!(validate_put(WidgetSlot::TileGutter, gutter(64)).is_ok());
        assert!(validate_put(WidgetSlot::TileGutter, gutter(65)).unwrap_err().message.contains("64 gutter"));
        let no_row = put(json!({"scopes": {"s": [{"kind": "marker"}]}}));
        assert!(validate_put(WidgetSlot::TileGutter, no_row).unwrap_err().message.contains("row"));
        // strip.right takes only "*"; filler only lives in the strip
        let scoped = put(json!({"scopes": {"garage/a": [{"segments": []}]}}));
        assert!(validate_put(WidgetSlot::StripRight, scoped).unwrap_err().message.contains("\"*\""));
        let filler = put(json!({"scopes": {"s": [{"placement": {"kind": "filler", "x": 3}, "segments": []}]}}));
        assert_eq!(validate_put(WidgetSlot::RailRow, filler).unwrap_err().status, 422);
    }

    #[test]
    fn animated_items_clamp_the_interval_and_exclude_segments() {
        let anim = |ms: Value, n: usize| {
            put(json!({"scopes": {"*": [{"frames": vec![json!([seg("x")]); n], "frame_ms": ms}]}}))
        };
        let p = validate_put(WidgetSlot::StripRight, anim(json!(5), 2)).unwrap();
        assert_eq!(p.scopes["*"][0].frame_ms, Some(100));
        let p = validate_put(WidgetSlot::StripRight, anim(json!(99999), 2)).unwrap();
        assert_eq!(p.scopes["*"][0].frame_ms, Some(2000));
        let p = validate_put(WidgetSlot::StripRight, anim(Value::Null, 2)).unwrap();
        assert_eq!(p.scopes["*"][0].frame_ms, Some(250));
        assert!(validate_put(WidgetSlot::StripRight, anim(json!(300), 17)).unwrap_err().message.contains("16"));
        let both = put(json!({"scopes": {"*": [{"segments": [], "frames": [[]]}]}}));
        assert!(validate_put(WidgetSlot::StripRight, both).unwrap_err().message.contains("mutually exclusive"));
        let neither = put(json!({"scopes": {"*": [{}]}}));
        assert_eq!(validate_put(WidgetSlot::StripRight, neither).unwrap_err().status, 422);
    }

    #[test]
    fn colors_must_be_tokens_or_hex_and_clicks_are_collected_from_every_form() {
        let bad = put(json!({"scopes": {"*": [{"segments": [{"text": "x", "style": {"fg": "red"}}]}]}}));
        assert!(validate_put(WidgetSlot::StripRight, bad).unwrap_err().message.contains("theme token"));
        let good = put(json!({"scopes": {"*": [{"segments": [{"text": "x", "style": {"fg": "#a0B1c2", "bg": "ok"}}]}]}}));
        assert!(validate_put(WidgetSlot::StripRight, good).is_ok());

        let p = put(json!({"scopes": {"w": [
            {"segments": [{"text": "a", "click": {"action": "session.close"}}]},
            {"frames": [[{"text": "b", "click": {"action": "p:q"}}]], "click": {"action": "local"}}
        ]}}));
        let got: Vec<_> = click_actions(&p).into_iter().map(|c| (c.action.as_str(), c.is_core())).collect();
        assert!(got.contains(&("session.close", true)) && got.contains(&("p:q", false)) && got.contains(&("local", false)));
    }
}
