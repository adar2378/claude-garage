//! How a tile attaches to a session: the daemon's [`AttachSpec`] and the
//! helpers that turn it into process argv (tmux) or a host connection (socket).
//!
//! The daemon owns the backend choice (`GET /api/attach?id=`); the wall only
//! follows the spec. A daemon that predates the route, or any failure to ask,
//! falls back to [`tmux_default`], which is exactly the argv the wall used
//! before specs existed.

pub use garage_protocol::AttachSpec;

use crate::api::client::{ApiError, GarageClient};

/// `tmux attach -t =<session>` on the default server: the pre-spec behaviour.
pub fn tmux_default(session: &str) -> AttachSpec {
    AttachSpec::Tmux {
        argv: vec!["tmux".into(), "attach".into(), "-t".into(), format!("={session}")],
        detach_argv: vec!["tmux".into(), "detach-client".into(), "-t".into()],
    }
}

/// The tmux program plus its leading global flags (`tmux -L <socket>`), taken
/// from the spec so every other tmux call (list-clients, detach-client,
/// capture-pane) reaches the same server the attach client does.
pub fn tmux_base(spec: &AttachSpec) -> Vec<String> {
    let AttachSpec::Tmux { argv, detach_argv } = spec else {
        return vec!["tmux".into()];
    };
    for (v, marker) in [(detach_argv, "detach-client"), (argv, "attach")] {
        if let Some(i) = v.iter().position(|a| a == marker) {
            if i > 0 {
                return v[..i].to_vec();
            }
        }
    }
    vec!["tmux".into()]
}

/// Source of a session's [`AttachSpec`] when a tile (re)attaches.
pub trait AttachResolver: Send {
    fn resolve(&self, session: &str) -> Result<AttachSpec, String>;
}

/// Always tmux on the default server (tests and tools with no daemon).
pub struct TmuxOnly;

impl AttachResolver for TmuxOnly {
    fn resolve(&self, session: &str) -> Result<AttachSpec, String> {
        Ok(tmux_default(session))
    }
}

/// Asks the daemon.
pub struct DaemonResolver {
    client: GarageClient,
}

impl DaemonResolver {
    pub fn new(base_url: &str) -> DaemonResolver {
        DaemonResolver { client: GarageClient::new(Some(base_url.to_owned())) }
    }
}

impl AttachResolver for DaemonResolver {
    fn resolve(&self, session: &str) -> Result<AttachSpec, String> {
        decide(self.client.fetch_attach_spec(session), session)
    }
}

/// The daemon's answer, or what to do without one. Only an explicit "no such
/// running session" is an error; an old daemon (route missing) or an
/// unreachable one falls back to the pre-spec tmux behaviour.
pub fn decide(answer: Result<AttachSpec, ApiError>, session: &str) -> Result<AttachSpec, String> {
    match answer {
        Ok(spec) => Ok(spec),
        Err(ApiError::Status { status: 404, message }) if message.contains("no running session") => Err(message),
        Err(_) => Ok(tmux_default(session)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmux_with_socket(socket: &str) -> AttachSpec {
        AttachSpec::Tmux {
            argv: ["tmux", "-L", socket, "attach", "-t", "=s"].map(String::from).to_vec(),
            detach_argv: ["tmux", "-L", socket, "detach-client", "-t"].map(String::from).to_vec(),
        }
    }

    #[test]
    fn the_default_spec_is_todays_attach_argv() {
        let AttachSpec::Tmux { argv, detach_argv } = tmux_default("garage/ws/a") else { panic!() };
        assert_eq!(argv, ["tmux", "attach", "-t", "=garage/ws/a"]);
        assert_eq!(detach_argv, ["tmux", "detach-client", "-t"]);
    }

    #[test]
    fn the_tmux_base_carries_the_socket_flags_and_nothing_else() {
        assert_eq!(tmux_base(&tmux_default("s")), ["tmux"]);
        assert_eq!(tmux_base(&tmux_with_socket("t1")), ["tmux", "-L", "t1"]);
        assert_eq!(tmux_base(&AttachSpec::Socket { addr: "a".into(), token: "t".into() }), ["tmux"]);
    }

    #[test]
    fn only_a_missing_session_is_an_error() {
        let spec = tmux_with_socket("t1");
        assert_eq!(decide(Ok(spec.clone()), "s"), Ok(spec));
        let gone = ApiError::Status { status: 404, message: "no running session garage/ws/a".into() };
        assert!(decide(Err(gone), "garage/ws/a").is_err());
        // An older daemon has no such route: its 404 says "not found".
        let old = ApiError::Status { status: 404, message: "not found".into() };
        assert_eq!(decide(Err(old), "s"), Ok(tmux_default("s")));
        assert_eq!(decide(Err(ApiError::Transport("refused".into())), "s"), Ok(tmux_default("s")));
    }
}
