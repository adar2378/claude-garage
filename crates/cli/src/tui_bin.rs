//! Port of `resolveTuiBinary` (and the p11 staleness guard) from
//! `bin/garage.js`. Linking `garage_wall` as a library replaces this later.

use crate::launcher::{Fail, Out};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Cargo, injected so tests never build anything.
pub trait Builder {
    fn have_cargo(&self) -> bool;
    /// `cargo build --release -p garage-wall` in `root`; true on success.
    fn build(&self, root: &Path) -> bool;
}

pub struct Inputs {
    /// `GARAGE_TUI_BIN`.
    pub override_bin: Option<String>,
    /// Package/checkout root (JS `ROOT`).
    pub root: PathBuf,
    /// Directory holding the running `claude-garage` binary.
    pub exe_dir: Option<PathBuf>,
    /// `<platform>-<arch>`, Node-style (`darwin-arm64`).
    pub target: String,
}

/// `process.platform`-`process.arch` spelling of the current target.
pub fn node_target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        o => o,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        a => a,
    };
    format!("{os}-{arch}")
}

#[cfg(unix)]
fn is_executable(p: &str) -> bool {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(std::ffi::OsStr::new(p).as_bytes())
        .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0)
}

/// Windows has no execute bit: a file is runnable when it is a file.
#[cfg(not(unix))]
fn is_executable(p: &str) -> bool {
    Path::new(p).is_file()
}

/// `.exe` on Windows, nothing elsewhere.
const EXE: &str = std::env::consts::EXE_SUFFIX;

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Latest mtime of any file under `dir`, recursively (None when empty/missing).
fn newest_mtime_under(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&cur) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(path);
            } else if let Some(m) = mtime(&path) {
                newest = Some(newest.map_or(m, |n| n.max(m)));
            }
        }
    }
    newest
}

/// p11 staleness guard: wall/Cargo.toml, Cargo.lock, wall/src or crates/
/// newer than the built binary.
fn wall_sources_newer_than(root: &Path, binary: &Path) -> bool {
    let Some(bin_m) = mtime(binary) else {
        return false;
    };
    for f in [root.join("wall/Cargo.toml"), root.join("Cargo.lock")] {
        if mtime(&f).is_some_and(|m| m > bin_m) {
            return true;
        }
    }
    [root.join("wall/src"), root.join("crates")]
        .iter()
        .any(|d| newest_mtime_under(d).is_some_and(|m| m > bin_m))
}

fn cargo_build_tui_binary(root: &Path, dist_bin: &Path, builder: &dyn Builder) -> Result<(), Fail> {
    let built = root
        .join("target/release")
        .join(format!("garage-wall{EXE}"));
    if !builder.build(root) || !built.exists() {
        return Err(Fail("cargo build failed — see the output above".into()));
    }
    let copy = || -> std::io::Result<()> {
        if let Some(dir) = dist_bin.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(&built, dist_bin)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dist_bin, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    };
    copy().map_err(|e| {
        Fail(format!(
            "could not install the built TUI binary at {}: {e}",
            dist_bin.display()
        ))
    })
}

/// Lookup order: `GARAGE_TUI_BIN` (verbatim, hard error if unusable);
/// `wall/dist/garage-wall-<target>` (with the staleness guard in a
/// checkout); build once with cargo (checkout only); actionable error.
///
/// Deviation from the JS: with no `wall/` sources (an installed layout) a
/// `garage-wall` beside the running binary is used before giving up, since
/// the Rust distribution ships both side by side.
pub fn resolve(inp: &Inputs, builder: &dyn Builder, out: &dyn Out) -> Result<PathBuf, Fail> {
    if let Some(o) = inp.override_bin.as_deref().filter(|s| !s.is_empty()) {
        if is_executable(o) {
            return Ok(PathBuf::from(o));
        }
        return Err(Fail(format!(
            "GARAGE_TUI_BIN is set but not an executable file: {o}"
        )));
    }

    let target = &inp.target;
    let dist_bin = inp
        .root
        .join("wall/dist")
        .join(format!("garage-wall-{target}{EXE}"));
    let wall_src = inp.root.join("wall/Cargo.toml");
    let in_checkout = wall_src.exists();

    if dist_bin.exists() {
        if in_checkout && wall_sources_newer_than(&inp.root, &dist_bin) {
            if builder.have_cargo() {
                out.say("wall sources newer than the built binary — rebuilding");
                cargo_build_tui_binary(&inp.root, &dist_bin, builder)?;
            } else {
                out.warn(&format!(
                    "warning: wall/src is newer than the built TUI binary ({}) and no cargo on PATH to rebuild it — it may be stale",
                    dist_bin.display()
                ));
            }
        }
        return Ok(dist_bin);
    }

    if !in_checkout
        && let Some(sib) = inp
            .exe_dir
            .as_ref()
            .map(|d| d.join(format!("garage-wall{EXE}")))
            .filter(|p| p.exists())
    {
        return Ok(sib);
    }

    if in_checkout && builder.have_cargo() {
        out.say(&format!(
            "no prebuilt TUI binary for {target} — building once with cargo (release build; lands at wall/dist/garage-wall-{target}{EXE})"
        ));
        cargo_build_tui_binary(&inp.root, &dist_bin, builder)?;
        return Ok(dist_bin);
    }

    Err(Fail(if in_checkout {
        format!(
            "no prebuilt TUI binary for {target} and no Rust toolchain (cargo) on PATH.\nInstall Rust (https://rustup.rs) and re-run — the TUI builds itself once from wall/ — or use a platform with a shipped binary (macOS arm64)."
        )
    } else {
        format!(
            "no prebuilt TUI binary for {target} in this package, and no wall/ sources to build from.\nUse a platform with a shipped binary (macOS arm64), or run from a git checkout with the Rust toolchain installed (npm run build:tui)."
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn chmod(p: &Path, mode: u32) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        #[cfg(not(unix))]
        let _ = (p, mode);
    }
    fn is_exec(p: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777 == 0o755
        }
        #[cfg(not(unix))]
        {
            p.exists()
        }
    }
    use std::time::Duration;

    struct Sink(RefCell<Vec<String>>);
    impl Out for Sink {
        fn say(&self, l: &str) {
            self.0.borrow_mut().push(l.to_owned());
        }
        fn warn(&self, l: &str) {
            self.0.borrow_mut().push(format!("warn: {l}"));
        }
    }

    struct FakeCargo {
        have: bool,
        builds: Cell<u32>,
        ok: bool,
    }
    impl Builder for FakeCargo {
        fn have_cargo(&self) -> bool {
            self.have
        }
        fn build(&self, root: &Path) -> bool {
            self.builds.set(self.builds.get() + 1);
            if self.ok {
                let out = root.join("target/release/garage-wall");
                std::fs::create_dir_all(out.parent().unwrap()).unwrap();
                std::fs::write(out, b"built").unwrap();
            }
            self.ok
        }
    }
    fn cargo(have: bool, ok: bool) -> FakeCargo {
        FakeCargo {
            have,
            builds: Cell::new(0),
            ok,
        }
    }

    /// A unique scratch dir under the system temp dir, removed on drop.
    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(
                format!(
                    "cg-cli-{tag}-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                )
                .replace(['(', ')'], ""),
            );
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn file(&self, rel: &str, age_secs: u64) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
            chmod(&p, 0o755);
            let t = SystemTime::now() - Duration::from_secs(age_secs);
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
            p
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn inputs(root: &Path) -> Inputs {
        Inputs {
            override_bin: None,
            root: root.to_path_buf(),
            exe_dir: None,
            target: "darwin-arm64".into(),
        }
    }

    #[test]
    fn garage_tui_bin_wins_verbatim_and_never_triggers_a_rebuild() {
        let t = Tmp::new("override");
        let custom = t.file("custom-wall", 0);
        t.file("wall/Cargo.toml", 0); // checkout with sources newer than anything
        t.file("wall/dist/garage-wall-darwin-arm64", 1000);
        let mut i = inputs(&t.0);
        i.override_bin = Some(custom.display().to_string());
        let c = cargo(true, true);
        assert_eq!(resolve(&i, &c, &Sink(Default::default())).unwrap(), custom);
        assert_eq!(c.builds.get(), 0);
    }

    #[test]
    fn an_unusable_override_is_a_hard_error_not_a_silent_fallback() {
        let t = Tmp::new("badoverride");
        t.file("wall/dist/garage-wall-darwin-arm64", 0); // would be a fine fallback
        let mut i = inputs(&t.0);
        i.override_bin = Some(t.0.join("nope").display().to_string());
        let err = resolve(&i, &cargo(true, true), &Sink(Default::default())).unwrap_err();
        assert!(
            err.0
                .starts_with("GARAGE_TUI_BIN is set but not an executable file: "),
            "{}",
            err.0
        );

        // present but not executable
        if cfg!(unix) {
            let plain = t.0.join("plain");
            std::fs::write(&plain, b"x").unwrap();
            chmod(&plain, 0o644);
            i.override_bin = Some(plain.display().to_string());
            assert!(resolve(&i, &cargo(true, true), &Sink(Default::default())).is_err());
        }
    }

    #[test]
    fn a_fresh_prebuilt_binary_is_used_as_is() {
        let t = Tmp::new("fresh");
        t.file("wall/Cargo.toml", 100);
        t.file("wall/src/main.rs", 100);
        t.file("Cargo.lock", 100);
        let bin = t.file("wall/dist/garage-wall-darwin-arm64", 10);
        let c = cargo(true, true);
        assert_eq!(
            resolve(&inputs(&t.0), &c, &Sink(Default::default())).unwrap(),
            bin
        );
        assert_eq!(c.builds.get(), 0);
    }

    #[test]
    fn a_source_newer_than_the_binary_triggers_one_rebuild_that_refreshes_dist() {
        // each of the watched inputs, in turn (the invisible-picker regression)
        for newer in [
            "wall/Cargo.toml",
            "wall/src/deep/ui.rs",
            "Cargo.lock",
            "crates/protocol/src/lib.rs",
        ] {
            let t = Tmp::new("stale");
            t.file("wall/Cargo.toml", 100);
            t.file("wall/src/main.rs", 100);
            let bin = t.file("wall/dist/garage-wall-darwin-arm64", 50);
            t.file(newer, 1);
            let c = cargo(true, true);
            let sink = Sink(Default::default());
            assert_eq!(resolve(&inputs(&t.0), &c, &sink).unwrap(), bin, "{newer}");
            assert_eq!(c.builds.get(), 1, "{newer}");
            assert_eq!(
                std::fs::read(&bin).unwrap(),
                b"built",
                "{newer}: dist replaced by the build"
            );
            assert!(is_exec(&bin));
            assert_eq!(
                sink.0.borrow()[0],
                "wall sources newer than the built binary — rebuilding"
            );
        }
    }

    #[test]
    fn stale_binary_without_cargo_warns_and_still_runs_it() {
        let t = Tmp::new("nocargo");
        t.file("wall/Cargo.toml", 1);
        let bin = t.file("wall/dist/garage-wall-darwin-arm64", 50);
        let sink = Sink(Default::default());
        assert_eq!(
            resolve(&inputs(&t.0), &cargo(false, true), &sink).unwrap(),
            bin
        );
        assert!(
            sink.0.borrow()[0]
                .starts_with("warn: warning: wall/src is newer than the built TUI binary")
        );
    }

    #[test]
    fn an_installed_layout_never_rebuilds_even_if_mtimes_look_odd() {
        let t = Tmp::new("installed");
        // no wall/Cargo.toml: an installed package ships no sources
        let bin = t.file("wall/dist/garage-wall-darwin-arm64", 50);
        t.file("crates/x/lib.rs", 1);
        let c = cargo(true, true);
        assert_eq!(
            resolve(&inputs(&t.0), &c, &Sink(Default::default())).unwrap(),
            bin
        );
        assert_eq!(c.builds.get(), 0);
    }

    #[test]
    fn no_binary_in_a_checkout_builds_once_into_dist() {
        let t = Tmp::new("buildonce");
        t.file("wall/Cargo.toml", 10);
        let c = cargo(true, true);
        let sink = Sink(Default::default());
        let got = resolve(&inputs(&t.0), &c, &sink).unwrap();
        assert_eq!(got, t.0.join("wall/dist/garage-wall-darwin-arm64"));
        assert_eq!(std::fs::read(&got).unwrap(), b"built");
        assert_eq!(c.builds.get(), 1);
        assert!(
            sink.0.borrow()[0]
                .starts_with("no prebuilt TUI binary for darwin-arm64 — building once with cargo")
        );
    }

    #[test]
    fn a_failed_build_is_fatal() {
        let t = Tmp::new("buildfail");
        t.file("wall/Cargo.toml", 10);
        let err = resolve(
            &inputs(&t.0),
            &cargo(true, false),
            &Sink(Default::default()),
        )
        .unwrap_err();
        assert_eq!(err.0, "cargo build failed — see the output above");
    }

    #[test]
    fn no_binary_and_no_way_to_get_one_names_what_is_missing() {
        let t = Tmp::new("nothing");
        t.file("wall/Cargo.toml", 10);
        let e = resolve(
            &inputs(&t.0),
            &cargo(false, true),
            &Sink(Default::default()),
        )
        .unwrap_err();
        assert!(
            e.0.contains("no Rust toolchain (cargo)") && e.0.contains("rustup.rs"),
            "{}",
            e.0
        );

        let t2 = Tmp::new("nothing2");
        let e = resolve(
            &inputs(&t2.0),
            &cargo(true, true),
            &Sink(Default::default()),
        )
        .unwrap_err();
        assert!(e.0.contains("no wall/ sources to build from"), "{}", e.0);
    }

    #[test]
    fn an_installed_layout_uses_the_garage_wall_beside_the_binary() {
        let t = Tmp::new("sibling");
        let sib = t.file("bin/garage-wall", 10);
        let mut i = inputs(&t.0);
        i.exe_dir = Some(t.0.join("bin"));
        assert_eq!(
            resolve(&i, &cargo(true, true), &Sink(Default::default())).unwrap(),
            sib
        );
    }

    #[test]
    fn node_target_uses_node_spelling() {
        let t = node_target();
        assert!(
            !t.contains("macos") && !t.contains("aarch64") && !t.contains("x86_64"),
            "{t}"
        );
    }
}
