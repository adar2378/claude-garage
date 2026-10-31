//! Getting a plugin's binaries: a release asset verified against the
//! release's checksums file, or the manifest's `[[build]]` commands. Both sit
//! behind seams (`Fetch`, `Builder`) so tests never touch the network or run
//! a real toolchain.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use garage_plugin::argv::{self, Vars};
use garage_plugin::manifest::{sha256_hex, Build, Manifest};

/// Largest release asset or checksums file accepted.
pub const MAX_ASSET_BYTES: usize = 200 * 1024 * 1024;

pub trait Fetch {
    /// GET `url` and return the body; any non-2xx status is an error.
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, String>;
}

/// HTTPS through the system `curl` (present on macOS, Linux and current
/// Windows), which brings the platform's certificate store with it. Only
/// https is allowed, including across redirects.
pub struct CurlFetch;

impl Fetch for CurlFetch {
    fn get(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
        // `-q` first: ignore any ~/.curlrc (it could point curl anywhere).
        let out = Command::new("curl")
            .arg("-q")
            .args(["--fail", "--silent", "--show-error", "--location", "--proto", "=https", "--proto-redir", "=https", "--max-time", "300"])
            .args(["--max-filesize", &max_bytes.to_string(), "--", url])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run curl: {e}"))?;
        if !out.status.success() {
            return Err(format!("download of {url} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        if out.stdout.len() > max_bytes {
            return Err(format!("download of {url} is larger than {max_bytes} bytes"));
        }
        Ok(out.stdout)
    }
}

/// What a release download would fetch for this platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleasePlan {
    pub platform: String,
    pub asset: String,
    pub asset_url: String,
    pub checksums_url: String,
}

fn repo_ok(r: &str) -> bool {
    let mut p = r.split('/');
    let ok = |s: &str| !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    matches!((p.next(), p.next(), p.next()), (Some(a), Some(b), None) if ok(a) && ok(b))
}

fn asset_ok(a: &str) -> bool {
    !a.is_empty() && a != "." && a != ".." && a.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+'))
}

/// URLs for one platform. Only `github.com` releases of the declared repo
/// (default: the install repo) are ever produced.
pub fn release_plan_for(m: &Manifest, install_repo: Option<&str>, platform: &str) -> Result<Option<ReleasePlan>, String> {
    let Some(rel) = &m.release else { return Ok(None) };
    let Some(asset) = rel.assets.get(platform) else { return Ok(None) };
    let repo = rel.repo.as_deref().or(install_repo).ok_or("release.repo is required when the plugin is not installed from GitHub")?;
    if !repo_ok(repo) {
        return Err(format!("release.repo {repo:?} is not owner/repo"));
    }
    let tag = rel.tag.as_deref().unwrap_or("v{version}").replace("{version}", &m.version);
    if !asset_ok(&tag) || !asset_ok(asset) || !asset_ok(&rel.checksums) {
        return Err("release tag and asset names may only contain letters, digits, '-', '_', '.' and '+'".into());
    }
    let base = format!("{}/{repo}/releases/download/{tag}", super::source::GITHUB);
    Ok(Some(ReleasePlan {
        platform: platform.to_owned(),
        asset: asset.clone(),
        asset_url: format!("{base}/{asset}"),
        checksums_url: format!("{base}/{}", rel.checksums),
    }))
}

/// The plan for the platform garage is running on.
pub fn release_plan(m: &Manifest, install_repo: Option<&str>) -> Result<Option<ReleasePlan>, String> {
    release_plan_for(m, install_repo, &argv::platform_id())
}

/// Every release URL the manifest could ever download (all platforms), for
/// the "did the URLs change" comparison.
pub fn all_release_urls(m: &Manifest, install_repo: Option<&str>) -> Vec<String> {
    let Some(rel) = &m.release else { return vec![] };
    let mut urls = Vec::new();
    for platform in rel.assets.keys() {
        if let Ok(Some(p)) = release_plan_for(m, install_repo, platform) {
            urls.push(p.asset_url);
            urls.push(p.checksums_url);
        } else {
            urls.push(format!("invalid release for {platform}"));
        }
    }
    urls.sort();
    urls.dedup();
    urls
}

/// `<sha256>  <name>` lines (also `<sha256> *<name>`, as sha256sum prints
/// for binary files). The digest for `asset`.
pub fn checksum_for(checksums: &str, asset: &str) -> Option<String> {
    checksums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (hash, name) = (it.next()?, it.next()?);
        let name = name.strip_prefix('*').unwrap_or(name);
        (name == asset && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

/// Download the platform's asset into `<plugin_dir>/bin/<id>` after checking
/// its sha256 against the checksums file. Nothing is written on a mismatch.
pub fn download_release(fetch: &dyn Fetch, plan: &ReleasePlan, plugin_dir: &Path, plugin_id: &str) -> Result<(), String> {
    let sums = fetch.get(&plan.checksums_url, 1024 * 1024)?;
    let sums = String::from_utf8_lossy(&sums);
    let want = checksum_for(&sums, &plan.asset).ok_or_else(|| format!("the checksums file has no entry for {}", plan.asset))?;
    let bytes = fetch.get(&plan.asset_url, MAX_ASSET_BYTES)?;
    let got = sha256_hex(&bytes);
    if got != want {
        return Err(format!("checksum mismatch for {}: expected {want}, got {got}; nothing was installed", plan.asset));
    }
    let dest = argv::bin_path(plugin_dir, plugin_id);
    let dir = dest.parent().expect("bin has a parent");
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let is_link = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink());
    if is_link(dir) || is_link(&dest) {
        return Err(format!("{} is a symlink; refusing to write through it", if is_link(dir) { dir } else { &dest }.display()));
    }
    write_atomic_exec(&dest, &bytes)
}

/// Write `bytes` next to `dest` under a fresh name (`create_new` never
/// follows a planted link), flush them to disk, mark the file executable and
/// rename it into place.
fn write_atomic_exec(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let dir = dest.parent().expect("dest has a parent");
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = dir.join(format!(".download-{}-{nanos}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, dest)
    })();
    result.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", dest.display())
    })
}

/// The `[[build]]` entries that apply to this OS.
pub fn builds_for_os(m: &Manifest) -> Vec<&Build> {
    let os = argv::current_os();
    m.build.iter().filter(|b| b.platforms.is_empty() || b.platforms.iter().any(|p| p.as_str() == os)).collect()
}

/// Runs one build command to completion.
pub trait Builder {
    fn run(&self, argv: &[String], cwd: &Path, timeout: Duration, log: &Path) -> Result<(), String>;
}

/// Argv only, no shell; stdout and stderr go to the plugin's log file.
pub struct ProcessBuilder;

impl Builder for ProcessBuilder {
    fn run(&self, argv: &[String], cwd: &Path, timeout: Duration, log: &Path) -> Result<(), String> {
        let Some((program, args)) = argv.split_first() else { return Err("empty build command".into()) };
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(log).map_err(|e| format!("cannot open {}: {e}", log.display()))?;
        {
            use std::io::Write;
            let _ = writeln!(&file, "[build] {}", argv.join(" "));
        }
        let mut cmd = Command::new(argv::resolve_program(program, cwd));
        // Same clean environment as a plugin's own commands: the user's tokens
        // and cloud keys stay out of a stranger's build script.
        cmd.args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(garage_daemon::plugins::runner::base_env(std::env::vars()))
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(file));
        let mut child = cmd.spawn().map_err(|e| format!("cannot start {program}: {e}"))?;
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if start.elapsed() > timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("build step {program} timed out after {}s", timeout.as_secs()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(e) => return Err(e.to_string()),
            }
        };
        if status.success() {
            Ok(())
        } else {
            Err(format!("build step {} failed ({status}); output is in {}", argv.join(" "), log.display()))
        }
    }
}

/// Run the manifest's builds in `plugin_dir`, substituting the argv variables
/// against that directory. At install and update time `plugin_dir` is the
/// staging folder, which is moved to its final place afterwards, so a build
/// must not record `{plugin_dir}` in what it produces; use paths relative to
/// the plugin folder instead.
pub fn run_builds(builder: &dyn Builder, m: &Manifest, plugin_dir: &Path, state_dir: &Path, log: &Path) -> Result<(), String> {
    let platform = argv::platform_id();
    let vars = Vars { plugin_id: &m.id, plugin_dir, state_dir, platform: &platform };
    for b in builds_for_os(m) {
        let timeout = Duration::from_secs(u64::from(b.timeout_secs.unwrap_or(garage_plugin::manifest::DEFAULT_BUILD_TIMEOUT_SECS)));
        builder.run(&argv::substitute(&b.command, &vars), plugin_dir, timeout, log)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::testkit::FakeFetch;

    fn manifest(release: &str) -> Manifest {
        garage_plugin::parse(&format!(
            "id = \"rel-demo\"\nname = \"R\"\nversion = \"1.2.0\"\napi_version = 1\nplatforms = [\"macos\",\"linux\",\"windows\"]\n{release}"
        ))
        .unwrap()
    }

    fn rel_manifest() -> Manifest {
        manifest("[release]\nchecksums = \"SHA256SUMS\"\n[release.assets]\nmacos-aarch64 = \"demo-mac\"\nlinux-x86_64 = \"demo-linux\"\nmacos-x86_64 = \"demo-mac-x\"\nlinux-aarch64 = \"demo-linux-a\"\nwindows-x86_64 = \"demo.exe\"\n")
    }

    #[test]
    fn urls_come_only_from_github_releases_of_the_declared_repo_with_the_version_in_the_tag() {
        let m = rel_manifest();
        let p = release_plan_for(&m, Some("o/r"), "macos-aarch64").unwrap().unwrap();
        assert_eq!(p.asset_url, "https://github.com/o/r/releases/download/v1.2.0/demo-mac");
        assert_eq!(p.checksums_url, "https://github.com/o/r/releases/download/v1.2.0/SHA256SUMS");
        assert_eq!(release_plan_for(&m, Some("o/r"), "freebsd-x86_64").unwrap(), None, "unlisted platform means build");
        assert_eq!(release_plan_for(&manifest(""), Some("o/r"), "macos-aarch64").unwrap(), None);
        let other = manifest("[release]\nrepo = \"x/y\"\ntag = \"rel-{version}\"\nchecksums = \"S\"\n[release.assets]\nmacos-aarch64 = \"a\"\n");
        let p = release_plan_for(&other, Some("o/r"), "macos-aarch64").unwrap().unwrap();
        assert_eq!(p.asset_url, "https://github.com/x/y/releases/download/rel-1.2.0/a");
        // hostile names cannot redirect the download
        let evil = manifest("[release]\nrepo = \"x/y\"\nchecksums = \"S\"\n[release.assets]\nmacos-aarch64 = \"../../evil\"\n");
        assert!(release_plan_for(&evil, None, "macos-aarch64").is_err());
        let evil = manifest("[release]\nrepo = \"x/y/z\"\nchecksums = \"S\"\n[release.assets]\nmacos-aarch64 = \"a\"\n");
        assert!(release_plan_for(&evil, None, "macos-aarch64").unwrap_err().contains("owner/repo"));
        assert!(release_plan_for(&other, None, "macos-aarch64").is_ok(), "own repo declared");
        assert!(release_plan_for(&manifest("[release]\nchecksums = \"S\"\n[release.assets]\nmacos-aarch64 = \"a\"\n"), None, "macos-aarch64").is_err());
        assert_eq!(all_release_urls(&m, Some("o/r")).len(), 6, "five assets and one shared checksums file");
    }

    #[test]
    fn checksum_lines_accept_both_sha256sum_spellings_and_ignore_other_assets() {
        let h = "a".repeat(64);
        let sums = format!("{h}  demo-mac\n{} *demo-linux\nnot a line\n{}  short\n", "b".repeat(64), "c".repeat(10));
        assert_eq!(checksum_for(&sums, "demo-mac"), Some(h));
        assert_eq!(checksum_for(&sums, "demo-linux"), Some("b".repeat(64)));
        assert_eq!(checksum_for(&sums, "short"), None, "a malformed digest is no digest");
        assert_eq!(checksum_for(&sums, "missing"), None);
    }

    #[test]
    fn a_verified_asset_is_stored_executable_and_a_bad_one_is_never_written() {
        let m = rel_manifest();
        let plan = release_plan_for(&m, Some("o/r"), "macos-aarch64").unwrap().unwrap();
        let body = b"#!/bin/sh\necho hi\n";
        let sums = format!("{}  demo-mac\n", sha256_hex(body));
        let ok = FakeFetch::new(&[(&plan.asset_url, body), (&plan.checksums_url, sums.as_bytes())]);
        let d = tempfile::tempdir().unwrap();
        download_release(&ok, &plan, d.path(), "rel-demo").unwrap();
        let bin = argv::bin_path(d.path(), "rel-demo");
        assert_eq!(std::fs::read(&bin).unwrap(), body);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777, 0o755);
        }
        assert_eq!(*ok.calls.lock().unwrap(), [plan.checksums_url.clone(), plan.asset_url.clone()], "checksums first");

        let d = tempfile::tempdir().unwrap();
        let tampered = FakeFetch::new(&[(&plan.asset_url, b"tampered"), (&plan.checksums_url, sums.as_bytes())]);
        let e = download_release(&tampered, &plan, d.path(), "rel-demo").unwrap_err();
        assert!(e.contains("checksum mismatch"), "{e}");
        assert!(!argv::bin_path(d.path(), "rel-demo").exists());
        let no_entry = FakeFetch::new(&[(&plan.asset_url, body), (&plan.checksums_url, b"")]);
        assert!(download_release(&no_entry, &plan, d.path(), "rel-demo").unwrap_err().contains("no entry"));
        assert!(download_release(&FakeFetch::new(&[]), &plan, d.path(), "rel-demo").unwrap_err().contains("404"));
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_at_the_destination_is_never_written_through() {
        use std::os::unix::fs::symlink;
        let m = rel_manifest();
        let plan = release_plan_for(&m, Some("o/r"), "macos-aarch64").unwrap().unwrap();
        let body = b"#!/bin/sh\necho hi\n";
        let sums = format!("{}  demo-mac\n", sha256_hex(body));
        let fetch = FakeFetch::new(&[(&plan.asset_url, body), (&plan.checksums_url, sums.as_bytes())]);
        let victim = tempfile::tempdir().unwrap();
        std::fs::write(victim.path().join("precious"), "keep me").unwrap();

        // the binary path itself is a link to a file elsewhere
        let d = tempfile::tempdir().unwrap();
        let bin = argv::bin_path(d.path(), "rel-demo");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        symlink(victim.path().join("precious"), &bin).unwrap();
        let e = download_release(&fetch, &plan, d.path(), "rel-demo").unwrap_err();
        assert!(e.contains("symlink"), "{e}");
        assert_eq!(std::fs::read_to_string(victim.path().join("precious")).unwrap(), "keep me");

        // the bin folder is a link to a folder elsewhere
        let d = tempfile::tempdir().unwrap();
        let bin = argv::bin_path(d.path(), "rel-demo");
        symlink(victim.path(), bin.parent().unwrap()).unwrap();
        assert!(download_release(&fetch, &plan, d.path(), "rel-demo").unwrap_err().contains("symlink"));
        assert_eq!(std::fs::read_dir(victim.path()).unwrap().count(), 1, "nothing was created in the target folder");

        // a regular old file is replaced atomically and no temp file is left behind
        let d = tempfile::tempdir().unwrap();
        let bin = argv::bin_path(d.path(), "rel-demo");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "old").unwrap();
        download_release(&fetch, &plan, d.path(), "rel-demo").unwrap();
        assert_eq!(std::fs::read(&bin).unwrap(), body);
        assert_eq!(std::fs::read_dir(bin.parent().unwrap()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_real_build_step_runs_without_a_shell_logs_output_and_reports_failure_and_timeout() {
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("logs/p.log");
        let b = ProcessBuilder;
        let sh = |script: &str| vec!["sh".to_owned(), "-c".into(), script.to_owned()];
        b.run(&sh("echo built > out.txt; echo noise"), d.path(), Duration::from_secs(10), &log).unwrap();
        assert_eq!(std::fs::read_to_string(d.path().join("out.txt")).unwrap().trim(), "built");
        assert!(std::fs::read_to_string(&log).unwrap().contains("noise"));
        let e = b.run(&sh("echo boom >&2; exit 3"), d.path(), Duration::from_secs(10), &log).unwrap_err();
        assert!(e.contains("failed") && e.contains("p.log"), "{e}");
        assert!(std::fs::read_to_string(&log).unwrap().contains("boom"));
        // the parent's variables are not in a build's environment, but PATH is
        assert!(std::env::var("CARGO_MANIFEST_DIR").is_ok(), "cargo gives the test process this variable");
        b.run(&sh("echo \"[$CARGO_MANIFEST_DIR][$PATH]\" > env.txt"), d.path(), Duration::from_secs(10), &log).unwrap();
        let env_seen = std::fs::read_to_string(d.path().join("env.txt")).unwrap();
        assert!(env_seen.starts_with("[]["), "{env_seen}");
        assert!(env_seen.trim_end().len() > "[][]".len(), "PATH survives: {env_seen}");
        let e = b.run(&sh("sleep 30"), d.path(), Duration::from_millis(200), &log).unwrap_err();
        assert!(e.contains("timed out"), "{e}");
        assert!(b.run(&["./definitely-not-here".into()], d.path(), Duration::from_secs(1), &log).unwrap_err().contains("cannot start"));
        // metacharacters are plain argv, not shell syntax
        b.run(&["touch".into(), "a;b".into()], d.path(), Duration::from_secs(5), &log).unwrap();
        assert!(d.path().join("a;b").exists());
    }

    #[test]
    fn builds_are_filtered_by_os() {
        let m = manifest(
            "[[build]]\ncommand = [\"all\"]\n[[build]]\nplatforms = [\"windows\"]\ncommand = [\"win\"]\n[[build]]\nplatforms = [\"macos\", \"linux\"]\ncommand = [\"unix\"]\n",
        );
        let got: Vec<_> = builds_for_os(&m).iter().map(|b| b.command[0].as_str()).collect();
        if cfg!(windows) {
            assert_eq!(got, ["all", "win"]);
        } else {
            assert_eq!(got, ["all", "unix"]);
        }
    }
}
