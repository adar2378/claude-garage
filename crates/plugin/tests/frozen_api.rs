//! The plugin API v1 freeze (p25 task 4.2): `api-v1.golden.json` is a
//! hand-edited copy of what v1 promises. Everything in it must still exist;
//! extra things in the code are fine (v1 only grows). A failure here means a
//! breaking change, which belongs in `api_version = 2`.

use std::collections::BTreeSet;

use garage_plugin::features::{self, API_VERSIONS, ROUTES_V1};
use serde_json::Value;

fn golden() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/api-v1.golden.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden file")).expect("golden json")
}

fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("not an array: {v}"))
        .iter()
        .map(|x| x.as_str().expect("string").to_owned())
        .collect()
}

/// Panics naming everything that v1 promised and the code no longer has.
fn assert_kept(what: &str, promised: &[String], now: &BTreeSet<String>) {
    let gone: Vec<&String> = promised.iter().filter(|p| !now.contains(*p)).collect();
    assert!(
        gone.is_empty(),
        "plugin API v1 {what} removed or renamed: {gone:?}. v1 may only grow; a break needs api_version 2"
    );
}

fn set(items: impl IntoIterator<Item = impl Into<String>>) -> BTreeSet<String> {
    items.into_iter().map(Into::into).collect()
}

fn feature_list(key: &str) -> BTreeSet<String> {
    set(names(&features::features()[key]))
}

#[test]
fn v1_is_still_loadable() {
    assert!(API_VERSIONS.contains(&1));
    assert_eq!(golden()["api_versions"], serde_json::json!([1]));
}

#[test]
fn no_v1_event_permission_slot_or_token_was_removed() {
    let g = golden();
    assert_kept(
        "event(s)",
        &names(&g["events"]),
        &set(features::EVENTS.iter().copied()),
    );
    assert_kept(
        "event(s) in /api/plugins/features",
        &names(&g["events"]),
        &feature_list("events"),
    );
    assert_kept(
        "permission(s)",
        &names(&g["permissions"]),
        &set(features::PERMISSIONS.iter().copied()),
    );
    assert_kept(
        "permission(s) in /api/plugins/features",
        &names(&g["permissions"]),
        &feature_list("permissions"),
    );
    assert_kept(
        "pane placement(s)",
        &names(&g["pane_placements"]),
        &set(features::PANE_PLACEMENTS.iter().copied()),
    );
    assert_kept(
        "theme token(s)",
        &names(&g["theme_tokens"]),
        &feature_list("theme_tokens"),
    );
    let slots = feature_list("slots");
    let promised: Vec<String> = g["slots"].as_object().unwrap().keys().cloned().collect();
    assert_kept("slot(s)", &promised, &slots);
    for (slot, scope) in g["slots"].as_object().unwrap() {
        let now = garage_protocol::widgets::WidgetSlot::ALL
            .iter()
            .find(|s| s.as_str() == slot)
            .map(|s| format!("{:?}", s.scope()).to_lowercase());
        assert_eq!(
            now.as_deref(),
            scope.as_str(),
            "slot {slot} changed its scope (session/workspace/global)"
        );
    }
}

#[test]
fn a_dangerous_permission_never_becomes_safe() {
    for p in names(&golden()["dangerous_permissions"]) {
        assert!(features::is_dangerous(&p), "{p} stopped being dangerous");
    }
}

#[test]
fn the_features_endpoint_lists_every_frozen_feature_key() {
    let features = features::features();
    let keys: Vec<String> = features.as_object().unwrap().keys().cloned().collect();
    assert_kept("feature key(s)", &names(&golden()["features"]), &set(keys));
    assert_eq!(features["api.v1.frozen"], true);
}

#[test]
fn no_v1_route_was_removed_or_lost_its_permission_rule() {
    let promised = golden()["routes"].as_array().unwrap().clone();
    for r in promised {
        let (m, p, n) = (
            r["method"].as_str().unwrap(),
            r["path"].as_str().unwrap(),
            r["needs"].as_str().unwrap(),
        );
        let found = ROUTES_V1.iter().find(|x| x.method == m && x.path == p);
        let found = found.unwrap_or_else(|| panic!("v1 route {m} {p} removed or renamed"));
        assert_eq!(found.needs, n, "{m} {p} changed what it needs");
        // a route promised to take a token must keep taking it
        assert_eq!(
            found.tokenless_deprecated,
            r["tokenless"] == "deprecated",
            "{m} {p} changed its tokenless status"
        );
    }
    // the endpoint publishes the same table
    let published = features::features()["routes"].as_array().unwrap().clone();
    assert_eq!(published.len(), ROUTES_V1.len());
    for r in &published {
        let deprecated = ROUTES_V1
            .iter()
            .any(|x| x.path == r["path"] && x.tokenless_deprecated);
        assert_eq!(r.get("tokenless").is_some(), deprecated, "{r}");
    }
}

#[test]
fn withdrawn_permissions_are_not_part_of_v1() {
    let g = golden();
    for p in features::WITHDRAWN_PERMISSIONS {
        assert!(!features::PERMISSIONS.contains(p), "{p} is back in v1");
        assert!(!names(&g["permissions"]).contains(&(*p).to_owned()), "{p}");
    }
}

#[test]
fn limits_are_split_into_sizes_that_only_grow_and_rates_that_may_tighten() {
    let g = golden();
    let limits = features::features()["limits"].clone();
    let keys: Vec<String> = limits.as_object().unwrap().keys().cloned().collect();
    let sizes = names(&g["limit_sizes"]);
    let rates = names(&g["limit_rates"]);
    assert!(g.get("limit_keys").is_none());
    for k in sizes.iter().chain(&rates) {
        assert!(keys.contains(k), "limit {k} is gone");
    }
    // every published limit is classified, so none is promised by accident
    for k in &keys {
        assert!(
            sizes.contains(k) ^ rates.contains(k),
            "limit {k} is neither a size nor a rate (or both)"
        );
    }
    assert!(rates.iter().all(|k| k.ends_with("_per_sec")));
}

#[test]
fn every_frozen_error_code_still_exists() {
    let g = golden();
    let now = set(garage_protocol::ErrorCode::ALL
        .iter()
        .map(|c| c.as_str().to_owned()));
    assert_kept("error code(s)", &names(&g["error_codes"]), &now);
    assert_eq!(names(&g["error_body"]), ["error", "code", "permission"]);
}

/// The naming rule: names are dot-separated segments of lowercase snake_case.
fn follows_the_naming_rule(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|seg| {
            !seg.is_empty()
                && seg.starts_with(|c: char| c.is_ascii_lowercase())
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && !seg.ends_with('_')
                && !seg.contains("__")
        })
}

#[test]
fn every_name_v1_promises_follows_the_naming_rule() {
    let g = golden();
    let mut bad = Vec::new();
    let mut check = |what: &str, name: &str| {
        if !follows_the_naming_rule(name) {
            bad.push(format!("{what}: {name}"));
        }
    };
    for key in [
        "events",
        "permissions",
        "dangerous_permissions",
        "theme_tokens",
        "pane_placements",
        "features",
        "limit_sizes",
        "limit_rates",
        "error_codes",
        "error_body",
        "widget_item_fields",
        "segment_fields",
        "style_fields",
        "core_actions_invocable",
    ] {
        for n in names(&g[key]) {
            check(key, &n);
        }
    }
    for table in [
        "manifest",
        "event_payload",
        "event_payload_nested",
        "context",
    ] {
        for (k, fields) in g[table].as_object().unwrap() {
            if table != "manifest" && table != "context" {
                check(table, k);
            }
            for f in names(fields) {
                check(table, &f);
            }
        }
    }
    for slot in g["slots"].as_object().unwrap().keys() {
        check("slots", slot);
    }
    assert!(bad.is_empty(), "names that break the rule: {bad:#?}");
    // the rule itself
    assert!(follows_the_naming_rule("session.status_changed"));
    for no in ["needs-input", "Session.turn", "a..b", "a.b_", "1a", ""] {
        assert!(!follows_the_naming_rule(no), "{no}");
    }
}

#[test]
fn every_theme_token_and_the_published_names_follow_the_rule_too() {
    let published = features::features();
    for key in ["events", "permissions", "theme_tokens", "slots"] {
        for n in names(&published[key]) {
            assert!(follows_the_naming_rule(&n), "{key}: {n}");
        }
    }
    for key in published.as_object().unwrap().keys() {
        assert!(follows_the_naming_rule(key), "feature key {key}");
    }
}

#[test]
fn every_core_action_id_a_plugin_could_invoke_still_can() {
    let g = golden();
    for id in names(&g["core_actions_invocable"]) {
        let a = garage_plugin::core_actions::lookup(&id)
            .unwrap_or_else(|| panic!("core action {id} removed from the plugin contract"));
        assert!(
            a.plugin_invocable,
            "core action {id} is no longer invocable by plugins"
        );
    }
    // exact ids only: no prefix family, and `session.engage` stays out
    assert!(g.get("core_actions_invocable_prefixes").is_none());
    assert!(
        !garage_plugin::core_actions::lookup("session.engage").is_some_and(|a| a.plugin_invocable),
        "a plugin may never engage a session"
    );
    assert_kept(
        "core action(s) in /api/plugins/features",
        &names(&g["core_actions_invocable"]),
        &set(features::features()["actions.core_invocable"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())),
    );
}

#[test]
fn every_frozen_manifest_field_is_still_in_the_schema() {
    let schema: Value = serde_json::from_str(&garage_plugin::schema::schema_json()).unwrap();
    let defs = &schema["definitions"];
    let table_type = |t: &str| match t {
        "" => &schema,
        "actions" => &defs["Action"],
        "events" => &defs["EventHook"],
        "panes" => &defs["Pane"],
        "widgets" => &defs["Widget"],
        "buttons" => &defs["Button"],
        "cli" => &defs["Cli"],
        "build" => &defs["Build"],
        "release" => &defs["Release"],
        other => panic!("golden manifest table {other:?} unknown to this test"),
    };
    for (table, fields) in golden()["manifest"].as_object().unwrap() {
        let props = table_type(table)["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("manifest table {table:?} has no properties"));
        let now = set(props.keys().cloned());
        assert_kept(
            &format!("manifest field(s) of [{table}]"),
            &names(fields),
            &now,
        );
    }
}

/// A validator for one named type of the published API schema.
fn validator_for(def: &str) -> jsonschema::Validator {
    let mut schema: Value = serde_json::from_str(&garage_protocol::api::api_schema_json()).unwrap();
    schema["$ref"] = format!("#/definitions/{def}").into();
    jsonschema::validator_for(&schema).expect("schema compiles")
}

#[test]
fn the_features_document_matches_the_published_api_schema() {
    let v = validator_for("FeaturesResponse");
    let body = features::features_response("0.7.0");
    let errors: Vec<String> = v.iter_errors(&body).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "{errors:?}");
}
