//! The plugin docs (`docs/plugins/`) are checked, so they cannot rot:
//! every full manifest example parses and validates, every widget example
//! passes the real push validation for its slot, and every name the v1
//! freeze promises is mentioned somewhere in the docs.

use std::path::PathBuf;

use serde_json::Value;

fn docs_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/plugins"))
}

fn pages() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(docs_dir())
        .expect("docs/plugins exists")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(e.path()).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

/// `(page, info string, body)` of every fenced code block.
fn fences(page: &str, text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut open: Option<(String, Vec<&str>)> = None;
    for line in text.lines() {
        match (&mut open, line.strip_prefix("```")) {
            (None, Some(info)) => open = Some((info.trim().to_owned(), Vec::new())),
            (Some(_), Some(_)) => {
                let (info, body) = open.take().unwrap();
                out.push((page.to_owned(), info, body.join("\n")));
            }
            (Some((_, body)), None) => body.push(line),
            (None, None) => {}
        }
    }
    out
}

fn all_fences() -> Vec<(String, String, String)> {
    pages()
        .iter()
        .flat_map(|(name, text)| fences(name, text))
        .collect()
}

#[test]
fn every_full_manifest_in_the_docs_is_valid() {
    let mut seen = 0;
    for (page, info, body) in all_fences() {
        if info != "toml" || !body.contains("api_version =") {
            continue;
        }
        seen += 1;
        let manifest = garage_plugin::parse(&body).unwrap_or_else(|p| {
            panic!("{page}: manifest example does not validate: {p:?}\n{body}")
        });
        let host = garage_plugin::check_host(&manifest);
        assert!(host.is_empty(), "{page}: {host:?}");
    }
    assert!(
        seen >= 2,
        "expected the quick start and the reference examples"
    );
}

#[test]
fn every_widget_example_passes_the_push_validation_of_its_slot() {
    let mut seen = 0;
    for (page, info, body) in all_fences() {
        let Some(slot) = info.strip_prefix("json widget:") else {
            continue;
        };
        seen += 1;
        let slot = garage_protocol::widgets::WidgetSlot::ALL
            .iter()
            .copied()
            .find(|s| s.as_str() == slot)
            .unwrap_or_else(|| panic!("{page}: unknown slot {slot:?}"));
        let put: garage_protocol::widgets::WidgetPut = serde_json::from_str(&body)
            .unwrap_or_else(|e| panic!("{page}: widget example is not a WidgetPut: {e}\n{body}"));
        garage_protocol::widgets::validate_put(slot, put)
            .unwrap_or_else(|e| panic!("{page}: widget example is refused: {e}\n{body}"));
    }
    assert!(seen >= 5, "expected examples for several slots, saw {seen}");
}

#[test]
fn every_other_json_example_is_json() {
    for (page, info, body) in all_fences() {
        if info == "json" {
            serde_json::from_str::<Value>(&body)
                .unwrap_or_else(|e| panic!("{page}: invalid JSON example: {e}\n{body}"));
        }
    }
}

/// Everything v1 promises by name appears in the docs.
#[test]
fn the_docs_mention_every_name_the_freeze_promises() {
    let corpus: String = pages()
        .into_iter()
        .map(|(_, t)| t)
        .collect::<Vec<_>>()
        .join("\n");
    let golden: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/api-v1.golden.json"))
            .unwrap(),
    )
    .unwrap();
    let mut missing = Vec::new();
    let mut want = |what: &str, name: &str| {
        if !corpus.contains(name) {
            missing.push(format!("{what} {name}"));
        }
    };
    for key in [
        "events",
        "permissions",
        "theme_tokens",
        "env",
        "env_cli",
        "pane_placements",
        "error_codes",
        "error_body",
    ] {
        for n in golden[key].as_array().unwrap() {
            want(key, n.as_str().unwrap());
        }
    }
    for slot in golden["slots"].as_object().unwrap().keys() {
        want("slot", slot);
    }
    for r in golden["routes"].as_array().unwrap() {
        want(
            "route",
            &format!(
                "{} {}",
                r["method"].as_str().unwrap(),
                r["path"].as_str().unwrap()
            ),
        );
    }
    for table in golden["manifest"].as_object().unwrap().keys() {
        if !table.is_empty() {
            want(
                "manifest table",
                &format!("[[{table}]]").replace("[[release]]", "[release]"),
            );
        }
    }
    for id in golden["core_actions_invocable"].as_array().unwrap() {
        // the ids a user is most likely to meet are spelled out; the rest are
        // published by /api/plugins/features. Dangerous ones must be named.
        let id = id.as_str().unwrap();
        if garage_plugin::core_actions::lookup(id).is_some_and(|a| a.dangerous) {
            want("dangerous core action", id);
        }
    }
    assert!(
        missing.is_empty(),
        "docs/plugins does not mention: {missing:#?}"
    );
}
