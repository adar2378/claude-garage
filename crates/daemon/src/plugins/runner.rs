//! Running one plugin command (design section 4): argv only, no shell, its own
//! process group, context on stdin and in the environment, output captured to
//! the plugin's log, a timeout that terminates the whole tree.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::os::process::{own_group, signal_group};

/// SIGTERM, then SIGKILL after this long.
pub const KILL_GRACE: Duration = Duration::from_secs(2);
/// After the child exits, how long leftover pipe holders may keep the output open.
const DRAIN_GRACE: Duration = Duration::from_millis(500);
pub const LOG_MAX_BYTES: u64 = 1024 * 1024;
const MAX_LINE: usize = 8 * 1024;

/// `<id>.log` plus one rotated `<id>.log.1`.
pub struct PluginLog {
    path: PathBuf,
    max: u64,
    lock: tokio::sync::Mutex<()>,
}

impl PluginLog {
    pub fn new(path: PathBuf) -> Arc<Self> {
        Self::with_max(path, LOG_MAX_BYTES)
    }

    pub fn with_max(path: PathBuf, max: u64) -> Arc<Self> {
        Arc::new(Self { path, max, lock: tokio::sync::Mutex::new(()) })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Append `[prefix] line`, rotating first when the file would pass the cap.
    pub async fn append(&self, prefix: &str, line: &str) {
        use std::io::Write;
        let _g = self.lock.lock().await;
        let entry = format!("[{prefix}] {line}\n");
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let size = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if size > 0 && size + entry.len() as u64 > self.max {
            let mut rotated = self.path.clone().into_os_string();
            rotated.push(".1");
            let _ = std::fs::rename(&self.path, PathBuf::from(rotated));
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = f.write_all(entry.as_bytes());
        }
    }
}

/// Pids of live runs per plugin, so teardown can kill their groups.
#[derive(Default)]
pub struct Running(Mutex<HashMap<String, HashMap<u64, u32>>>);

impl Running {
    fn add(&self, plugin: &str, run_id: u64, pid: u32) {
        self.0.lock().unwrap().entry(plugin.to_owned()).or_default().insert(run_id, pid);
    }

    fn remove(&self, plugin: &str, run_id: u64) {
        let mut map = self.0.lock().unwrap();
        if let Some(runs) = map.get_mut(plugin) {
            runs.remove(&run_id);
            if runs.is_empty() {
                map.remove(plugin);
            }
        }
    }

    pub fn count(&self, plugin: &str) -> usize {
        self.0.lock().unwrap().get(plugin).map_or(0, HashMap::len)
    }

    /// SIGTERM every group of `plugin` now, SIGKILL the stragglers after
    /// [`KILL_GRACE`]. Returns how many runs were signalled.
    pub fn terminate(self: &Arc<Self>, plugin: &str) -> usize {
        let runs: Vec<(u64, u32)> =
            self.0.lock().unwrap().get(plugin).map(|r| r.iter().map(|(a, b)| (*a, *b)).collect()).unwrap_or_default();
        for (_, pid) in &runs {
            signal_group(*pid, false);
        }
        if !runs.is_empty() {
            let (this, plugin) = (self.clone(), plugin.to_owned());
            let n = runs.len();
            tokio::spawn(async move {
                tokio::time::sleep(KILL_GRACE).await;
                let map = this.0.lock().unwrap();
                for (run_id, pid) in runs {
                    if map.get(&plugin).and_then(|r| r.get(&run_id)) == Some(&pid) {
                        signal_group(pid, true);
                    }
                }
            });
            return n;
        }
        0
    }
}

/// Everything needed to start one run.
pub struct RunSpec {
    pub plugin: String,
    pub run_id: u64,
    /// Log prefix label, `action:refresh`, `event:session.closed`, ...
    pub label: String,
    /// Substituted argv; the first element is the program.
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    /// Added to the daemon's own environment.
    pub env: Vec<(String, String)>,
    pub stdin: Vec<u8>,
    /// `None` runs until it exits.
    pub timeout: Option<Duration>,
    pub log: Arc<PluginLog>,
    pub running: Arc<Running>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Success,
    /// Non-zero exit (`None`: killed by a signal).
    Failed(Option<i32>),
    TimedOut(Duration),
    SpawnFailed(String),
}

impl RunOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, RunOutcome::Success)
    }

    /// One line for the toast and the log.
    pub fn reason(&self) -> String {
        match self {
            RunOutcome::Success => "ok".into(),
            RunOutcome::Failed(Some(c)) => format!("exited with code {c}"),
            RunOutcome::Failed(None) => "was killed by a signal".into(),
            RunOutcome::TimedOut(d) => format!("timed out after {}s", d.as_secs()),
            RunOutcome::SpawnFailed(e) => format!("could not start: {e}"),
        }
    }
}

/// The seam tests replace with a recorder.
#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, spec: RunSpec) -> RunOutcome;
}

pub struct ProcessRunner;

#[async_trait]
impl Runner for ProcessRunner {
    async fn run(&self, spec: RunSpec) -> RunOutcome {
        let prefix = format!("{} {}", spec.run_id, spec.label);
        let Some((program, args)) = spec.argv.split_first() else {
            return RunOutcome::SpawnFailed("empty command".into());
        };
        let mut cmd = Command::new(crate::plugins::resolve_program(program, &spec.cwd));
        cmd.args(args)
            .current_dir(&spec.cwd)
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        own_group(&mut cmd);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                spec.log.append(&prefix, &format!("could not start {program}: {e}")).await;
                return RunOutcome::SpawnFailed(e.to_string());
            }
        };
        let pid = child.id().unwrap_or(0);
        spec.running.add(&spec.plugin, spec.run_id, pid);

        if let Some(mut stdin) = child.stdin.take() {
            let body = spec.stdin.clone();
            // A plugin that never reads stdin must not block the run.
            tokio::spawn(async move {
                let _ = stdin.write_all(&body).await;
            });
        }
        let readers: Vec<_> = [
            child.stdout.take().map(|s| tokio::spawn(pump(s, spec.log.clone(), prefix.clone()))),
            child.stderr.take().map(|s| tokio::spawn(pump(s, spec.log.clone(), prefix.clone()))),
        ]
        .into_iter()
        .flatten()
        .collect();

        let waited = match spec.timeout {
            Some(limit) => match tokio::time::timeout(limit, child.wait()).await {
                Ok(r) => Ok(r),
                Err(_) => Err(limit),
            },
            None => Ok(child.wait().await),
        };
        let outcome = match waited {
            Ok(Ok(status)) if status.success() => RunOutcome::Success,
            Ok(Ok(status)) => RunOutcome::Failed(status.code()),
            Ok(Err(e)) => RunOutcome::SpawnFailed(e.to_string()),
            Err(limit) => {
                signal_group(pid, false);
                if tokio::time::timeout(KILL_GRACE, child.wait()).await.is_err() {
                    signal_group(pid, true);
                    let _ = child.wait().await;
                }
                RunOutcome::TimedOut(limit)
            }
        };
        for r in readers {
            if tokio::time::timeout(DRAIN_GRACE, r).await.is_err() {
                // A grandchild still holds the pipe; the reader is dropped.
            }
        }
        spec.running.remove(&spec.plugin, spec.run_id);
        if !outcome.is_success() {
            spec.log.append(&prefix, &format!("run failed: {}", outcome.reason())).await;
        }
        outcome
    }
}

/// Copy a pipe into the log line by line; an over-long line is cut.
async fn pump<R: AsyncReadExt + Unpin>(mut r: R, log: Arc<PluginLog>, prefix: String) {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            log.append(&prefix, String::from_utf8_lossy(&line[..line.len() - 1]).trim_end_matches('\r')).await;
        }
        if buf.len() >= MAX_LINE {
            let line = std::mem::take(&mut buf);
            log.append(&prefix, &format!("{} [cut]", String::from_utf8_lossy(&line))).await;
        }
    }
    if !buf.is_empty() {
        log.append(&prefix, &String::from_utf8_lossy(&buf)).await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn spec(dir: &std::path::Path, argv: Vec<String>, timeout: Option<Duration>) -> (RunSpec, Arc<PluginLog>, Arc<Running>) {
        let log = PluginLog::new(dir.join("logs/p.log"));
        let running = Arc::new(Running::default());
        let s = RunSpec {
            plugin: "p".into(),
            run_id: 7,
            label: "action:t".into(),
            argv,
            cwd: dir.to_path_buf(),
            env: vec![("GARAGE_PLUGIN_ID".into(), "p".into())],
            stdin: b"{\"hello\":1}".to_vec(),
            timeout,
            log: log.clone(),
            running: running.clone(),
        };
        (s, log, running)
    }

    #[tokio::test]
    async fn a_real_script_gets_stdin_env_and_cwd_and_its_output_lands_in_the_log() {
        let d = tempfile::tempdir().unwrap();
        let sh = script(d.path(), "run.sh", "cat > got-stdin.json\necho \"id=$GARAGE_PLUGIN_ID cwd=$(pwd)\"\necho oops >&2\nexit 0");
        let (s, log, running) = spec(d.path(), vec![sh], Some(Duration::from_secs(10)));
        assert_eq!(ProcessRunner.run(s).await, RunOutcome::Success);
        assert_eq!(std::fs::read_to_string(d.path().join("got-stdin.json")).unwrap(), "{\"hello\":1}");
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(text.contains("[7 action:t] id=p cwd="), "{text}");
        assert!(text.contains("[7 action:t] oops"), "stderr is captured too: {text}");
        assert_eq!(running.count("p"), 0, "finished runs are unregistered");
    }

    #[tokio::test]
    async fn argv_is_never_a_shell_metacharacters_stay_arguments() {
        let d = tempfile::tempdir().unwrap();
        let sh = script(d.path(), "args.sh", "printf '%s|' \"$@\" > args.txt");
        let (s, _log, _r) = spec(d.path(), vec![sh, "a; touch pwned".into(), "$(touch pwned)".into(), "`x`".into()], None);
        assert_eq!(ProcessRunner.run(s).await, RunOutcome::Success);
        assert_eq!(std::fs::read_to_string(d.path().join("args.txt")).unwrap(), "a; touch pwned|$(touch pwned)|`x`|");
        assert!(!d.path().join("pwned").exists());
    }

    #[tokio::test]
    async fn failures_report_the_exit_code_and_a_missing_program_a_spawn_error() {
        let d = tempfile::tempdir().unwrap();
        let sh = script(d.path(), "fail.sh", "echo boom; exit 3");
        let (s, log, _r) = spec(d.path(), vec![sh], None);
        let out = ProcessRunner.run(s).await;
        assert_eq!(out, RunOutcome::Failed(Some(3)));
        assert_eq!(out.reason(), "exited with code 3");
        assert!(std::fs::read_to_string(log.path()).unwrap().contains("run failed: exited with code 3"));
        let (s, _l, _r) = spec(d.path(), vec![d.path().join("nope").to_string_lossy().into_owned()], None);
        assert!(matches!(ProcessRunner.run(s).await, RunOutcome::SpawnFailed(_)));
    }

    #[tokio::test]
    async fn a_timeout_terminates_the_whole_process_group_including_children() {
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("child.pid");
        // The shell backgrounds a sleeper (a grandchild) and waits on it.
        let sh = script(d.path(), "hang.sh", &format!("sleep 30 &\necho $! > {}\nwait", pidfile.display()));
        let (s, _log, running) = spec(d.path(), vec![sh], Some(Duration::from_millis(1500)));
        let started = std::time::Instant::now();
        let out = ProcessRunner.run(s).await;
        assert!(matches!(out, RunOutcome::TimedOut(_)), "{out:?}");
        assert!(started.elapsed() < Duration::from_secs(8), "TERM must not wait for the 30 s sleeper");
        let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        // the grandchild is gone (kill -0 fails once it has exited)
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(std::time::Instant::now() < end, "grandchild {pid} survived the group kill");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(running.count("p"), 0);
    }

    #[tokio::test]
    async fn terminate_kills_running_groups_for_a_plugin_only() {
        let d = tempfile::tempdir().unwrap();
        let sh = script(d.path(), "long.sh", "sleep 30");
        let (s, _log, running) = spec(d.path(), vec![sh], None);
        let r2 = running.clone();
        let run = tokio::spawn(async move { ProcessRunner.run(s).await });
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while r2.count("p") == 0 {
            assert!(std::time::Instant::now() < end);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(r2.terminate("other"), 0);
        assert_eq!(r2.terminate("p"), 1);
        let out = tokio::time::timeout(Duration::from_secs(5), run).await.expect("run ends").unwrap();
        assert_eq!(out, RunOutcome::Failed(None), "killed by SIGTERM");
    }

    #[tokio::test]
    async fn the_log_rotates_to_one_old_file_at_the_size_cap() {
        let d = tempfile::tempdir().unwrap();
        let log = PluginLog::with_max(d.path().join("p.log"), 200);
        for i in 0..40 {
            log.append("1 t", &format!("line number {i:02}")).await;
        }
        let cur = std::fs::read_to_string(d.path().join("p.log")).unwrap();
        let old = std::fs::read_to_string(d.path().join("p.log.1")).unwrap();
        assert!(cur.len() <= 200 && old.len() <= 200, "{} {}", cur.len(), old.len());
        assert!(cur.contains("line number 39"));
        assert!(!d.path().join("p.log.2").exists(), "two files only");
    }
}
