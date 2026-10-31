//! Port of `daemon/src/tmux.js`: the `SessionBackend` implementation over tmux.
//!
//! When `GARAGE_TMUX_SOCKET` is set every command runs as
//! `tmux -L <socket> -f /dev/null ...` with `TMUX`/`TMUX_PANE` removed from the
//! child environment, so an isolated server can never be confused with the
//! user's. When unset the argv is exactly what the JS daemon ran.
//!
//! Note `-f /dev/null`: setting `GARAGE_TMUX_SOCKET` also means "do not load the
//! user's tmux.conf". That is intended (tests and isolation); production leaves
//! the variable unset.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use garage_protocol::AttachSpec;
use tokio::process::Command;

use super::{BackendSession, PaneInfo, SessionBackend};

pub const GARAGE_PREFIX: &str = "garage/";

/// Name of the control client's own session. Deliberately outside
/// `GARAGE_PREFIX`, so `list`/`panes` and every garage listing skip it.
pub const CONTROL_SESSION: &str = "garage-control";

/// Workspace and session labels: `[a-z0-9-]+` (the JS `NAME_RE`).
pub fn is_valid_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `garage/<workspace>/<label>`.
pub fn session_id(workspace: &str, label: &str) -> String {
    format!("{GARAGE_PREFIX}{workspace}/{label}")
}

/// Split `garage/<workspace>/<label>` the way the JS does (`name.split("/")`,
/// taking parts 1 and 2). Missing parts are empty strings.
pub fn split_session_id(id: &str) -> (String, String) {
    let mut parts = id.split('/');
    parts.next();
    (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string())
}

/// `=` forces exact-name matching; a bare `-t` does prefix matching.
fn exact(id: &str) -> String {
    format!("={id}")
}

/// Pure split of a `#{session_name}\t#{pane_current_path}\t#{pane_title}` line.
/// `pane_title` is free text and sits last, so anything past the second tab
/// is rejoined into it (an embedded tab can never shift the earlier fields).
pub fn parse_pane_line(line: &str) -> (String, String, String) {
    let mut it = line.splitn(3, '\t');
    let name = it.next().unwrap_or("").to_string();
    let path = it.next().unwrap_or("").to_string();
    let title = it.next().unwrap_or("").to_string();
    (name, path, title)
}

/// tmux's own default pane titles carry no signal: empty, the machine
/// hostname (full or short), or a bare shell name. Anything else is a title
/// something set (e.g. Claude Code's OSC updates) and is returned trimmed.
pub fn normalize_title(title: Option<&str>, hostname: Option<&str>) -> Option<String> {
    const BARE: [&str; 9] = ["sh", "bash", "zsh", "fish", "-sh", "-bash", "-zsh", "-fish", "tmux"];
    let trimmed = title?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    if BARE.contains(&lower.as_str()) {
        return None;
    }
    if let Some(h) = hostname.filter(|h| !h.is_empty()) {
        let host = h.to_lowercase();
        let short = host.split('.').next().unwrap_or("");
        if lower == host || lower == short {
            return None;
        }
    }
    Some(trimmed.to_string())
}

#[derive(Debug, Clone)]
pub struct TmuxBackend {
    socket: Option<String>,
    /// Extra environment for the tmux child processes (tests: `TMUX_TMPDIR`).
    child_env: Vec<(String, String)>,
}

impl TmuxBackend {
    pub fn new(socket: Option<String>) -> Self {
        Self { socket, child_env: Vec::new() }
    }

    /// Also set `key=value` in every tmux child process's environment.
    pub fn with_child_env(mut self, key: &str, value: &str) -> Self {
        self.child_env.push((key.into(), value.into()));
        self
    }

    /// Leading argv words for a command: `-L <socket> -f /dev/null` when an
    /// isolated server is configured, nothing otherwise.
    fn prefix(&self) -> Vec<String> {
        match &self.socket {
            Some(s) => vec!["-L".into(), s.clone(), "-f".into(), "/dev/null".into()],
            None => Vec::new(),
        }
    }

    /// Full tmux argv (after the `tmux` program name) for `args`.
    fn argv(&self, args: &[&str]) -> Vec<String> {
        let mut v = self.prefix();
        v.extend(args.iter().map(|s| s.to_string()));
        v
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new("tmux");
        cmd.args(args).stdin(Stdio::null()).kill_on_drop(true);
        if self.socket.is_some() {
            cmd.env_remove("TMUX").env_remove("TMUX_PANE");
        }
        for (k, v) in &self.child_env {
            cmd.env(k, v);
        }
        cmd
    }

    /// Run tmux; `Err` carries tmux's stderr on a non-zero exit.
    async fn run(&self, args: &[String]) -> Result<String> {
        let out = tokio::time::timeout(super::COMMAND_TIMEOUT, self.command(args).output())
            .await
            .map_err(|_| anyhow!("tmux timed out after {:?}", super::COMMAND_TIMEOUT))?
            .map_err(|e| anyhow!("cannot run tmux: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(anyhow!("tmux {}: {}", args.get(self.prefix().len()).map(String::as_str).unwrap_or(""), stderr.trim()))
        }
    }

    async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.run(&self.argv(args)).await
    }

    /// Claude Code's Shift+Enter (kitty `ESC[13;2u`) only reaches a pane if the
    /// server has extended keys on, advertises them, and re-encodes in csi-u
    /// form (so Option+Enter survives). Server options live until the server
    /// exits, so call this at boot and after every create. Idempotent:
    /// `set -as terminal-features` appends, so `xterm*:extkeys` is added only
    /// when missing. No server (or ancient tmux) is silently ignored.
    pub async fn ensure_extended_keys(&self) {
        let _ = self.ensure_extended_keys_inner().await;
    }

    async fn ensure_extended_keys_inner(&self) -> Result<()> {
        self.run_args(&["set-option", "-s", "extended-keys", "on"]).await?;
        let _ = self.run_args(&["set-option", "-s", "extended-keys-format", "csi-u"]).await;
        let shown = self.run_args(&["show-options", "-s", "terminal-features"]).await.unwrap_or_default();
        if !has_extkeys_feature(&shown) {
            self.run_args(&["set-option", "-as", "terminal-features", "xterm*:extkeys"]).await?;
        }
        Ok(())
    }

    /// True when a tmux server answers on this backend's socket.
    pub async fn server_running(&self) -> bool {
        self.run_args(&["list-sessions", "-F", "#{session_name}"]).await.is_ok()
    }

    /// The control-mode client: `tmux [-L s -f /dev/null] -C new-session -A -s
    /// garage-control cat`. It attaches to its own session, which has no
    /// `garage/` prefix (so no listing shows it) and is destroyed when the
    /// client goes away, so a stopped daemon leaves nothing behind. stdin and
    /// stdout are piped and the child dies with the handle (`kill_on_drop`).
    pub fn control_command(&self) -> Command {
        let mut argv = self.prefix();
        argv.extend(
            ["-C", "new-session", "-A", "-s", CONTROL_SESSION, "cat", ";", "set-option", "-t", CONTROL_SESSION, "destroy-unattached", "on"]
                .map(String::from),
        );
        let mut cmd = self.command(&argv);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        cmd
    }

    fn env_args(env: &[(String, String)]) -> Vec<String> {
        env.iter().flat_map(|(k, v)| ["-e".to_string(), format!("{k}={v}")]).collect()
    }
}

/// The JS `/xterm\*?[^\n]*extkeys/` test, line by line.
fn has_extkeys_feature(shown: &str) -> bool {
    shown.lines().any(|l| l.find("xterm").is_some_and(|i| l[i + 5..].contains("extkeys")))
}

#[async_trait]
impl SessionBackend for TmuxBackend {
    async fn init(&self) -> Result<()> {
        self.ensure_extended_keys().await;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<BackendSession>> {
        // tmux exits non-zero when no server runs: that means "no sessions".
        let Ok(out) = self
            .run_args(&[
                "ls",
                "-F",
                // `session_created` (epoch seconds) backs the UI's elapsed timer.
                "#{session_name}\t#{session_path}\t#{session_attached}\t#{session_created}",
            ])
            .await
        else {
            return Ok(Vec::new());
        };
        Ok(out
            .lines()
            .filter(|l| l.starts_with(GARAGE_PREFIX))
            .map(|line| {
                let mut f = line.split('\t');
                let id = f.next().unwrap_or("").to_string();
                let dir = f.next().unwrap_or("").to_string();
                let attached = f.next().is_some_and(|a| a != "0");
                // 0 or unparseable = unknown (the JS sent null)
                let created_at = f
                    .next()
                    .and_then(|c| c.parse::<f64>().ok())
                    .map(|s| (s * 1000.0) as i64)
                    .filter(|ms| *ms > 0);
                BackendSession { id, dir, attached, created_at }
            })
            .collect())
    }

    /// Live pane per garage session: cwd (`pane_current_path`, not the frozen
    /// `session_path`), title and root pid. One `list-panes -a` call. Only the
    /// first pane seen per session is kept.
    async fn panes(&self) -> Result<HashMap<String, PaneInfo>> {
        let Ok(out) = self
            .run_args(&["list-panes", "-a", "-F", "#{session_name}\t#{pane_current_path}\t#{pane_pid}\t#{pane_title}"])
            .await
        else {
            return Ok(HashMap::new());
        };
        let mut panes = HashMap::new();
        for line in out.lines().filter(|l| l.starts_with(GARAGE_PREFIX)) {
            let mut f = line.splitn(4, '\t');
            let name = f.next().unwrap_or("").to_string();
            let cwd = f.next().unwrap_or("").to_string();
            let pid = f.next().and_then(|p| p.parse::<u32>().ok());
            let title = f.next().unwrap_or("").to_string();
            panes.entry(name).or_insert(PaneInfo { cwd, title, pid });
        }
        Ok(panes)
    }

    async fn has(&self, id: &str) -> Result<bool> {
        Ok(self.run_args(&["has-session", "-t", &exact(id)]).await.is_ok())
    }

    async fn create(&self, id: &str, dir: &Path, cmd: &str, args: &[String], env: &[(String, String)]) -> Result<()> {
        let dir = dir.to_string_lossy();
        let mut argv = self.argv(&["new-session", "-d", "-s", id, "-c", &dir]);
        argv.extend(Self::env_args(env));
        argv.push(cmd.into());
        argv.extend(args.iter().cloned());
        self.run(&argv).await?;
        // new-session may have just started the server.
        self.ensure_extended_keys().await;
        // The wall draws its own title bars; tmux's status line is noise.
        // set-option rejects the `=` prefix; the fresh full name is unambiguous.
        let _ = self.run_args(&["set-option", "-t", id, "status", "off"]).await;
        Ok(())
    }

    /// In-place restart: `respawn-pane -k` keeps the tmux session identity.
    /// `-t` targets a PANE, so `=<id>:` (trailing colon) is required to mean
    /// "this session's active window's active pane".
    async fn respawn(&self, id: &str, dir: &Path, cmd: &str, args: &[String], env: &[(String, String)]) -> Result<()> {
        let dir = dir.to_string_lossy();
        let mut argv = self.argv(&["respawn-pane", "-k", "-c", &dir]);
        argv.extend(Self::env_args(env));
        argv.extend(["-t".into(), format!("{}:", exact(id)), cmd.into()]);
        argv.extend(args.iter().cloned());
        self.run(&argv).await?;
        self.ensure_extended_keys().await;
        Ok(())
    }

    async fn kill(&self, id: &str) -> Result<()> {
        self.run_args(&["kill-session", "-t", &exact(id)]).await.map(|_| ())
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.run_args(&["rename-session", "-t", &exact(from), to]).await.map(|_| ())
    }

    /// Last `lines` lines of the session's visible pane plus scrollback.
    async fn capture(&self, id: &str, lines: usize) -> Result<String> {
        let start = format!("-{lines}");
        self.run_args(&["capture-pane", "-p", "-t", &format!("{}:", exact(id)), "-S", &start]).await
    }

    fn attach_spec(&self, id: &str) -> AttachSpec {
        // Attaching needs a running server, so `-f` is never relevant here.
        let base = |rest: &[&str]| {
            let mut v = vec!["tmux".to_string()];
            if let Some(s) = &self.socket {
                v.extend(["-L".to_string(), s.clone()]);
            }
            v.extend(rest.iter().map(|s| s.to_string()));
            v
        };
        let attach = format!("={id}");
        AttachSpec::Tmux { argv: base(&["attach", "-t", &attach]), detach_argv: base(&["detach-client", "-t"]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestTmux;

    // ---- tmux.test.js: pure functions (JS test names in comments) ----

    #[test]
    fn attach_spec_carries_the_isolated_socket() {
        let AttachSpec::Tmux { argv, detach_argv } = TmuxBackend::new(Some("t1".into())).attach_spec("garage/ws/a") else {
            panic!("tmux backend must yield a Tmux spec");
        };
        assert_eq!(argv, ["tmux", "-L", "t1", "attach", "-t", "=garage/ws/a"]);
        assert_eq!(detach_argv, ["tmux", "-L", "t1", "detach-client", "-t"]);
    }

    #[test]
    fn attach_spec_uses_default_server_without_socket() {
        let AttachSpec::Tmux { argv, .. } = TmuxBackend::new(None).attach_spec("s") else { panic!() };
        assert_eq!(argv, ["tmux", "attach", "-t", "=s"]);
    }

    #[test]
    fn argv_is_unchanged_without_a_socket_and_isolated_with_one() {
        assert_eq!(TmuxBackend::new(None).argv(&["ls"]), ["ls"]);
        assert_eq!(TmuxBackend::new(Some("s".into())).argv(&["ls"]), ["-L", "s", "-f", "/dev/null", "ls"]);
    }

    // parsePaneLine splits the three fixed fields
    #[test]
    fn parse_pane_line_splits_the_three_fixed_fields() {
        assert_eq!(
            parse_pane_line("garage/ws/one\t/Users/me/repo\t✳ Refactoring the poller"),
            ("garage/ws/one".into(), "/Users/me/repo".into(), "✳ Refactoring the poller".into())
        );
    }

    // parsePaneLine rejoins an embedded separator into the title
    #[test]
    fn parse_pane_line_rejoins_an_embedded_separator_into_the_title() {
        assert_eq!(
            parse_pane_line("garage/ws/one\t/Users/me/repo\tbefore\tafter"),
            ("garage/ws/one".into(), "/Users/me/repo".into(), "before\tafter".into())
        );
    }

    // parsePaneLine handles an empty title
    #[test]
    fn parse_pane_line_handles_an_empty_title() {
        assert_eq!(
            parse_pane_line("garage/ws/one\t/Users/me/repo\t"),
            ("garage/ws/one".into(), "/Users/me/repo".into(), String::new())
        );
    }

    // normalizeTitle: empty/whitespace-only title is null
    #[test]
    fn normalize_title_empty_or_whitespace_is_none() {
        for t in [Some(""), Some("   "), None] {
            assert_eq!(normalize_title(t, Some("myhost")), None);
        }
    }

    // normalizeTitle: the machine hostname is null (case-insensitive)
    #[test]
    fn normalize_title_hostname_is_none_case_insensitive() {
        assert_eq!(normalize_title(Some("My-Mac"), Some("my-mac")), None);
        assert_eq!(normalize_title(Some("MY-MAC"), Some("my-mac")), None);
        assert_eq!(normalize_title(Some("my-mac"), Some("MY-MAC")), None);
    }

    // normalizeTitle: the short hostname (before the first dot) is null
    #[test]
    fn normalize_title_short_hostname_is_none() {
        assert_eq!(normalize_title(Some("my-mac"), Some("my-mac.local")), None);
        assert_eq!(normalize_title(Some("MY-MAC"), Some("my-mac.local")), None);
        assert_eq!(normalize_title(Some("my-mac.local"), Some("my-mac.local")), None);
    }

    // normalizeTitle: bare shell/process names are null
    #[test]
    fn normalize_title_bare_shell_names_are_none() {
        for name in ["zsh", "bash", "sh", "fish", "-zsh", "-bash", "-sh", "-fish", "tmux"] {
            assert_eq!(normalize_title(Some(name), Some("my-mac")), None, "{name}");
            assert_eq!(normalize_title(Some(&name.to_uppercase()), Some("my-mac")), None, "{name} upper");
        }
    }

    // normalizeTitle: a real title is returned trimmed
    #[test]
    fn normalize_title_real_title_is_trimmed() {
        assert_eq!(normalize_title(Some("✳ Refactoring the poller"), Some("my-mac")).as_deref(), Some("✳ Refactoring the poller"));
        assert_eq!(normalize_title(Some("  ✳ Refactoring the poller  "), Some("my-mac")).as_deref(), Some("✳ Refactoring the poller"));
    }

    // normalizeTitle: a title that merely contains the hostname is kept
    #[test]
    fn normalize_title_hostname_substring_is_kept() {
        assert_eq!(normalize_title(Some("ssh my-mac"), Some("my-mac")).as_deref(), Some("ssh my-mac"));
    }

    // normalizeTitle: works with no hostname supplied
    #[test]
    fn normalize_title_without_hostname_skips_that_check() {
        assert_eq!(normalize_title(Some("some title"), None).as_deref(), Some("some title"));
        assert_eq!(normalize_title(Some("zsh"), None), None);
    }

    #[test]
    fn session_id_helpers() {
        assert_eq!(session_id("ws", "one"), "garage/ws/one");
        assert_eq!(split_session_id("garage/ws/one"), ("ws".into(), "one".into()));
        assert!(is_valid_name("a-b-1") && !is_valid_name("A") && !is_valid_name("a b") && !is_valid_name(""));
    }

    #[test]
    fn extkeys_detection_matches_the_js_regex() {
        assert!(has_extkeys_feature("terminal-features[0] xterm*:clipboard\nterminal-features[1] xterm*:extkeys"));
        assert!(has_extkeys_feature("xterm-256color:extkeys"));
        assert!(!has_extkeys_feature("terminal-features[0] xterm*:clipboard"));
        assert!(!has_extkeys_feature("extkeys only\nxterm on another line"));
    }

    // ---- real tmux, isolated socket (TestTmux) ----

    fn sleeper() -> Vec<String> {
        vec!["60".into()]
    }

    #[tokio::test]
    async fn lifecycle_create_list_has_rename_kill() {
        let t = TestTmux::new();
        let b = t.backend();
        assert!(b.list().await.unwrap().is_empty(), "no server yet means no sessions");
        assert!(!b.has("garage/ws/a").await.unwrap());

        b.create("garage/ws/a", t.dir(), "sleep", &sleeper(), &[]).await.unwrap();
        // A non-garage session on the same server must not be listed.
        t.tmux(&["new-session", "-d", "-s", "other", "sleep", "60"]);

        let list = b.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "garage/ws/a");
        assert_eq!(Path::new(&list[0].dir).canonicalize().unwrap(), t.dir().canonicalize().unwrap());
        assert!(!list[0].attached);
        assert!(list[0].created_at.is_some_and(|c| c > 1_600_000_000_000));
        assert!(b.has("garage/ws/a").await.unwrap());
        // `=` exact matching: a prefix is not a session.
        assert!(!b.has("garage/ws").await.unwrap());

        b.rename("garage/ws/a", "garage/ws/b").await.unwrap();
        assert!(!b.has("garage/ws/a").await.unwrap());
        assert!(b.has("garage/ws/b").await.unwrap());

        b.kill("garage/ws/b").await.unwrap();
        assert!(b.list().await.unwrap().is_empty());
        assert!(b.kill("garage/ws/b").await.is_err());
    }

    #[tokio::test]
    async fn create_turns_status_off_and_enables_extended_keys() {
        let t = TestTmux::new();
        let b = t.backend();
        b.create("garage/ws/a", t.dir(), "sleep", &sleeper(), &[]).await.unwrap();
        assert_eq!(t.tmux(&["show-options", "-t", "garage/ws/a", "-v", "status"]).trim(), "off");
        assert_eq!(t.tmux(&["show-options", "-sv", "extended-keys"]).trim(), "on");
        assert!(has_extkeys_feature(&t.tmux(&["show-options", "-s", "terminal-features"])));
        // Idempotent: a second create does not append another feature.
        b.create("garage/ws/b", t.dir(), "sleep", &sleeper(), &[]).await.unwrap();
        let feats = t.tmux(&["show-options", "-s", "terminal-features"]);
        assert_eq!(feats.matches("extkeys").count(), 1, "{feats}");
    }

    #[tokio::test]
    async fn create_passes_env_and_args_to_the_pane() {
        let t = TestTmux::new();
        let b = t.backend();
        let args = vec!["-c".to_string(), "echo $GARAGE_T $0 > out.txt; sleep 60".to_string(), "x".to_string()];
        b.create("garage/ws/e", t.dir(), "sh", &args, &[("GARAGE_T".into(), "hello".into())]).await.unwrap();
        let out = t.dir().join("out.txt");
        t.wait_for(|| std::fs::read_to_string(&out).is_ok_and(|s| !s.is_empty())).await;
        assert_eq!(std::fs::read_to_string(&out).unwrap().trim(), "hello x");
    }

    #[tokio::test]
    async fn create_fails_loudly_on_a_duplicate_name() {
        let t = TestTmux::new();
        let b = t.backend();
        b.create("garage/ws/a", t.dir(), "sleep", &sleeper(), &[]).await.unwrap();
        let e = b.create("garage/ws/a", t.dir(), "sleep", &sleeper(), &[]).await.unwrap_err();
        assert!(e.to_string().contains("duplicate"), "{e}");
    }

    #[tokio::test]
    async fn respawn_keeps_the_session_and_swaps_the_process() {
        let t = TestTmux::new();
        let b = t.backend();
        b.create("garage/ws/r", t.dir(), "sleep", &sleeper(), &[]).await.unwrap();
        let before = b.panes().await.unwrap()["garage/ws/r"].pid.unwrap();
        let created = b.list().await.unwrap()[0].created_at;
        assert!(created.is_some());
        b.respawn("garage/ws/r", t.dir(), "sleep", &["61".into()], &[]).await.unwrap();
        let after = b.panes().await.unwrap()["garage/ws/r"].pid.unwrap();
        assert_ne!(before, after, "a new process runs in the same pane");
        assert_eq!(b.list().await.unwrap()[0].created_at, created, "same tmux session");
        assert!(b.respawn("garage/ws/missing", t.dir(), "sleep", &sleeper(), &[]).await.is_err());
    }

    #[tokio::test]
    async fn panes_reports_cwd_title_and_pid_for_garage_sessions_only() {
        let t = TestTmux::new();
        let b = t.backend();
        assert!(b.panes().await.unwrap().is_empty());
        b.create("garage/ws/p", t.dir(), "sleep", &sleeper(), &[]).await.unwrap();
        t.tmux(&["new-session", "-d", "-s", "other", "sleep", "60"]);
        let panes = b.panes().await.unwrap();
        assert_eq!(panes.len(), 1);
        let p = &panes["garage/ws/p"];
        assert_eq!(Path::new(&p.cwd).canonicalize().unwrap(), t.dir().canonicalize().unwrap());
        assert!(p.pid.is_some_and(|p| p > 1));
    }

    #[tokio::test]
    async fn a_session_with_two_panes_reports_the_first_pane() {
        let t = TestTmux::new();
        let b = t.backend();
        let first = t.dir().join("first");
        let second = t.dir().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        b.create("garage/ws/two", &first, "sleep", &sleeper(), &[]).await.unwrap();
        let before = b.panes().await.unwrap()["garage/ws/two"].clone();
        t.tmux(&["split-window", "-d", "-t", "garage/ws/two:", "-c", second.to_str().unwrap(), "sleep", "60"]);
        let panes = b.panes().await.unwrap();
        assert_eq!(panes.len(), 1);
        let p = &panes["garage/ws/two"];
        assert_eq!(Path::new(&p.cwd).canonicalize().unwrap(), first.canonicalize().unwrap(), "first pane, not the split");
        assert_eq!(p.pid, before.pid);
    }

    #[tokio::test]
    async fn capture_returns_pane_text() {
        let t = TestTmux::new();
        let b = t.backend();
        b.create("garage/ws/c", t.dir(), "sh", &["-c".into(), "echo marker-xyz; sleep 60".into()], &[]).await.unwrap();
        t.wait_for_async(|| async { b.capture("garage/ws/c", 50).await.is_ok_and(|s| s.contains("marker-xyz")) }).await;
        assert!(b.capture("garage/ws/none", 5).await.is_err());
    }

    // sessions.title.test.js, backend part: "a live session's set pane title
    // surfaces normalized" and "a live session with no set title has null".
    #[tokio::test]
    async fn live_session_set_pane_title_surfaces_normalized() {
        let t = TestTmux::new();
        let b = t.backend();
        b.create("garage/ws/live", t.dir(), "sleep", &sleeper(), &[]).await.unwrap();
        t.tmux(&["select-pane", "-t", "garage/ws/live", "-T", "  ✳ Refactoring the poller  "]);
        let title = b.panes().await.unwrap()["garage/ws/live"].title.clone();
        assert_eq!(normalize_title(Some(&title), Some("my-mac")).as_deref(), Some("✳ Refactoring the poller"));
    }

    #[tokio::test]
    async fn live_session_with_default_title_normalizes_to_none() {
        let t = TestTmux::new();
        let b = t.backend();
        b.create("garage/ws/default-title", t.dir(), "sleep", &sleeper(), &[]).await.unwrap();
        let title = b.panes().await.unwrap()["garage/ws/default-title"].title.clone();
        let host = gethostname::gethostname().to_string_lossy().into_owned();
        assert_eq!(normalize_title(Some(&title), Some(&host)), None, "default title {title:?}");
    }
}
