//! jump-palette: a fuzzy list of live sessions and projects. Choosing one
//! asks the wall to focus it (`ui/action` with `session.focus` or
//! `workspace.focus`). `session.engage` is refused to plugins on purpose, so
//! the palette moves focus but never starts typing into a session.

use garage_plugin_sdk::tui::{self, Picker};
use garage_plugin_sdk::{Api, Error, Target, Value};

/// What a palette line jumps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Session(String),
    Workspace(String),
}

impl Choice {
    /// The core action and its target.
    pub fn action(&self) -> (&'static str, Target<'_>) {
        match self {
            Choice::Session(id) => ("session.focus", Target::Session(id)),
            Choice::Workspace(name) => ("workspace.focus", Target::Workspace(name)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub line: String,
    pub choice: Choice,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Live sessions first (sorted), then projects. `restorable` entries are not
/// live sessions and are left out.
pub fn entries(sessions: &Value, workspaces: &Value) -> Vec<Entry> {
    let mut out: Vec<Entry> = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s.get("restorable").and_then(Value::as_bool) != Some(true))
        .filter_map(|s| {
            let id = text(s, "id")?;
            let name = id.strip_prefix("garage/").unwrap_or(id);
            let mut line = name.to_owned();
            for extra in [text(s, "status"), text(s, "branch")].into_iter().flatten() {
                line.push_str("  ");
                line.push_str(extra);
            }
            Some(Entry {
                line,
                choice: Choice::Session(id.to_owned()),
            })
        })
        .collect();
    out.sort_by(|a, b| a.line.cmp(&b.line));
    let mut projects: Vec<Entry> = workspaces
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let name = text(w, "name")?;
            let dir = text(w, "dir").unwrap_or("");
            Some(Entry {
                line: format!("{name}  project  {dir}").trim_end().to_owned(),
                choice: Choice::Workspace(name.to_owned()),
            })
        })
        .collect();
    projects.sort_by(|a, b| a.line.cmp(&b.line));
    out.extend(projects);
    out
}

/// Ask the wall to focus the choice.
pub fn jump(api: &Api, choice: &Choice) -> Result<(), Error> {
    let (action, target) = choice.action();
    api.ui_action(action, target)
}

/// The palette pane: pick, jump, exit.
pub fn run() -> Result<(), String> {
    let api = Api::from_env().map_err(|e| e.to_string())?;
    let sessions = api.get("/api/sessions").map_err(|e| e.to_string())?;
    let workspaces = api.get("/api/workspaces").map_err(|e| e.to_string())?;
    let all = entries(&sessions, &workspaces);
    if all.is_empty() {
        let _ = tui::show(
            &["Nothing to jump to".to_owned(), "Press any key".to_owned()],
            None,
        );
        return Ok(());
    }
    let lines = all.iter().map(|e| e.line.clone()).collect();
    let Some(i) = tui::run_picker(Picker::new("Jump to", lines)).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    if let Err(e) = jump(&api, &all[i].choice) {
        let _ = tui::show(
            &[format!("Could not jump: {e}"), "Press any key".to_owned()],
            None,
        );
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::json;
    use garage_plugin_sdk::testing::FakeDaemon;

    fn sessions() -> Value {
        json!([
            {"id": "garage/web/main", "status": "working", "branch": "main"},
            {"id": "garage/api/fix-auth", "status": "waiting", "branch": null},
            {"id": "garage/api/old", "status": "restorable", "restorable": true},
        ])
    }

    fn workspaces() -> Value {
        json!([{"name": "web", "dir": "/dev/web"}, {"name": "api", "dir": "/dev/api"}])
    }

    #[test]
    fn live_sessions_come_first_then_projects_and_restorable_entries_are_left_out() {
        let e = entries(&sessions(), &workspaces());
        let lines: Vec<&str> = e.iter().map(|e| e.line.as_str()).collect();
        assert_eq!(
            lines,
            [
                "api/fix-auth  waiting",
                "web/main  working  main",
                "api  project  /dev/api",
                "web  project  /dev/web",
            ]
        );
        assert_eq!(e[0].choice, Choice::Session("garage/api/fix-auth".into()));
        assert_eq!(e[3].choice, Choice::Workspace("web".into()));
    }

    #[test]
    fn malformed_entries_are_skipped_not_fatal() {
        let e = entries(&json!([{"status": "idle"}, 5]), &json!([{"dir": "/x"}]));
        assert!(e.is_empty());
        assert!(entries(&Value::Null, &Value::Null).is_empty());
    }

    #[test]
    fn a_session_focuses_with_the_session_as_target_and_a_project_with_the_workspace() {
        let d = FakeDaemon::start(|_| (200, r#"{"ok":true}"#.into()));
        let api = d.api("jump-palette");
        jump(&api, &Choice::Session("garage/web/main".into())).unwrap();
        jump(&api, &Choice::Workspace("api".into())).unwrap();
        let reqs = d.requests();
        assert_eq!(reqs[0].path, "/api/plugins/ui/action");
        assert_eq!(
            reqs[0].body,
            json!({"action": "session.focus", "target": {"session": "garage/web/main"}})
        );
        assert_eq!(
            reqs[1].body,
            json!({"action": "workspace.focus", "target": {"workspace": "api"}})
        );
        assert_eq!(reqs[0].authorization.as_deref(), Some("Bearer tok"));
    }

    #[test]
    fn a_refused_jump_is_an_error_the_pane_can_show() {
        let d = FakeDaemon::start(|_| {
            (
                403,
                r#"{"error":"plugin jump-palette lacks permission ui.invoke"}"#.into(),
            )
        });
        let err = jump(&d.api("jump-palette"), &Choice::Workspace("api".into())).unwrap_err();
        assert!(err.to_string().contains("ui.invoke"), "{err}");
    }
}
