//! prune-empty-projects: when the user closes a project's last chat, the
//! project goes too. The daemon decides whether it is empty, atomically with
//! the removal (`DELETE /api/workspaces/{name}?if_empty=1&drop_closed=1`): a
//! live session or a restorable one (a crash waiting to be resumed) keeps the
//! project. Closed records do not; they are the chat the user just closed and
//! go with the project.

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
    match api.delete(&format!(
        "/api/workspaces/{workspace}?if_empty=1&drop_closed=1"
    )) {
        // closing many chats at once races several events against one delete
        Ok(_) | Err(Error::Http { status: 404, .. }) => Ok(Outcome::Pruned(workspace.to_owned())),
        // the daemon found a session in it
        Err(Error::Http { status: 409, .. }) => Ok(Outcome::Kept),
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

    fn daemon(delete_status: u16) -> FakeDaemon {
        FakeDaemon::start(move |req| match (req.method.as_str(), req.path.as_str()) {
            ("DELETE", "/api/workspaces/app?if_empty=1&drop_closed=1") => {
                (delete_status, "{}".into())
            }
            _ => (500, "{}".into()),
        })
    }

    #[test]
    fn a_user_close_of_the_last_chat_asks_the_daemon_to_remove_the_project_if_empty() {
        let d = daemon(204);
        let out = handle(&d.api("p"), &closed("user"), NONE).unwrap();
        assert_eq!(out, Outcome::Pruned("app".into()));
        assert_eq!(
            d.log(),
            ["DELETE /api/workspaces/app?if_empty=1&drop_closed=1"],
            "one atomic call: no list-then-delete window"
        );
    }

    #[test]
    fn a_crash_or_a_daemon_restart_never_prunes_and_does_not_even_look() {
        for reason in ["crash", "daemon-restart", "something-new"] {
            let d = daemon(204);
            assert_eq!(
                handle(&d.api("p"), &closed(reason), NONE).unwrap(),
                Outcome::Ignored
            );
            assert!(d.requests().is_empty(), "{reason}");
        }
    }

    #[test]
    fn a_workspace_the_daemon_finds_occupied_is_kept() {
        // the daemon answers 409 for a live or restorable session
        let d = daemon(409);
        assert_eq!(
            handle(&d.api("p"), &closed("user"), NONE).unwrap(),
            Outcome::Kept
        );
        assert_eq!(d.requests().len(), 1);
    }

    #[test]
    fn an_already_removed_workspace_is_success_and_a_real_failure_is_not() {
        let d = daemon(404);
        assert_eq!(
            handle(&d.api("p"), &closed("user"), NONE).unwrap(),
            Outcome::Pruned("app".into())
        );
        let d = daemon(500);
        assert!(matches!(
            handle(&d.api("p"), &closed("user"), NONE),
            Err(Error::Http { status: 500, .. })
        ));
    }

    #[test]
    fn an_event_without_a_workspace_is_ignored() {
        let d = daemon(204);
        let data = json!({"reason": "user"});
        assert_eq!(handle(&d.api("p"), &data, NONE).unwrap(), Outcome::Ignored);
    }
}
