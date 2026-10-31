//! The one place that says what this garage supports: API versions, events,
//! slots and optional capabilities. Validation, plugin load and
//! `GET /api/plugin/features` all read it (design section 9).

use garage_protocol::widgets::{WidgetSlot, limits};
use serde_json::{Value, json};

/// Plugin API majors this garage loads.
pub const API_VERSIONS: &[u32] = &[1];

/// Plugin-visible event names (v1). `timer` is a schedule, not a bus event.
pub const EVENTS: &[&str] = &[
    "session.created",
    "session.closed",
    "session.turn",
    "session.forgotten",
    "session.status_changed",
    "worktree.created",
    "workspace.added",
    "workspace.removed",
    "daemon.started",
    "timer",
];

pub const PANE_PLACEMENTS: &[&str] = &["tile", "overlay", "popup"];

/// Permissions a manifest may declare.
pub const PERMISSIONS: &[&str] = &[
    "sessions.read",
    "sessions.input",
    "sessions.manage",
    "workspaces.read",
    "workspaces.write",
    "ui.widgets",
    "ui.panes",
    "ui.invoke",
    "core.actions",
    "ui.navigate",
    "ui.notice",
    "network",
];

/// One HTTP route of plugin API v1: method, path (with `{param}` holes) and
/// what it needs. `needs` is `none`, a permission name, `ui-token` or
/// `permission-or-ui-token`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    pub needs: &'static str,
}

const fn route(method: &'static str, path: &'static str, needs: &'static str) -> Route {
    Route {
        method,
        path,
        needs,
    }
}

/// The HTTP routes a plugin may rely on in API v1 (frozen: see
/// `api-v1.golden.json`). The daemon has more routes; those are not promised.
pub const ROUTES_V1: &[Route] = &[
    route("GET", "/api/health", "none"),
    route("GET", "/api/events", "none"),
    route("GET", "/api/sessions", "none"),
    route("GET", "/api/workspaces", "none"),
    route("GET", "/api/usage", "none"),
    route("GET", "/api/workspaces/{name}/closed", "sessions.read"),
    route("GET", "/api/plugin/features", "none"),
    route("GET", "/api/plugins", "none"),
    route("GET", "/api/plugins/widgets", "none"),
    route("GET", "/api/plugins/panes", "none"),
    route("PUT", "/api/plugins/{id}/widgets/{widget}", "ui.widgets"),
    route("DELETE", "/api/plugins/{id}/widgets/{widget}", "ui.widgets"),
    route("POST", "/api/plugins/{id}/notice", "ui.notice"),
    route("POST", "/api/plugins/ui/action", "ui.invoke"),
    route("POST", "/api/plugins/{id}/panes/{pane}/open", "ui.panes"),
    route("DELETE", "/api/plugins/{id}/panes/{pane}", "ui.panes"),
    route("POST", "/api/sessions/input", "sessions.input"),
];

/// Permissions the install prompt flags.
pub const DANGEROUS_PERMISSIONS: &[&str] = &["sessions.input", "core.actions"];

pub fn is_dangerous(permission: &str) -> bool {
    DANGEROUS_PERMISSIONS.contains(&permission)
}

/// The `features` object of `/api/plugin/features`.
pub fn features() -> Value {
    json!({
        "api.v1.frozen": true,
        "events": EVENTS,
        "permissions": PERMISSIONS,
        "dangerous_permissions": DANGEROUS_PERMISSIONS,
        "theme_tokens": garage_protocol::widgets::THEME_TOKENS,
        "routes": ROUTES_V1
            .iter()
            .map(|r| json!({"method": r.method, "path": r.path, "needs": r.needs}))
            .collect::<Vec<_>>(),
        "slots": WidgetSlot::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "widgets.click_args": true,
        "widgets.filler": true,
        "widgets.animated": true,
        "widgets.frame_x": true,
        "widgets.slot_geometry": true,
        "ui.notice": true,
        "cli.fallback": true,
        "panes.placements": PANE_PLACEMENTS,
        "sessions.input": true,
        "actions.core_invocable": crate::core_actions::invocable_ids(),
        "limits": {
            "widget_items": limits::ITEMS_PER_SCOPE,
            "widget_segments": limits::SEGMENTS_PER_ITEM,
            "widget_cols": limits::COLS_PER_ITEM,
            "widget_body_bytes": limits::BODY_BYTES,
            "widget_puts_per_sec": limits::PUTS_PER_SEC,
            "widget_ttl_secs": limits::TTL_SECS_MAX,
            "widget_frames": limits::FRAMES_PER_ITEM,
            "notice_chars": limits::NOTICE_LEN,
        },
    })
}

/// The full `/api/plugin/features` body.
pub fn features_response(garage_version: &str) -> Value {
    json!({"api_versions": API_VERSIONS, "garage": garage_version, "features": features()})
}

/// Is one `requires` entry satisfied? `name` needs a true or non-empty
/// feature; `name:value` needs `value` in a list-valued feature.
pub fn has_feature(requirement: &str) -> bool {
    let features = features();
    if let Some(v) = features.get(requirement) {
        return match v {
            Value::Bool(b) => *b,
            Value::Array(a) => !a.is_empty(),
            Value::Null => false,
            _ => true,
        };
    }
    match requirement.split_once(':') {
        Some((name, value)) => features
            .get(name)
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(value))),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_cover_booleans_lists_and_name_value_pairs() {
        for ok in [
            "widgets.animated",
            "cli.fallback",
            "widgets.slot_geometry",
            "events:session.turn",
            "events:session.forgotten",
            "events:timer",
            "slots:tile.gutter",
            "events",
        ] {
            assert!(has_feature(ok), "{ok}");
        }
        for no in [
            "warp.drive",
            "events:session.exploded",
            "widgets.animated:x",
            "",
            "limits:x",
        ] {
            assert!(!has_feature(no), "{no}");
        }
    }

    #[test]
    fn the_response_matches_the_documented_shape() {
        let r = features_response("0.7.0");
        assert_eq!(r["api_versions"], json!([1]));
        assert_eq!(r["garage"], "0.7.0");
        assert_eq!(r["features"]["limits"]["widget_items"], 16);
        assert!(
            r["features"]["slots"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == "tile.gutter")
        );
        assert!(
            r["features"]["actions.core_invocable"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s == "session.close")
        );
    }
}
