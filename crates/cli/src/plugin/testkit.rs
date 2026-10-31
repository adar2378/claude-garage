//! Fakes shared by the plugin CLI tests. Nothing here touches the network, a
//! real daemon or the user's `~/.garage`.
#![allow(dead_code)] // some fakes are used only by the unix-gated install tests

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use garage_daemon::plugins::admin::{Registration, register};
use garage_daemon::plugins::layout::{Entry, MANIFEST_FILE, Paths, PluginsFile, Source};

use super::fetch::{Builder, Fetch};
use super::install::Ui;

pub fn toml(id: &str, extra: &str) -> String {
    format!(
        "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi_version = 1\nplatforms = [\"macos\",\"linux\",\"windows\"]\n{extra}\n"
    )
}

/// Write a linked plugin's folder and register it (granted its own
/// permissions when `approved`).
pub fn link_plugin(paths: &Paths, root: &Path, id: &str, extra: &str, approved: bool) {
    let dir = root.join("linked").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MANIFEST_FILE), toml(id, extra)).unwrap();
    let source = Source::Link {
        path: dir.to_string_lossy().into_owned(),
    };
    if approved {
        let granted = garage_plugin::parse(&toml(id, extra))
            .unwrap()
            .permission_names();
        register(
            paths,
            &Registration {
                id: id.into(),
                source,
                granted_permissions: granted,
                manifest_sha256: None,
                installed_at: None,
            },
        )
        .unwrap();
    } else {
        PluginsFile::update(paths, |f| {
            f.plugins.push(Entry::new(id, source));
            (true, ())
        })
        .unwrap();
        // a plugin the user pointed us at has a folder but no approval
        let _ = id;
    }
}

/// A scripted terminal.
pub struct FakeUi {
    pub out: String,
    pub interactive: bool,
    pub answers: Vec<bool>,
    pub asked: Vec<String>,
    /// Runs when a question is asked (to assert what has happened so far).
    pub on_confirm: Option<Box<dyn Fn()>>,
}

impl FakeUi {
    pub fn new(interactive: bool, answers: &[bool]) -> Self {
        Self {
            out: String::new(),
            interactive,
            answers: answers.to_vec(),
            asked: vec![],
            on_confirm: None,
        }
    }
}

impl Ui for FakeUi {
    fn say(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push('\n');
    }
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn confirm(&mut self, q: &str) -> bool {
        if let Some(f) = &self.on_confirm {
            f();
        }
        self.asked.push(q.to_owned());
        if self.answers.is_empty() {
            false
        } else {
            self.answers.remove(0)
        }
    }
}

/// Serves canned bodies by URL and records what was asked for.
#[derive(Default)]
pub struct FakeFetch {
    pub files: HashMap<String, Vec<u8>>,
    pub calls: Mutex<Vec<String>>,
}

impl FakeFetch {
    pub fn new(files: &[(&str, &[u8])]) -> Self {
        Self {
            files: files
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_vec()))
                .collect(),
            calls: Mutex::new(vec![]),
        }
    }
}

impl Fetch for FakeFetch {
    fn get(&self, url: &str, _max: usize) -> Result<Vec<u8>, String> {
        self.calls.lock().unwrap().push(url.to_owned());
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| format!("404 {url}"))
    }
}

/// Records build commands instead of running them; can be told to fail.
#[derive(Clone, Default)]
pub struct RecordingBuilder {
    pub calls: Arc<Mutex<Vec<Vec<String>>>>,
    pub fail: Arc<Mutex<bool>>,
    /// A file created in the cwd by each run, to make "built" observable.
    pub produces: Option<String>,
}

impl Builder for RecordingBuilder {
    fn run(&self, argv: &[String], cwd: &Path, _t: Duration, _log: &Path) -> Result<(), String> {
        self.calls.lock().unwrap().push(argv.to_vec());
        if *self.fail.lock().unwrap() {
            return Err(format!("build step {} failed (exit 1)", argv.join(" ")));
        }
        if let Some(f) = &self.produces {
            std::fs::write(cwd.join(f), "built").map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// A git repository on disk to clone through `file://`.
#[cfg(unix)]
pub mod repo {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    pub fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// `<base>/<owner>/<repo>` as a repository with `files` committed.
    pub fn make(base: &Path, owner_repo: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = base.join(owner_repo);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        write(&dir, files);
        commit(&dir, "first");
        dir
    }

    pub fn write(dir: &Path, files: &[(&str, &str)]) {
        for (name, body) in files {
            let p = dir.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
    }

    pub fn commit(dir: &Path, msg: &str) -> String {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
        git(dir, &["rev-parse", "HEAD"])
    }
}
