//! `SessionBackend` over per-session host processes (the Windows backend;
//! `GARAGE_BACKEND=host` forces it anywhere for testing).
//!
//! Each session is one detached `claude-garage host` process (see
//! `garage-host`). The registry directory `<garage dir>/hosts/` is the list of
//! sessions; a record counts only while its host answers on the loopback port.
//! Hosts outlive the daemon, the TUI and the terminal, but not a reboot: a
//! record whose port refuses connections is stale and is deleted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use garage_host::proto::{self, Info};
use garage_host::registry::{self, Entry};
use garage_protocol::AttachSpec;

use super::tmux::GARAGE_PREFIX;
use super::{BackendSession, PaneInfo, SessionBackend};

/// How long `create` waits for a new host to publish its record.
const START_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `kill` waits for the host to exit after the shutdown frame.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// What probing a registry record found.
#[derive(Debug)]
enum Probe {
    Alive(Info),
    /// The port is open but did not answer in time (a host busy writing to a
    /// slow client). Not stale: never delete its record.
    Busy,
    /// Nothing listens there any more.
    Dead,
}

/// Connection-level failures mean nobody is listening; anything else (a read
/// timeout, a reset mid-reply) means something is there.
fn is_gone(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(e.kind(), ConnectionRefused | AddrNotAvailable | NotFound | InvalidInput)
}

fn probe(entry: &Entry) -> Probe {
    match proto::info(&entry.addr(), &entry.token) {
        Ok(info) => Probe::Alive(info),
        Err(e) if is_gone(&e) => Probe::Dead,
        // A connect timeout also lands here; a host that cannot even accept in
        // 2s is stuck, but deleting its record would orphan a live process.
        Err(_) => Probe::Busy,
    }
}

/// Run `f` off the async threads: every call here is blocking socket or file I/O.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| anyhow!("host task failed: {e}"))
}

#[derive(Debug, Clone)]
pub struct HostBackend {
    hosts_dir: PathBuf,
    /// Program and leading args that start a host: `[claude-garage, host]`.
    host_program: Vec<String>,
}

impl HostBackend {
    pub fn new(hosts_dir: PathBuf, host_program: Vec<String>) -> Self {
        Self { hosts_dir, host_program }
    }

    pub fn hosts_dir(&self) -> &Path {
        &self.hosts_dir
    }

    /// Every garage record with its probe result; stale records are deleted.
    async fn live(&self) -> Result<Vec<(Entry, Option<Info>)>> {
        let dir = self.hosts_dir.clone();
        blocking(move || {
            let entries: Vec<Entry> =
                registry::read_all(&dir).into_iter().filter(|e| e.id.starts_with(GARAGE_PREFIX)).collect();
            // Probe in parallel: one stuck host must not delay the others.
            let results: Vec<Probe> = std::thread::scope(|s| {
                let handles: Vec<_> = entries.iter().map(|e| s.spawn(move || probe(e))).collect();
                handles.into_iter().map(|h| h.join().unwrap_or(Probe::Busy)).collect()
            });
            let mut out = Vec::new();
            for (entry, result) in entries.into_iter().zip(results) {
                match result {
                    Probe::Alive(info) => out.push((entry, Some(info))),
                    Probe::Busy => out.push((entry, None)),
                    Probe::Dead => registry::remove(&dir, &entry.id),
                }
            }
            Ok(out)
        })
        .await?
    }

    /// The live record for `id`, if any.
    async fn find(&self, id: &str) -> Result<Option<Entry>> {
        let (dir, id) = (self.hosts_dir.clone(), id.to_owned());
        blocking(move || {
            let Some(entry) = registry::read(&dir, &id) else { return None };
            match probe(&entry) {
                Probe::Alive(_) | Probe::Busy => Some(entry),
                Probe::Dead => {
                    registry::remove(&dir, &id);
                    None
                }
            }
        })
        .await
    }

    async fn start(&self, id: &str, dir: &Path, cmd: &str, args: &[String], env: &[(String, String)]) -> Result<()> {
        let (program, lead) = self.host_program.split_first().ok_or_else(|| anyhow!("no host program configured"))?;
        registry::ensure_dir(&self.hosts_dir).context("cannot create the hosts directory")?;
        let log_path = registry::log_path(&self.hosts_dir, id);
        let log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).context("cannot open the host log")?;

        let mut c = tokio::process::Command::new(program);
        c.args(lead)
            .args(["--id", id, "--dir"])
            .arg(dir)
            .arg("--hosts-dir")
            .arg(&self.hosts_dir)
            .arg("--")
            .arg(cmd)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(false);
        detach(&mut c);
        let mut child = c.spawn().with_context(|| format!("cannot start the session host ({program})"))?;
        let pid = child.id();

        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            let (d, i) = (self.hosts_dir.clone(), id.to_owned());
            if blocking(move || registry::read(&d, &i)).await?.is_some_and(|e| Some(e.pid) == pid) {
                return Ok(());
            }
            if let Ok(Some(status)) = child.try_wait() {
                bail!("the session host exited at start ({status}): {}", log_tail(&log_path));
            }
            if Instant::now() > deadline {
                bail!("the session host did not start in {START_TIMEOUT:?}: {}", log_tail(&log_path));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Ask the host to exit and wait until its record is gone.
    async fn stop(&self, entry: Entry) -> Result<()> {
        let dir = self.hosts_dir.clone();
        blocking(move || {
            if let Err(e) = proto::shutdown(&entry.addr(), &entry.token) {
                if is_gone(&e) {
                    registry::remove(&dir, &entry.id);
                    bail!("can't find session: {}", entry.id);
                }
                // Busy or reset mid-reply: the frame may still have landed.
            }
            let deadline = Instant::now() + STOP_TIMEOUT;
            while registry::read(&dir, &entry.id).is_some() {
                if Instant::now() > deadline {
                    bail!("the session host for {} did not exit in {STOP_TIMEOUT:?}", entry.id);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(())
        })
        .await?
    }
}

/// Start `c` detached from the daemon: it must survive the daemon and the terminal.
fn detach(c: &mut tokio::process::Command) {
    #[cfg(unix)]
    c.process_group(0);
    #[cfg(windows)]
    {
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
}

fn log_tail(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let tail: Vec<&str> = text.lines().rev().take(5).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    if tail.is_empty() { format!("(see {})", path.display()) } else { tail.join(" | ") }
}

#[async_trait]
impl SessionBackend for HostBackend {
    async fn init(&self) -> Result<()> {
        registry::ensure_dir(&self.hosts_dir).with_context(|| format!("cannot create {}", self.hosts_dir.display()))
    }

    async fn list(&self) -> Result<Vec<BackendSession>> {
        Ok(self
            .live()
            .await?
            .into_iter()
            .map(|(entry, info)| BackendSession {
                id: entry.id,
                dir: entry.dir,
                attached: info.is_some_and(|i| i.attached > 0),
                created_at: Some(entry.created_at as i64).filter(|ms| *ms > 0),
            })
            .collect())
    }

    async fn panes(&self) -> Result<HashMap<String, PaneInfo>> {
        Ok(self
            .live()
            .await?
            .into_iter()
            .map(|(entry, info)| {
                let title = info.map(|i| i.title).unwrap_or(entry.title);
                (entry.id, PaneInfo { cwd: entry.dir, title, pid: Some(entry.child_pid).filter(|p| *p > 0) })
            })
            .collect())
    }

    async fn has(&self, id: &str) -> Result<bool> {
        Ok(self.find(id).await?.is_some())
    }

    async fn create(&self, id: &str, dir: &Path, cmd: &str, args: &[String], env: &[(String, String)]) -> Result<()> {
        if self.find(id).await?.is_some() {
            bail!("duplicate session: {id}");
        }
        self.start(id, dir, cmd, args, env).await
    }

    /// A host cannot swap its command in place, so this stops the host and
    /// starts a new one under the same id. Unlike tmux, `created_at` changes.
    async fn respawn(&self, id: &str, dir: &Path, cmd: &str, args: &[String], env: &[(String, String)]) -> Result<()> {
        let entry = self.find(id).await?.ok_or_else(|| anyhow!("can't find session: {id}"))?;
        self.stop(entry).await?;
        self.start(id, dir, cmd, args, env).await
    }

    async fn kill(&self, id: &str) -> Result<()> {
        let entry = self.find(id).await?.ok_or_else(|| anyhow!("can't find session: {id}"))?;
        self.stop(entry).await
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let entry = self.find(from).await?.ok_or_else(|| anyhow!("can't find session: {from}"))?;
        if self.find(to).await?.is_some() {
            bail!("duplicate session: {to}");
        }
        let to = to.to_owned();
        blocking(move || proto::rename(&entry.addr(), &entry.token, &to).map_err(|e| anyhow!("rename failed: {e}"))).await?
    }

    async fn capture(&self, id: &str, lines: usize) -> Result<String> {
        let entry = self.find(id).await?.ok_or_else(|| anyhow!("can't find session: {id}"))?;
        let n = u32::try_from(lines).unwrap_or(u32::MAX);
        blocking(move || proto::capture_text(&entry.addr(), &entry.token, n).map_err(|e| anyhow!("capture failed: {e}"))).await?
    }

    /// Only the id: the token stays on disk (see [`AttachSpec::Host`]). An
    /// empty id means "no host for this session" (the route answers 404).
    fn attach_spec(&self, id: &str) -> AttachSpec {
        let known = registry::read(&self.hosts_dir, id).is_some();
        AttachSpec::Host { id: if known { id.to_owned() } else { String::new() } }
    }
}
