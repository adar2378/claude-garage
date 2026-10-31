//! prune-empty-projects: when the user closes a project's last chat, the
//! project goes too. "Empty" means the workspace has no entry at all in
//! `GET /api/sessions`: not live, and not `restorable` (a restorable entry is
//! a crash waiting to be resumed).

use std::time::Duration;

use garage_plugin_sdk::{Api, Error, Value};

/// How long to wait after the close before looking: a user who closes one
/// chat and opens another a moment later keeps the project.
pub const SETTLE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not a user close, or the event named no workspace.
    Ignored,
    /// The workspace still has sessions.
    Kept,
    /// The workspace was removed (or was already gone).
    Pruned(String),
}

/// Does `sessions` (the `GET /api/sessions` array) hold nothing for `workspace`?
pub fn is_empty(sessions: &Value, workspace: &str) -> bool {
    !sessions.as_array().into_iter().flatten().any(|s| {
        s.get("workspace").and_then(Value::as_str) == Some(workspace)
            || s.get("id")
                .and_then(Value::as_str)
                .and_then(garage_plugin_sdk::workspace_of)
                == Some(workspace)
    })
}

/// Handle one `session.closed` event payload.
pub fn handle(api: &Api, data: &Value, settle: Duration) -> Result<Outcome, Error> {
    if data.get("reason").and_then(Value::as_str) != Some("user") {
        return Ok(Outcome::Ignored);
    }
    let Some(workspace) = data
        .get("workspace")
        .and_then(Value::as_str)
        .filter(|w| !w.is_empty())
    else {
        return Ok(Outcome::Ignored);
    };
    std::thread::sleep(settle);
    if !is_empty(&api.get("/api/sessions")?, workspace) {
        return Ok(Outcome::Kept);
    }
    match api.delete(&format!("/api/workspaces/{workspace}")) {
        // closing many chats at once races several events against one delete
        Ok(_) | Err(Error::Http { status: 404, .. }) => Ok(Outcome::Pruned(workspace.to_owned())),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::json;
    use garage_plugin_sdk::testing::FakeDaemon;

    const NONE: Duration = Duration::ZERO;

    fn closed(reason: &str) -> Value {
        json!({"id": "garage/app/main", "workspace": "app", "reason": reason})
    }

    fn daemon(sessions: &'static str, delete_status: u16) -> FakeDaemon {
        FakeDaemon::start(move |req| match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/api/sessions") => (200, sessions.into()),
            ("DELETE", "/api/workspaces/app") => (delete_status, "{}".into()),
            _ => (500, "{}".into()),
        })
    }

    #[test]
    fn only_a_session_of_that_workspace_makes_it_non_empty() {
        let list = json!([
            {"id": "garage/other/a", "workspace": "other", "status": "idle"},
            {"id": "garage/app/b", "workspace": "app", "restorable": true, "status": "restorable"},
        ]);
        assert!(!is_empty(&list, "app"), "a restorable entry still counts");
        assert!(is_empty(&list, "third"));
        assert!(is_empty(&json!([]), "app"));
        assert!(is_empty(&Value::Null, "app"));
        // an entry with no workspace field is matched by its id
        assert!(!is_empty(&json!([{"id": "garage/app/c"}]), "app"));
    }

    #[test]
    fn a_user_close_of_the_last_chat_removes_the_project() {
        let d = daemon("[]", 204);
        let out = handle(&d.api("p"), &closed("user"), NONE).unwrap();
        assert_eq!(out, Outcome::Pruned("app".into()));
        assert_eq!(d.log(), ["GET /api/sessions", "DELETE /api/workspaces/app"]);
    }

    #[test]
    fn a_crash_or_a_daemon_restart_never_prunes_and_does_not_even_look() {
        for reason in ["crash", "daemon-restart", "something-new"] {
            let d = daemon("[]", 204);
            assert_eq!(
                handle(&d.api("p"), &closed(reason), NONE).unwrap(),
                Outcome::Ignored
            );
            assert!(d.requests().is_empty(), "{reason}");
        }
    }

    #[test]
    fn a_workspace_that_still_has_a_chat_is_kept() {
        let d = daemon(
            r#"[{"id":"garage/app/other","workspace":"app","status":"idle"}]"#,
            204,
        );
        assert_eq!(
            handle(&d.api("p"), &closed("user"), NONE).unwrap(),
            Outcome::Kept
        );
        assert_eq!(d.log(), ["GET /api/sessions"], "nothing is deleted");
    }

    #[test]
    fn an_already_removed_workspace_is_success_and_a_real_failure_is_not() {
        let d = daemon("[]", 404);
        assert_eq!(
            handle(&d.api("p"), &closed("user"), NONE).unwrap(),
            Outcome::Pruned("app".into())
        );
        let d = daemon("[]", 500);
        assert!(matches!(
            handle(&d.api("p"), &closed("user"), NONE),
            Err(Error::Http { status: 500, .. })
        ));
    }

    #[test]
    fn an_event_without_a_workspace_is_ignored() {
        let d = daemon("[]", 204);
        let data = json!({"reason": "user"});
        assert_eq!(handle(&d.api("p"), &data, NONE).unwrap(), Outcome::Ignored);
    }
}
