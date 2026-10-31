//! How a tile attaches to a session.
//!
//! The daemon only says WHERE the session lives ([`AttachSpec`]: a tmux
//! socket name, or a session host id); the wall builds every command and
//! connection itself. Whatever answers on the daemon port can therefore never
//! make the wall run an arbitrary program or dial an arbitrary address:
//!
//! - tmux: the fixed argv `tmux [-L <socket>] attach -t =<session>`, with the
//!   socket name checked against `^[A-Za-z0-9_-]{1,64}$`;
//! - host: address and token are read from `hosts/<id>.json` in the
//!   user-private garage dir (the token is never sent over HTTP), and the
//!   address must be loopback.
//!
//! A daemon that predates the route, or any failure to ask, falls back to
//! [`Target::tmux_default`], exactly the argv the wall used before specs.

use std::path::{Path, PathBuf};

use garage_host::registry;
pub use garage_protocol::AttachSpec;
use garage_protocol::is_valid_tmux_socket;

use crate::api::client::{ApiError, GarageClient};

/// A validated way to reach a session; the only thing the PTY and scrollback
/// code accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Tmux { socket: Option<String> },
    Socket { addr: String, token: String },
}

impl Target {
    /// `tmux attach` on the default server: the pre-spec behaviour.
    pub fn tmux_default() -> Target {
        Target::Tmux { socket: None }
    }

    /// The tmux program plus its global flags: `["tmux"]` or `["tmux", "-L", s]`.
    /// Every tmux call made for a tile (attach, list-clients, detach,
    /// capture) starts with this, so they all reach the same server.
    pub fn tmux_base(&self) -> Vec<String> {
        match self {
            Target::Tmux { socket: Some(s) } => vec!["tmux".into(), "-L".into(), s.clone()],
            _ => vec!["tmux".into()],
        }
    }

    /// Full argv of the attach client for `session`.
    pub fn attach_argv(&self, session: &str) -> Vec<String> {
        let mut v = self.tmux_base();
        v.extend(["attach".into(), "-t".into(), format!("={session}")]);
        v
    }
}

/// Turn the daemon's spec into a [`Target`], reading the registry from
/// `hosts_dir` for a host. Refuses anything unsafe.
pub fn target_from(spec: AttachSpec, hosts_dir: &Path) -> Result<Target, String> {
    match spec {
        AttachSpec::Tmux { socket: None } => Ok(Target::Tmux { socket: None }),
        AttachSpec::Tmux { socket: Some(s) } if is_valid_tmux_socket(&s) => Ok(Target::Tmux { socket: Some(s) }),
        AttachSpec::Tmux { socket: Some(s) } => Err(format!("refusing tmux socket name {s:?}")),
        AttachSpec::Host { id } if id.is_empty() => Err("no running session".into()),
        AttachSpec::Host { id } => {
            let e = registry::read(hosts_dir, &id).ok_or_else(|| format!("no session host registered for {id}"))?;
            let addr = e.addr();
            garage_host::proto::parse_loopback(&addr).map_err(|e| e.to_string())?;
            Ok(Target::Socket { addr, token: e.token })
        }
    }
}

/// `$GARAGE_DIR`, else `~/.garage` (`%USERPROFILE%` where `HOME` is unset).
pub fn garage_dir() -> PathBuf {
    crate::state::persistence::garage_dir_from_env()
}

/// Source of a session's [`Target`] when a tile (re)attaches.
pub trait AttachResolver: Send {
    fn resolve(&self, session: &str) -> Result<Target, String>;
}

/// Always tmux on the default server (tests and tools with no daemon).
pub struct TmuxOnly;

impl AttachResolver for TmuxOnly {
    fn resolve(&self, _session: &str) -> Result<Target, String> {
        Ok(Target::tmux_default())
    }
}

/// Asks the daemon.
pub struct DaemonResolver {
    client: GarageClient,
    hosts_dir: PathBuf,
}

impl DaemonResolver {
    pub fn new(base_url: &str) -> DaemonResolver {
        DaemonResolver { client: GarageClient::new(Some(base_url.to_owned())), hosts_dir: garage_dir().join("hosts") }
    }
}

impl AttachResolver for DaemonResolver {
    fn resolve(&self, session: &str) -> Result<Target, String> {
        decide(self.client.fetch_attach_spec(session), &self.hosts_dir)
    }
}

/// The daemon's answer, or what to do without one. Only an explicit "no such
/// running session" is an error; an old daemon (route missing) or an
/// unreachable one falls back to the pre-spec tmux behaviour. A spec the wall
/// refuses is an error, never a fallback.
pub fn decide(answer: Result<AttachSpec, ApiError>, hosts_dir: &Path) -> Result<Target, String> {
    match answer {
        Ok(spec) => target_from(spec, hosts_dir),
        Err(ApiError::Status { status: 404, message }) if message.contains("no running session") => Err(message),
        Err(_) => Ok(Target::tmux_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmux(socket: Option<&str>) -> AttachSpec {
        AttachSpec::Tmux { socket: socket.map(String::from) }
    }

    #[test]
    fn tmux_argv_is_exactly_what_the_wall_ran_before_specs() {
        assert_eq!(Target::tmux_default().attach_argv("garage/ws/a"), ["tmux", "attach", "-t", "=garage/ws/a"]);
        let t = target_from(tmux(Some("t1")), Path::new("/nope")).unwrap();
        assert_eq!(t.attach_argv("garage/ws/a"), ["tmux", "-L", "t1", "attach", "-t", "=garage/ws/a"]);
        assert_eq!(t.tmux_base(), ["tmux", "-L", "t1"]);
        assert_eq!(Target::tmux_default().tmux_base(), ["tmux"]);
    }

    #[test]
    fn a_bad_socket_name_is_refused_not_passed_to_tmux() {
        for bad in ["", "a b", "../x", "-f/etc/passwd x", "a;b", "x\ny", &"a".repeat(65)] {
            let e = target_from(tmux(Some(bad)), Path::new("/nope"));
            assert!(e.is_err(), "{bad:?} must be refused");
        }
    }

    fn write_host(dir: &Path, id: &str, port: u16) {
        registry::write(
            dir,
            &registry::Entry {
                id: id.into(), pid: 1, child_pid: 2, port, token: "t".repeat(32), dir: "/w".into(), created_at: 1,
                title: String::new(),
            },
        )
        .unwrap();
    }

    #[test]
    fn a_host_spec_is_resolved_from_the_registry_file_never_from_the_wire() {
        let d = tempfile::tempdir().unwrap();
        write_host(d.path(), "garage/ws/a", 4321);
        let t = target_from(AttachSpec::Host { id: "garage/ws/a".into() }, d.path()).unwrap();
        assert_eq!(t, Target::Socket { addr: "127.0.0.1:4321".into(), token: "t".repeat(32) });
        assert!(target_from(AttachSpec::Host { id: "garage/ws/none".into() }, d.path()).is_err());
        assert!(target_from(AttachSpec::Host { id: String::new() }, d.path()).is_err());
    }

    #[test]
    fn a_registry_record_pointing_off_loopback_is_refused() {
        let d = tempfile::tempdir().unwrap();
        // The registry is ours, but a tampered or foreign record must still not be dialed.
        std::fs::write(
            d.path().join("garage__ws__x.json"),
            serde_json::json!({"id":"garage/ws/x","pid":1,"childPid":2,"port":80,"token":"t".repeat(32),"dir":"/","createdAt":1})
                .to_string(),
        )
        .unwrap();
        // addr() is always 127.0.0.1:<port>, so the loopback check is the backstop for the
        // address string; prove the backstop itself.
        assert!(garage_host::proto::parse_loopback("203.0.113.9:80").is_err());
        let t = target_from(AttachSpec::Host { id: "garage/ws/x".into() }, d.path()).unwrap();
        assert!(matches!(t, Target::Socket { ref addr, .. } if addr == "127.0.0.1:80"));
    }

    #[test]
    fn only_a_missing_session_or_a_refused_spec_is_an_error() {
        let d = Path::new("/nope");
        assert_eq!(decide(Ok(tmux(Some("t1"))), d), Ok(Target::Tmux { socket: Some("t1".into()) }));
        let gone = ApiError::Status { status: 404, message: "no running session garage/ws/a".into() };
        assert!(decide(Err(gone), d).is_err());
        assert!(decide(Ok(tmux(Some("bad name"))), d).is_err(), "a hostile spec never falls back");
        // An older daemon has no such route: its 404 says "not found".
        let old = ApiError::Status { status: 404, message: "not found".into() };
        assert_eq!(decide(Err(old), d), Ok(Target::tmux_default()));
        assert_eq!(decide(Err(ApiError::Transport("refused".into())), d), Ok(Target::tmux_default()));
    }
}
