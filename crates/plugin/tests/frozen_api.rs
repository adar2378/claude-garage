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
        "event(s) in /api/plugin/features",
        &names(&g["events"]),
        &feature_list("events"),
    );
    assert_kept(
        "permission(s)",
        &names(&g["permissions"]),
        &set(features::PERMISSIONS.iter().copied()),
    );
    assert_kept(
        "permission(s) in /api/plugin/features",
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
    }
    // the endpoint publishes the same table
    let published = features::features()["routes"].as_array().unwrap().len();
    assert_eq!(published, ROUTES_V1.len());
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
    for prefix in names(&g["core_actions_invocable_prefixes"]) {
        let id = format!("{prefix}x");
        assert!(
            garage_plugin::core_actions::lookup(&id).is_some_and(|a| a.plugin_invocable),
            "the invocable family {prefix}* was removed"
        );
    }
    assert_kept(
        "core action(s) in /api/plugin/features",
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
