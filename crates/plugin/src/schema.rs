//! The published JSON schema (`schemas/garage-plugin.schema.json`), derived
//! from the manifest types so editors can autocomplete manifests.

/// Pretty JSON with a trailing newline, exactly what is committed.
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(crate::Manifest);
    let mut out = serde_json::to_string_pretty(&schema).expect("schema serialises");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const COMMITTED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/garage-plugin.schema.json");

    /// The committed schema must match the types. After changing a manifest
    /// type, regenerate with `UPDATE_SCHEMA=1 cargo test -p garage-plugin`.
    #[test]
    fn the_committed_schema_is_not_stale() {
        let fresh = schema_json();
        if std::env::var_os("UPDATE_SCHEMA").is_some() {
            std::fs::create_dir_all(std::path::Path::new(COMMITTED).parent().unwrap()).unwrap();
            std::fs::write(COMMITTED, &fresh).unwrap();
        }
        let committed = std::fs::read_to_string(COMMITTED).expect(
            "schemas/garage-plugin.schema.json is missing; run `UPDATE_SCHEMA=1 cargo test -p garage-plugin`",
        );
        assert!(
            committed == fresh,
            "schemas/garage-plugin.schema.json is stale; run `UPDATE_SCHEMA=1 cargo test -p garage-plugin` and commit it"
        );
    }

    fn toml_as_json(src: &str) -> Value {
        toml::from_str::<Value>(src).expect("valid toml")
    }

    fn validator() -> jsonschema::Validator {
        let schema: Value = serde_json::from_str(&schema_json()).unwrap();
        jsonschema::validator_for(&schema).expect("schema compiles")
    }

    const CLOSE_BUTTON: &str = r#"
id = "close-button"
name = "Close button"
version = "0.1.0"
api_version = 1
platforms = ["macos", "linux", "windows"]
permissions = ["core.actions"]

[[buttons]]
slot = "tile.title.right"
label = " x "
on_click = "session.close"
confirm = true
priority = 5
"#;

    #[test]
    fn the_schema_accepts_the_close_button_example_and_rejects_bad_manifests() {
        let v = validator();
        assert!(v.is_valid(&toml_as_json(CLOSE_BUTTON)), "close-button must validate");
        for (what, bad) in [
            ("a typo'd key", CLOSE_BUTTON.replace("priority", "prority")),
            ("a bad slot", CLOSE_BUTTON.replace("tile.title.right", "tile.nowhere")),
            ("a bad permission", CLOSE_BUTTON.replace("core.actions", "root")),
            ("a missing id", CLOSE_BUTTON.replace("id = \"close-button\"\n", "")),
            ("a string api_version", CLOSE_BUTTON.replace("api_version = 1", "api_version = \"1\"")),
        ] {
            assert!(!v.is_valid(&toml_as_json(&bad)), "schema should reject {what}");
        }
    }
}
