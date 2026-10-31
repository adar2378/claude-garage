//! The manifest reference page, generated from the JSON schema so it cannot
//! drift from the types (`docs/plugins/manifest.md` holds the output between
//! two marker comments; a test keeps it fresh).

use serde_json::Value;

/// Opens the generated block in `manifest.md`.
pub const BEGIN: &str =
    "<!-- generated:manifest begin (UPDATE_DOCS=1 cargo test -p garage-plugin) -->";
/// Closes it.
pub const END: &str = "<!-- generated:manifest end -->";

fn def_name(r: &str) -> &str {
    r.rsplit('/').next().unwrap_or(r)
}

/// A short human type for a schema node.
fn type_of(schema: &Value, defs: &Value) -> String {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = def_name(r);
        return match defs.get(name) {
            Some(d) if d.get("enum").is_some() => enum_values(d),
            _ => name.to_owned(),
        };
    }
    if let Some(any) = schema
        .get("anyOf")
        .or_else(|| schema.get("allOf"))
        .and_then(Value::as_array)
    {
        let parts: Vec<String> = any
            .iter()
            .filter(|s| s.get("type").and_then(Value::as_str) != Some("null"))
            .map(|s| type_of(s, defs))
            .collect();
        return parts.join(" or ");
    }
    if schema.get("enum").is_some() {
        return enum_values(schema);
    }
    match schema.get("type") {
        Some(Value::String(t)) => single(t, schema, defs),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .filter(|t| *t != "null")
            .map(|t| single(t, schema, defs))
            .collect::<Vec<_>>()
            .join(" or "),
        _ => "any JSON".to_owned(),
    }
}

fn single(t: &str, schema: &Value, defs: &Value) -> String {
    match t {
        "array" => format!(
            "list of {}",
            schema
                .get("items")
                .map(|i| type_of(i, defs))
                .unwrap_or_else(|| "any JSON".into())
        ),
        "object" => match schema.get("additionalProperties") {
            Some(a) if a.is_object() => format!("table of {}", type_of(a, defs)),
            _ => "table".to_owned(),
        },
        "integer" => "integer".to_owned(),
        other => other.to_owned(),
    }
}

fn enum_values(d: &Value) -> String {
    d.get("enum")
        .and_then(Value::as_array)
        .map(|vs| {
            vs.iter()
                .filter_map(Value::as_str)
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .map(|s| format!("one of {s}"))
        .unwrap_or_default()
}

fn cell(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn table(
    props: &serde_json::Map<String, Value>,
    required: &[&str],
    defs: &Value,
    out: &mut String,
) {
    out.push_str("| Key | Type | Required | Default | Meaning |\n|---|---|---|---|---|\n");
    for (name, p) in props {
        let req = if required.contains(&name.as_str()) {
            "yes"
        } else {
            "no"
        };
        let default = match p.get("default") {
            None | Some(Value::Null) => String::new(),
            Some(v) => format!("`{v}`"),
        };
        let desc = p.get("description").and_then(Value::as_str).unwrap_or("");
        out.push_str(&format!(
            "| `{name}` | {} | {req} | {} | {} |\n",
            cell(&type_of(p, defs)),
            cell(&default),
            cell(desc)
        ));
    }
    out.push('\n');
}

fn required_of(schema: &Value) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The generated markdown: the header table, then one table per `[[...]]`
/// section and for `[release]`.
pub fn manifest_reference_md() -> String {
    let schema: Value = serde_json::from_str(&crate::schema::schema_json()).expect("schema json");
    let defs = &schema["definitions"];
    let top = schema["properties"].as_object().expect("properties");
    let mut header = serde_json::Map::new();
    let mut sections: Vec<(String, String, &Value)> = Vec::new();
    for (name, p) in top {
        let target = p
            .pointer("/items/$ref")
            .or_else(|| p.pointer("/anyOf/0/$ref"))
            .or_else(|| p.pointer("/allOf/0/$ref"))
            .and_then(Value::as_str)
            .map(def_name)
            .filter(|n| defs[*n].get("properties").is_some());
        match target {
            Some(def) => {
                let heading = if p.get("items").is_some() {
                    format!("[[{name}]]")
                } else {
                    format!("[{name}]")
                };
                sections.push((heading, name.clone(), &defs[def]));
            }
            None => {
                header.insert(name.clone(), p.clone());
            }
        }
    }
    let mut out = String::new();
    out.push_str("### Header\n\nThese keys sit at the top of the file.\n\n");
    table(&header, &required_of(&schema), defs, &mut out);
    for (heading, _name, def) in sections {
        out.push_str(&format!("### `{heading}`\n\n"));
        let props = def["properties"].as_object().expect("properties");
        table(props, &required_of(def), defs, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/plugins/manifest.md"
    );

    fn splice(page: &str, block: &str) -> String {
        let (b, e) = (
            page.find(BEGIN).expect("begin marker in manifest.md"),
            page.find(END).expect("end marker in manifest.md"),
        );
        format!("{}{}\n\n{}{}", &page[..b], BEGIN, block, &page[e..])
    }

    /// The reference tables in `docs/plugins/manifest.md` are generated.
    /// Refresh with `UPDATE_DOCS=1 cargo test -p garage-plugin`.
    #[test]
    fn the_manifest_page_holds_the_current_reference_tables() {
        let page = std::fs::read_to_string(PAGE).expect("docs/plugins/manifest.md");
        let want = splice(&page, &manifest_reference_md());
        if std::env::var_os("UPDATE_DOCS").is_some() {
            std::fs::write(PAGE, &want).unwrap();
            return;
        }
        assert!(
            page == want,
            "docs/plugins/manifest.md is stale; run `UPDATE_DOCS=1 cargo test -p garage-plugin` and commit it"
        );
    }

    #[test]
    fn the_reference_covers_every_section_and_names_types_plainly() {
        let md = manifest_reference_md();
        for h in [
            "### Header",
            "### `[[actions]]`",
            "### `[[events]]`",
            "### `[[panes]]`",
            "### `[[widgets]]`",
            "### `[[buttons]]`",
            "### `[[cli]]`",
            "### `[[build]]`",
            "### `[release]`",
        ] {
            assert!(md.contains(h), "missing {h}");
        }
        assert!(md.contains("| `api_version` | integer | yes |"), "{md}");
        assert!(
            md.contains("list of one of `macos`, `linux`, `windows`"),
            "{md}"
        );
        assert!(md.contains("| `on` | one of `session.created`"), "{md}");
    }
}
