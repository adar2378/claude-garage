//! quick-send: type one line into the focused session (`s`) or into every
//! live session of the focused project (`S`). The popup names the targets and
//! asks twice (Enter, Enter) before anything is typed anywhere.

use std::time::Duration;

use garage_plugin_sdk::tui::{self, Prompt};
use garage_plugin_sdk::{Api, Context, Value, workspace_of};

/// The status the daemon reports for a session that is asking the user
/// something (a permission prompt, a question).
const WAITING: &str = "needs-input";

/// Who gets the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Targets {
    /// `waiting`: the session is in `needs-input`; a line typed now would
    /// answer its prompt, so the popup makes the user say so first.
    One { id: String, waiting: bool },
    /// The live sessions of a project that are not waiting for input.
    /// `skipped` are the ones that were left out for that reason.
    Project {
        workspace: String,
        sessions: Vec<String>,
        skipped: Vec<String>,
    },
}

/// `ws/label` of `garage/<ws>/<label>`.
fn short(id: &str) -> &str {
    id.strip_prefix("garage/").unwrap_or(id)
}

impl Targets {
    pub fn ids(&self) -> Vec<&str> {
        match self {
            Targets::One { id, .. } => vec![id.as_str()],
            Targets::Project { sessions, .. } => sessions.iter().map(String::as_str).collect(),
        }
    }

    /// The popup title.
    pub fn title(&self) -> String {
        match self {
            Targets::One { id, .. } => format!("Send a line to {id}"),
            Targets::Project {
                workspace,
                sessions,
                ..
            } if sessions.len() == 1 => {
                format!(
                    "Send a line to {} (the only session in {workspace})",
                    sessions[0]
                )
            }
            Targets::Project {
                workspace,
                sessions,
                ..
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

    /// Lines under the prompt: who was left out, and why.
    pub fn notes(&self) -> Vec<String> {
        let Targets::Project { skipped, .. } = self else {
            return Vec::new();
        };
        let mut notes: Vec<String> = skipped
            .iter()
            .take(2)
            .map(|id| format!("skipped: {} — waiting for input", short(id)))
            .collect();
        if skipped.len() > 2 {
            notes.push(format!("and {} more waiting for input", skipped.len() - 2));
        }
        notes
    }

    /// The warning for a single session that is waiting for input.
    pub fn warning(&self) -> Option<String> {
        match self {
            Targets::One { id, waiting: true } => Some(format!(
                "{} is waiting for input; a line now answers its prompt",
                short(id)
            )),
            _ => None,
        }
    }
}

/// Why there is nobody to send to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoTarget {
    NoFocus,
    NothingLive(String),
    /// Every live session of the project is waiting for input.
    AllWaiting(String, Vec<String>),
}

impl std::fmt::Display for NoTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoTarget::NoFocus => write!(f, "no session is focused"),
            NoTarget::NothingLive(ws) => write!(f, "{ws} has no live session"),
            NoTarget::AllWaiting(ws, ids) => write!(
                f,
                "every session in {ws} is waiting for input ({})",
                ids.iter().map(|i| short(i)).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

fn status_of<'a>(sessions: &'a Value, id: &str) -> Option<&'a str> {
    sessions
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|s| s.get("status").and_then(Value::as_str))
}

/// The one focused session, or every live session of the focused project that
/// is not waiting for input. `sessions` is the `GET /api/sessions` array
/// (restorable entries are not live and are skipped).
pub fn targets(
    all: bool,
    session: Option<&str>,
    workspace: Option<&str>,
    sessions: &Value,
) -> Result<Targets, NoTarget> {
    if !all {
        let id = session.ok_or(NoTarget::NoFocus)?;
        return Ok(Targets::One {
            id: id.to_owned(),
            waiting: status_of(sessions, id) == Some(WAITING),
        });
    }
    let workspace = workspace
        .or_else(|| session.and_then(workspace_of))
        .ok_or(NoTarget::NoFocus)?;
    let mut live: Vec<(String, bool)> = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s.get("restorable").and_then(Value::as_bool) != Some(true))
        .filter_map(|s| {
            let id = s.get("id").and_then(Value::as_str)?;
            let waiting = s.get("status").and_then(Value::as_str) == Some(WAITING);
            Some((id.to_owned(), waiting))
        })
        .filter(|(id, _)| workspace_of(id) == Some(workspace))
        .collect();
    live.sort();
    if live.is_empty() {
        return Err(NoTarget::NothingLive(workspace.to_owned()));
    }
    let (skipped, sendable): (Vec<_>, Vec<_>) = live.into_iter().partition(|(_, w)| *w);
    let skipped: Vec<String> = skipped.into_iter().map(|(id, _)| id).collect();
    if sendable.is_empty() {
        return Err(NoTarget::AllWaiting(workspace.to_owned(), skipped));
    }
    Ok(Targets::Project {
        workspace: workspace.to_owned(),
        sessions: sendable.into_iter().map(|(id, _)| id).collect(),
        skipped,
    })
}

/// What `send` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Sent {
    /// `(id, error text)`.
    pub failed: Vec<(String, String)>,
    /// Sessions that started waiting for input after the popup was drawn; the
    /// daemon refused to type into them.
    pub skipped: Vec<String>,
}

/// Send `text` to each target; one failure does not stop the rest. Every
/// send carries the daemon's status guard, so a session that turned to
/// `needs-input` meanwhile is skipped by the daemon. The one exception is a
/// single session the user already confirmed typing into while it waits.
pub fn send(api: &Api, targets: &Targets, text: &str) -> Sent {
    let allow_waiting = matches!(targets, Targets::One { waiting: true, .. });
    let mut out = Sent::default();
    for id in targets.ids() {
        let res = if allow_waiting {
            api.session_input(id, text)
        } else {
            api.session_input_unless(id, text, &[WAITING])
        };
        match res {
            Ok(()) => {}
            Err(garage_plugin_sdk::Error::Http { status: 409, .. }) => {
                out.skipped.push(id.to_owned());
            }
            Err(e) => out.failed.push((id.to_owned(), e.to_string())),
        }
    }
    out
}

/// The popup.
pub fn run(all: bool) -> Result<(), String> {
    let ctx = Context::read().map_err(|e| e.to_string())?;
    let api = Api::from_env().map_err(|e| e.to_string())?;
    // The statuses decide who is skipped (S) or warned about (s). Without them
    // the popup cannot tell, so a failed read stops here rather than guessing.
    let sessions = api.get("/api/sessions").map_err(|e| e.to_string())?;
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
    let mut prompt = Prompt::new(&targets.title(), &targets.confirm()).with_notes(targets.notes());
    if let Some(w) = targets.warning() {
        prompt = prompt.with_warning(&w);
    }
    let Some(text) = tui::run_prompt(prompt).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let sent = send(&api, &targets, &text);
    if sent.failed.is_empty() && sent.skipped.is_empty() {
        let _ = tui::show(
            &[format!(
                "Sent to {}",
                targets.confirm().trim_start_matches("Send to ")
            )],
            Some(Duration::from_millis(700)),
        );
        return Ok(());
    }
    let mut lines = Vec::new();
    for id in &sent.skipped {
        lines.push(format!("skipped: {} — waiting for input", short(id)));
    }
    if !sent.failed.is_empty() {
        lines.push(format!(
            "{} of {} failed:",
            sent.failed.len(),
            targets.ids().len()
        ));
        lines.extend(sent.failed.iter().map(|(id, e)| format!("{id}: {e}")));
    }
    lines.push("Press any key".into());
    let _ = tui::show(&lines, None);
    if sent.failed.is_empty() {
        Ok(())
    } else {
        Err(format!("{} send(s) failed", sent.failed.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::json;
    use garage_plugin_sdk::testing::FakeDaemon;

    fn list() -> Value {
        json!([
            {"id": "garage/app/main", "workspace": "app", "status": "idle"},
            {"id": "garage/app/fix", "workspace": "app", "status": "working"},
            {"id": "garage/app/old", "workspace": "app", "restorable": true},
            {"id": "garage/apple/main", "workspace": "apple", "status": "idle"},
            {"id": "garage/other/main", "workspace": "other", "status": "idle"},
        ])
    }

    fn with_waiting() -> Value {
        json!([
            {"id": "garage/app/main", "status": "idle"},
            {"id": "garage/app/ask", "status": "needs-input"},
            {"id": "garage/app/fix", "status": "working"},
            {"id": "garage/app/also", "status": "needs-input"},
            {"id": "garage/app/third", "status": "needs-input"},
        ])
    }

    #[test]
    fn s_targets_exactly_the_focused_session() {
        let t = targets(false, Some("garage/app/main"), Some("app"), &list()).unwrap();
        assert_eq!(
            t,
            Targets::One {
                id: "garage/app/main".into(),
                waiting: false
            }
        );
        assert_eq!(t.title(), "Send a line to garage/app/main");
        assert_eq!(t.confirm(), "Send to 1 session");
        assert_eq!(t.warning(), None);
        assert_eq!(
            targets(false, None, Some("app"), &list()),
            Err(NoTarget::NoFocus)
        );
    }

    #[test]
    fn s_on_a_session_waiting_for_input_warns() {
        let t = targets(false, Some("garage/app/ask"), None, &with_waiting()).unwrap();
        assert_eq!(
            t,
            Targets::One {
                id: "garage/app/ask".into(),
                waiting: true
            }
        );
        assert!(
            t.warning()
                .unwrap()
                .contains("app/ask is waiting for input")
        );
        // working is not waiting
        let t = targets(false, Some("garage/app/fix"), None, &with_waiting()).unwrap();
        assert_eq!(t.warning(), None);
    }

    #[test]
    fn capital_s_targets_live_sessions_of_that_project_only() {
        let t = targets(true, Some("garage/app/main"), Some("app"), &list()).unwrap();
        assert_eq!(
            t.ids(),
            ["garage/app/fix", "garage/app/main"],
            "no restorable, no apple/, no other/; working sessions still get the line"
        );
        assert_eq!(t.title(), "Send a line to all 2 sessions in app");
        assert_eq!(t.confirm(), "Send to 2 sessions");
        assert!(t.notes().is_empty());
    }

    #[test]
    fn capital_s_skips_sessions_waiting_for_input_and_names_them() {
        let t = targets(true, None, Some("app"), &with_waiting()).unwrap();
        assert_eq!(t.ids(), ["garage/app/fix", "garage/app/main"]);
        assert_eq!(
            t.notes(),
            [
                "skipped: app/also — waiting for input",
                "skipped: app/ask — waiting for input",
                "and 1 more waiting for input"
            ]
        );
    }

    #[test]
    fn a_project_where_everything_is_waiting_has_nobody_to_send_to() {
        let only = json!([{"id": "garage/app/ask", "status": "needs-input"}]);
        let err = targets(true, None, Some("app"), &only).unwrap_err();
        assert_eq!(
            err,
            NoTarget::AllWaiting("app".into(), vec!["garage/app/ask".into()])
        );
        assert_eq!(
            err.to_string(),
            "every session in app is waiting for input (app/ask)"
        );
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

    fn project(ids: &[&str]) -> Targets {
        Targets::Project {
            workspace: "app".into(),
            sessions: ids.iter().map(|s| (*s).to_owned()).collect(),
            skipped: vec![],
        }
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
        let t = project(&["garage/app/fix", "garage/app/main"]);
        let sent = send(&d.api("quick-send"), &t, "/compact");
        assert_eq!(sent.failed.len(), 1);
        assert_eq!(sent.failed[0].0, "garage/app/fix");
        assert!(sent.failed[0].1.contains("404"), "{sent:?}");
        let reqs = d.requests();
        assert_eq!(reqs.len(), 2, "the second target was still tried");
        assert!(
            reqs.iter().all(|r| r.path == "/api/sessions/input"
                && r.authorization.as_deref() == Some("Bearer tok"))
        );
        assert_eq!(
            reqs[1].body,
            json!({"id": "garage/app/main", "text": "/compact", "if_status": {"not": ["needs-input"]}})
        );
    }

    #[test]
    fn a_session_that_starts_waiting_meanwhile_is_skipped_by_the_daemon_not_failed() {
        let d = FakeDaemon::start(|req| {
            if req.body["id"] == "garage/app/ask" {
                (409, r#"{"error":"garage/app/ask is needs-input; nothing was typed","status":"needs-input"}"#.into())
            } else {
                (200, r#"{"ok":true}"#.into())
            }
        });
        let t = project(&["garage/app/ask", "garage/app/main"]);
        let sent = send(&d.api("quick-send"), &t, "go");
        assert_eq!(sent.skipped, ["garage/app/ask"]);
        assert!(sent.failed.is_empty());
    }

    #[test]
    fn s_on_a_waiting_session_the_user_confirmed_is_sent_without_the_guard() {
        let d = FakeDaemon::start(|_| (200, r#"{"ok":true}"#.into()));
        let t = Targets::One {
            id: "garage/app/ask".into(),
            waiting: true,
        };
        assert_eq!(send(&d.api("quick-send"), &t, "y"), Sent::default());
        assert_eq!(
            d.requests()[0].body,
            json!({"id": "garage/app/ask", "text": "y"}),
            "the user accepted the warning, so the daemon must not refuse"
        );
        // while a plain s on a session that is not waiting stays guarded
        let d = FakeDaemon::start(|_| (409, r#"{"error":"x"}"#.into()));
        let t = Targets::One {
            id: "garage/app/main".into(),
            waiting: false,
        };
        assert_eq!(send(&d.api("quick-send"), &t, "y").skipped.len(), 1);
        assert_eq!(d.requests()[0].body["if_status"]["not"][0], "needs-input");
    }
}
