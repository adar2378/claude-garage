//! `wall.json` — the client-side persistence file for view assignments
//! (spec tui-views "View persistence"). Disposable by design: a missing or
//! invalid file collapses every workspace to its default view with no error;
//! nothing here is authoritative — the daemon never sees this file and its
//! absence is never a fault. (It used to hold the pit-pet choice too; the
//! pet is a plugin now and reads the old `pet` key once, see `plugins/pit-pet`.)
//!
//! Schema: `{"version": 1, "views": {"<workspace>": [{"name", "sessions":
//! [id, ...]}, ...]}}`. Only *named* (non-default)
//! views are written — the default view's membership is always the derived
//! complement (see `state::views` module docs), so an untouched workspace
//! costs nothing in the file, and a workspace with no named views at all is
//! omitted rather than written as `[]`. Unknown top-level keys (an old `pet`,
//! which the pit-pet plugin reads once) are ignored on read and kept as they
//! are when the file is saved, so this build never deletes what it does not own.
//!
//! **Ownership split** (this module vs. the runtime wave): everything here
//! is either pure (`serialize`/`deserialize`) or a small, self-contained
//! filesystem primitive (`load`/`save`/`wall_json_path`) with no state to
//! hide behind — `WallStore` itself stays IO-free, matching every other
//! store transition in this crate. What this module does NOT decide is
//! *when* to save: that's the runtime's debounce timer, driven by
//! `WallStore::views_revision()` (see store.rs docs) — call `save` with
//! `store.state().views` once the timer fires, off the render path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::state::views::{DEFAULT_VIEW, View, ViewsByWorkspace};
use crate::state::wall_state::RailPrefs;

pub const SCHEMA_VERSION: u64 = 1;

/// The full `wall.json` document: view assignments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WallFile {
    pub views: ViewsByWorkspace,
    /// The rail's hide/width choice. Written as `"rail": {"hidden", "width"}`
    /// only when it differs from the default; a missing or malformed value
    /// loads as the default.
    pub rail: RailPrefs,
}

/// `$GARAGE_DIR`, else `~/.garage` (shared by `wall.json` and `config.toml`).
pub fn garage_dir() -> PathBuf {
    garage_dir_from(
        std::env::var("GARAGE_DIR").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// `$GARAGE_DIR/wall.json`, else `~/.garage/wall.json` — mirrors the
/// daemon's `registry.js` GARAGE_DIR convention (same override, same
/// fallback), so a scratch `GARAGE_DIR` used for daemon testing also
/// isolates the wall's own state file.
pub fn wall_json_path() -> PathBuf {
    garage_dir_from_env().join("wall.json")
}

/// `$GARAGE_DIR`, else `~/.garage` (`%USERPROFILE%` when `HOME` is unset).
pub fn garage_dir_from_env() -> PathBuf {
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok());
    garage_dir_from(std::env::var("GARAGE_DIR").ok().as_deref(), home.as_deref())
}

/// Pure path-resolution logic, factored out of [`wall_json_path`] so it's
/// testable without mutating process-global env vars (which `cargo test`
/// runs concurrently across threads).
fn garage_dir_from(garage_dir_env: Option<&str>, home_env: Option<&str>) -> PathBuf {
    match garage_dir_env {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => Path::new(home_env.unwrap_or("")).join(".garage"),
    }
}

/// Render `file` as the `wall.json` document text.
pub fn serialize(file: &WallFile) -> String {
    serialize_keeping(file, serde_json::Map::new())
}

/// [`serialize`] with the unknown top-level keys of an earlier document
/// carried over untouched.
fn serialize_keeping(file: &WallFile, kept: serde_json::Map<String, Value>) -> String {
    let mut by_workspace = serde_json::Map::new();
    for (workspace, list) in &file.views {
        if list.is_empty() {
            continue;
        }
        let entries: Vec<Value> = list
            .iter()
            .map(|v| json!({ "name": v.name, "sessions": v.session_ids }))
            .collect();
        by_workspace.insert(workspace.clone(), Value::Array(entries));
    }
    let mut doc = kept;
    doc.insert("version".to_owned(), json!(SCHEMA_VERSION));
    doc.insert("views".to_owned(), Value::Object(by_workspace));
    let doc = Value::Object(doc);
    // Written only when it differs from the default; a stale kept value
    // must not outlive a reset.
    let mut doc = doc;
    if let Value::Object(map) = &mut doc {
        map.remove("rail");
        if file.rail != RailPrefs::default() {
            map.insert(
                "rail".to_owned(),
                json!({
                    "hidden": file.rail.hidden,
                    "width": RailPrefs::clamp_width(file.rail.width),
                }),
            );
        }
    }
    // A `Value` built from owned strings/numbers always serializes; the
    // fallback is unreachable in practice but keeps this function total.
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_owned())
}

/// Parse `raw`; anything that isn't exactly the expected shape — invalid
/// JSON, an unrecognized `version`, a malformed entry — yields the default
/// (empty views). Never errors: spec "View persistence" — "if
/// missing or invalid, all sessions collapse into the default view with no
/// error". A malformed *individual* view entry is skipped rather than
/// invalidating the whole file, so one bad line can't cost every other
/// workspace its view assignments.
pub fn deserialize(raw: &str) -> WallFile {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return WallFile::default();
    };
    let Some(root) = value.as_object() else {
        return WallFile::default();
    };
    if root.get("version").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return WallFile::default();
    }
    let Some(by_workspace) = root.get("views").and_then(Value::as_object) else {
        return WallFile::default();
    };

    let mut out = HashMap::new();
    for (workspace, entries) in by_workspace {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        let mut list = Vec::new();
        for entry in entries {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(sessions) = entry.get("sessions").and_then(Value::as_array) else {
                continue;
            };
            if name == DEFAULT_VIEW {
                continue; // the default is never a stored view — see module docs.
            }
            let session_ids: Vec<String> = sessions
                .iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect();
            if session_ids.is_empty() {
                continue;
            }
            list.push(View {
                name: name.to_owned(),
                session_ids,
            });
        }
        if !list.is_empty() {
            out.insert(workspace.clone(), list);
        }
    }

    // Lenient: a bad `rail` costs only the rail.
    let rail = root
        .get("rail")
        .and_then(Value::as_object)
        .map(|r| RailPrefs {
            hidden: r.get("hidden").and_then(Value::as_bool).unwrap_or(false),
            width: r
                .get("width")
                .and_then(Value::as_u64)
                .map_or(RailPrefs::DEFAULT_WIDTH, |w| {
                    RailPrefs::clamp_width(u16::try_from(w).unwrap_or(u16::MAX))
                }),
        })
        .unwrap_or_default();

    WallFile { views: out, rail }
}

/// Load `wall.json` from `path`. Missing file, unreadable, or invalid
/// content all collapse to [`WallFile::default`] — see [`deserialize`].
/// Does not prune `views` against the live session set; the caller
/// (`WallStore::load_views`) does that once it knows one, via
/// `state::views::prune_views`, the same path a later refetch uses.
pub fn load(path: &Path) -> WallFile {
    match std::fs::read_to_string(path) {
        Ok(raw) => deserialize(&raw),
        Err(_) => WallFile::default(),
    }
}

/// The top-level keys of the file at `path` that this build does not own
/// (everything but `version` and `views`); empty when it is missing or not a
/// JSON object.
fn unknown_keys(path: &Path) -> serde_json::Map<String, Value> {
    let Some(Value::Object(mut root)) = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
    else {
        return serde_json::Map::new();
    };
    root.remove("version");
    root.remove("views");
    root
}

/// Write `wall.json` atomically: a temp file in the same directory, then
/// `rename()` (atomic on the same filesystem — a crash mid-write leaves
/// either the previous complete file or the new one, never a truncation),
/// matching the daemon's `registry.js` `writeState` convention.
pub fn save(path: &Path, file: &WallFile) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("wall.json");
    let tmp = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let write_result = std::fs::write(&tmp, serialize_keeping(file, unknown_keys(path)));
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn view(name: &str, ids: &[&str]) -> View {
        View {
            name: name.to_owned(),
            session_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn file(views: ViewsByWorkspace) -> WallFile {
        WallFile {
            views,
            rail: RailPrefs::default(),
        }
    }

    // ── path resolution ─────────────────────────────────────────────────

    #[test]
    fn garage_dir_env_wins_when_set_and_non_empty() {
        assert_eq!(
            garage_dir_from(Some("/scratch/garage"), Some("/home/u")),
            PathBuf::from("/scratch/garage")
        );
    }

    #[test]
    fn falls_back_to_home_dot_garage_when_unset_or_empty() {
        assert_eq!(
            garage_dir_from(None, Some("/home/u")),
            PathBuf::from("/home/u/.garage")
        );
        assert_eq!(
            garage_dir_from(Some(""), Some("/home/u")),
            PathBuf::from("/home/u/.garage")
        );
    }

    // ── round trip ──────────────────────────────────────────────────────

    #[test]
    fn serialize_then_deserialize_round_trips() {
        let mut views: ViewsByWorkspace = HashMap::new();
        views.insert(
            "apexlabs".to_owned(),
            vec![view("api-fix", &["garage/apexlabs/api-fix"])],
        );
        let doc = file(views);
        let raw = serialize(&doc);
        assert_eq!(deserialize(&raw), doc);
    }

    #[test]
    fn serialize_omits_workspaces_with_no_named_views() {
        let mut views: ViewsByWorkspace = HashMap::new();
        views.insert("empty".to_owned(), vec![]);
        let raw = serialize(&file(views));
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["views"].as_object().unwrap().len(), 0);
    }

    // ── rail round trip (p25 sidebar hide and resize) ──────────────────

    #[test]
    fn the_rail_choice_round_trips_and_the_default_takes_no_disk_space() {
        let mut f = file(HashMap::new());
        assert!(
            serde_json::from_str::<Value>(&serialize(&f))
                .unwrap()
                .get("rail")
                .is_none()
        );
        f.rail = RailPrefs {
            hidden: true,
            width: 40,
        };
        assert_eq!(deserialize(&serialize(&f)).rail, f.rail);
    }

    #[test]
    fn a_bad_or_out_of_range_rail_value_loads_safely_without_losing_the_views() {
        let raw = r#"{"version":1,"views":{},"rail":{"hidden":"yes","width":9999}}"#;
        let got = deserialize(raw).rail;
        assert!(!got.hidden);
        assert_eq!(got.width, RailPrefs::MAX);
        let raw = r#"{"version":1,"views":{},"rail":"wide"}"#;
        assert_eq!(deserialize(raw).rail, RailPrefs::default());
        let raw = r#"{"version":1,"views":{},"rail":{"width":1}}"#;
        assert_eq!(deserialize(raw).rail.width, RailPrefs::MIN);
    }

    #[test]
    fn a_legacy_pet_key_is_ignored_when_reading() {
        let raw =
            r#"{"version":1,"views":{"w":[{"name":"v","sessions":["garage/w/a"]}]},"pet":"duck"}"#;
        let doc = deserialize(raw);
        assert_eq!(
            doc.views["w"][0].name, "v",
            "the views survive the unknown key"
        );
        let parsed: Value = serde_json::from_str(&serialize(&doc)).unwrap();
        assert!(
            parsed.get("pet").is_none(),
            "serialize alone writes only what it owns"
        );
    }

    #[test]
    fn saving_views_keeps_the_keys_it_does_not_own() {
        let dir = std::env::temp_dir().join(format!("wall-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wall.json");
        std::fs::write(
            &path,
            r#"{"version":1,"views":{"old":[{"name":"x","sessions":["a"]}]},"pet":"duck","future":{"k":[1,2]}}"#,
        )
        .unwrap();
        let mut views: ViewsByWorkspace = HashMap::new();
        views.insert("w".to_owned(), vec![view("v", &["garage/w/a"])]);
        save(&path, &file(views)).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved["pet"], "duck",
            "the legacy pet key stays for the plugin"
        );
        assert_eq!(saved["future"], json!({"k": [1, 2]}));
        assert_eq!(saved["version"], 1);
        assert!(
            saved["views"].get("old").is_none(),
            "the views are the new ones"
        );
        assert!(saved["views"].get("w").is_some());
        // A file that is not an object (or not JSON) is simply replaced.
        std::fs::write(&path, "[1,2").unwrap();
        save(&path, &file(HashMap::new())).unwrap();
        assert_eq!(load(&path), file(HashMap::new()));
    }

    // ── invalid file ────────────────────────────────────────────────────

    #[test]
    fn deserialize_of_garbage_is_the_default_never_an_error() {
        assert_eq!(deserialize("not json"), WallFile::default());
        assert_eq!(deserialize(""), WallFile::default());
        assert_eq!(deserialize("[]"), WallFile::default());
        assert_eq!(deserialize("{}"), WallFile::default());
    }

    #[test]
    fn deserialize_rejects_an_unrecognized_version() {
        assert_eq!(
            deserialize(r#"{"version":2,"views":{}}"#),
            WallFile::default()
        );
        assert_eq!(deserialize(r#"{"views":{}}"#), WallFile::default());
    }

    #[test]
    fn deserialize_skips_one_malformed_entry_without_losing_the_rest() {
        let raw = r#"{"version":1,"views":{
            "a":[{"name":"solo","sessions":["garage/a/one"]}, {"name":123}],
            "b":"not-an-array"
        }}"#;
        let views = deserialize(raw).views;
        assert_eq!(views.get("a").unwrap(), &[view("solo", &["garage/a/one"])]);
        assert!(!views.contains_key("b"));
    }

    #[test]
    fn deserialize_never_stores_the_default_view_by_name() {
        let raw = r#"{"version":1,"views":{"a":[{"name":"main","sessions":["x"]}]}}"#;
        assert!(deserialize(raw).views.is_empty());
    }

    // ── prune (composition with state::views::prune_views) ────────────

    #[test]
    fn a_loaded_file_referencing_a_dead_session_is_pruned_by_the_shared_prune_fn() {
        let raw = r#"{"version":1,"views":{"a":[{"name":"solo","sessions":["live","dead"]}]}}"#;
        let mut views = deserialize(raw).views;
        let valid: HashSet<&str> = ["live"].into_iter().collect();
        crate::state::views::prune_views(&mut views, &valid);
        assert_eq!(views.get("a").unwrap(), &[view("solo", &["live"])]);
    }

    // ── load / save against real files ─────────────────────────────────

    fn scratch_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "garage-wall-persistence-test-{}-{tag}-{n}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn load_of_a_missing_file_is_the_default() {
        let path = scratch_path("missing").join("wall.json");
        assert_eq!(load(&path), WallFile::default());
    }

    #[test]
    fn load_of_an_unreadable_directory_path_is_the_default() {
        // `path` itself is a directory, not a file — read_to_string fails.
        let dir = scratch_path("is-a-dir");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(&dir), WallFile::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_then_load_round_trips_through_real_files() {
        let dir = scratch_path("roundtrip");
        let path = dir.join("wall.json");
        let mut views: ViewsByWorkspace = HashMap::new();
        views.insert("a".to_owned(), vec![view("solo", &["garage/a/one"])]);
        let mut doc = file(views);

        save(&path, &doc).expect("save should create the dir and write atomically");
        assert_eq!(load(&path), doc);

        // A second save overwrites cleanly (exercises the rename-over-
        // existing-file path, not just create-fresh).
        doc.views.get_mut("a").unwrap()[0]
            .session_ids
            .push("garage/a/two".to_owned());
        save(&path, &doc).unwrap();
        assert_eq!(load(&path), doc);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_leaves_no_leftover_temp_file() {
        let dir = scratch_path("no-leftovers");
        let path = dir.join("wall.json");
        let mut views: ViewsByWorkspace = HashMap::new();
        views.insert("a".to_owned(), vec![view("solo", &["x"])]);
        save(&path, &file(views)).unwrap();

        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, ["wall.json"], "only the final file should remain");
        std::fs::remove_dir_all(&dir).ok();
    }
}
