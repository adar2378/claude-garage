//! quick-send: type one line into the focused session (`s`) or into every
//! live session of the focused project (`S`). The popup names the targets and
//! asks twice (Enter, Enter) before anything is typed anywhere.

use std::time::Duration;

use garage_plugin_sdk::tui::{self, Prompt};
use garage_plugin_sdk::{Api, Context, Value, workspace_of};

/// Who gets the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Targets {
    One(String),
    Project {
        workspace: String,
        sessions: Vec<String>,
    },
}

impl Targets {
    pub fn ids(&self) -> Vec<&str> {
        match self {
            Targets::One(id) => vec![id.as_str()],
            Targets::Project { sessions, .. } => sessions.iter().map(String::as_str).collect(),
        }
    }

    /// The popup title.
    pub fn title(&self) -> String {
        match self {
            Targets::One(id) => format!("Send a line to {id}"),
            Targets::Project {
                workspace,
                sessions,
            } if sessions.len() == 1 => {
                format!(
                    "Send a line to {} (the only session in {workspace})",
                    sessions[0]
                )
            }
            Targets::Project {
                workspace,
                sessions,
            } => {
                format!(
                    "Send a line to all {} sessions in {workspace}",
                    sessions.len()
                )
            }
        }
    }

    /// The second-step wording.
    pub fn confirm(&self) -> String {
        match self.ids().len() {
            1 => "Send to 1 session".to_owned(),
            n => format!("Send to {n} sessions"),
        }
    }
}

/// Why there is nobody to send to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoTarget {
    NoFocus,
    NothingLive(String),
}

impl std::fmt::Display for NoTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoTarget::NoFocus => write!(f, "no session is focused"),
            NoTarget::NothingLive(ws) => write!(f, "{ws} has no live session"),
        }
    }
}

/// The one focused session, or every live session of the focused project.
/// `sessions` is the `GET /api/sessions` array (restorable entries are not
/// live and are skipped).
pub fn targets(
    all: bool,
    session: Option<&str>,
    workspace: Option<&str>,
    sessions: &Value,
) -> Result<Targets, NoTarget> {
    if !all {
        return session
            .map(|s| Targets::One(s.to_owned()))
            .ok_or(NoTarget::NoFocus);
    }
    let workspace = workspace
        .or_else(|| session.and_then(workspace_of))
        .ok_or(NoTarget::NoFocus)?;
    let mut live: Vec<String> = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s.get("restorable").and_then(Value::as_bool) != Some(true))
        .filter_map(|s| s.get("id").and_then(Value::as_str))
        .filter(|id| workspace_of(id) == Some(workspace))
        .map(str::to_owned)
        .collect();
    live.sort();
    if live.is_empty() {
        return Err(NoTarget::NothingLive(workspace.to_owned()));
    }
    Ok(Targets::Project {
        workspace: workspace.to_owned(),
        sessions: live,
    })
}

/// Send `text` to each target; one failure does not stop the rest.
/// Returns `(id, error text)` for the failures.
pub fn send(api: &Api, targets: &Targets, text: &str) -> Vec<(String, String)> {
    targets
        .ids()
        .into_iter()
        .filter_map(|id| {
            api.session_input(id, text)
                .err()
                .map(|e| (id.to_owned(), e.to_string()))
        })
        .collect()
}

/// The popup.
pub fn run(all: bool) -> Result<(), String> {
    let ctx = Context::read().map_err(|e| e.to_string())?;
    let api = Api::from_env().map_err(|e| e.to_string())?;
    let sessions = if all {
        api.get("/api/sessions").map_err(|e| e.to_string())?
    } else {
        Value::Null
    };
    let targets = match targets(all, ctx.session(), ctx.workspace(), &sessions) {
        Ok(t) => t,
        Err(e) => {
            let _ = tui::show(
                &[format!("Nothing to send to: {e}"), "Press any key".into()],
                None,
            );
            return Ok(());
        }
    };
    let prompt = Prompt::new(&targets.title(), &targets.confirm());
    let Some(text) = tui::run_prompt(prompt).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let failed = send(&api, &targets, &text);
    if failed.is_empty() {
        let _ = tui::show(
            &[format!(
                "Sent to {}",
                targets.confirm().trim_start_matches("Send to ")
            )],
            Some(Duration::from_millis(700)),
        );
        return Ok(());
    }
    let mut lines = vec![format!(
        "{} of {} failed:",
        failed.len(),
        targets.ids().len()
    )];
    lines.extend(failed.iter().map(|(id, e)| format!("{id}: {e}")));
    lines.push("Press any key".into());
    let _ = tui::show(&lines, None);
    Err(format!("{} send(s) failed", failed.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::json;
    use garage_plugin_sdk::testing::FakeDaemon;

    fn list() -> Value {
        json!([
            {"id": "garage/app/main", "workspace": "app"},
            {"id": "garage/app/fix", "workspace": "app"},
            {"id": "garage/app/old", "workspace": "app", "restorable": true},
            {"id": "garage/apple/main", "workspace": "apple"},
            {"id": "garage/other/main", "workspace": "other"},
        ])
    }

    #[test]
    fn s_targets_exactly_the_focused_session() {
        let t = targets(false, Some("garage/app/main"), Some("app"), &Value::Null).unwrap();
        assert_eq!(t, Targets::One("garage/app/main".into()));
        assert_eq!(t.title(), "Send a line to garage/app/main");
        assert_eq!(t.confirm(), "Send to 1 session");
        assert_eq!(
            targets(false, None, Some("app"), &Value::Null),
            Err(NoTarget::NoFocus)
        );
    }

    #[test]
    fn capital_s_targets_live_sessions_of_that_project_only() {
        let t = targets(true, Some("garage/app/main"), Some("app"), &list()).unwrap();
        assert_eq!(
            t.ids(),
            ["garage/app/fix", "garage/app/main"],
            "no restorable, no apple/, no other/"
        );
        assert_eq!(t.title(), "Send a line to all 2 sessions in app");
        assert_eq!(t.confirm(), "Send to 2 sessions");
    }

    #[test]
    fn the_project_comes_from_the_session_when_no_workspace_is_focused() {
        let t = targets(true, Some("garage/other/main"), None, &list()).unwrap();
        assert_eq!(t.ids(), ["garage/other/main"]);
        assert!(t.title().contains("only session in other"), "{}", t.title());
    }

    #[test]
    fn nothing_to_send_to_is_reported_not_sent_to_everyone() {
        assert_eq!(targets(true, None, None, &list()), Err(NoTarget::NoFocus));
        assert_eq!(
            targets(true, None, Some("ghost"), &list()),
            Err(NoTarget::NothingLive("ghost".into()))
        );
        assert_eq!(
            NoTarget::NothingLive("ghost".into()).to_string(),
            "ghost has no live session"
        );
    }

    #[test]
    fn a_failure_on_one_target_is_reported_and_the_others_still_get_the_line() {
        let d = FakeDaemon::start(|req| {
            if req.body["id"] == "garage/app/fix" {
                (404, r#"{"error":"no such session"}"#.into())
            } else {
                (200, r#"{"ok":true}"#.into())
            }
        });
        let t = Targets::Project {
            workspace: "app".into(),
            sessions: vec!["garage/app/fix".into(), "garage/app/main".into()],
        };
        let failed = send(&d.api("quick-send"), &t, "/compact");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "garage/app/fix");
        assert!(failed[0].1.contains("404"), "{failed:?}");
        let reqs = d.requests();
        assert_eq!(reqs.len(), 2, "the second target was still tried");
        assert!(
            reqs.iter().all(|r| r.path == "/api/sessions/input"
                && r.authorization.as_deref() == Some("Bearer tok"))
        );
        assert_eq!(
            reqs[1].body,
            json!({"id": "garage/app/main", "text": "/compact"})
        );
    }
}
