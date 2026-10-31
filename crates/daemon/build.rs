//! Compiles the first-party plugin allowlist into the daemon: for every
//! `plugins/<id>/garage-plugin.toml` except the `hello` dev fixture, the id
//! and the SHA-256 of the manifest bytes. `plugins::bundled` auto-grants a
//! bundled plugin only when both match (a manifest edit never silently
//! re-grants).
//!
//! With the `embed-plugins` feature (release builds, via cargo-dist) it also
//! builds every first-party plugin for the target in a nested cargo run and
//! embeds each plugin's manifest and binary, with their SHA-256, as
//! `PAYLOAD`. The daemon unpacks that set under its data directory and checks
//! it before every plugin start, so a binary swapped on disk is never run.

use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

/// Dev/test fixture: shipped in the checkout, never staged, never trusted.
const FIXTURE: &str = "hello";

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// The `[package] name` of a plugin crate (first `name = "..."` line).
fn package_name(cargo_toml: &Path) -> String {
    let text = std::fs::read_to_string(cargo_toml).unwrap();
    text.lines()
        .find_map(|l| {
            let rest = l
                .trim()
                .strip_prefix("name")?
                .trim_start()
                .strip_prefix('=')?;
            Some(rest.trim().trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| panic!("no package name in {}", cargo_toml.display()))
}

/// Does this manifest's `platforms = [...]` list the target OS? A manifest
/// with no `platforms` line is a build error, not a silent skip: a release
/// must never quietly lose a plugin.
fn supports(id: &str, manifest: &str, os: &str) -> bool {
    let line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("platforms"))
        .unwrap_or_else(|| panic!("plugins/{id}/garage-plugin.toml has no `platforms` line"));
    line.contains(&format!("\"{os}\""))
}

/// The cargo profile the outer build uses (`dev`, `release`, `dist`, ...),
/// read from `OUT_DIR` = `<target>[/<triple>]/<profile>/build/<pkg>/out`:
/// `PROFILE` only says debug or release, and the plugins must be built the
/// way the daemon is (cargo-dist builds with `--profile dist`).
fn outer_profile(out: &Path) -> String {
    let dir = out
        .ancestors()
        .nth(3)
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("release");
    if dir == "debug" { "dev" } else { dir }.to_owned()
}

/// Build the shippable plugins for the target and return the generated
/// `PAYLOAD` source.
fn embed_plugins(root: &Path, plugins: &Path) -> String {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let (target, host) = (
        std::env::var("TARGET").unwrap(),
        std::env::var("HOST").unwrap(),
    );
    let exe_suffix = if os == "windows" { ".exe" } else { "" };
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let tdir = out.join("plugin-target");
    let profile = outer_profile(&out);
    // cargo names the output directory `debug` for the dev profile
    let profile_dir = if profile == "dev" {
        "debug"
    } else {
        profile.as_str()
    };

    // (id, manifest path, optional binary crate)
    let mut found: Vec<(String, PathBuf, Option<PathBuf>)> = Vec::new();
    for e in std::fs::read_dir(plugins).unwrap().flatten() {
        let Ok(id) = e.file_name().into_string() else {
            continue;
        };
        let manifest = e.path().join("garage-plugin.toml");
        if id == FIXTURE || !manifest.is_file() {
            continue;
        }
        if !supports(&id, &std::fs::read_to_string(&manifest).unwrap(), &os) {
            continue;
        }
        let cargo_toml = e.path().join("Cargo.toml");
        found.push((id, manifest, cargo_toml.is_file().then_some(cargo_toml)));
    }
    found.sort();
    // An embed build that ships nothing is a broken release, not an empty one.
    assert!(
        !found.is_empty(),
        "embed-plugins: no first-party plugin supports target OS {os}; refusing to build an empty payload"
    );

    let packages: Vec<String> = found
        .iter()
        .filter_map(|(_, _, c)| c.as_deref().map(package_name))
        .collect();
    if !packages.is_empty() {
        let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        // --locked: the nested build must use the root Cargo.lock as it is
        // and never rewrite it.
        cmd.arg("build")
            .arg("--locked")
            .args(["--profile", &profile])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&tdir);
        if target != host {
            cmd.args(["--target", &target]);
        }
        for p in &packages {
            cmd.args(["-p", p]);
        }
        // The outer cargo's own target dir and build state must not leak in.
        cmd.env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_TARGET");
        // They are embedded in the binary: keep them small.
        cmd.env(
            format!(
                "CARGO_PROFILE_{}_STRIP",
                profile.to_uppercase().replace('-', "_")
            ),
            "true",
        );
        let status = cmd.status().expect("run cargo for the bundled plugins");
        assert!(status.success(), "building the bundled plugins failed");
    }
    let bins = if target != host {
        tdir.join(&target)
    } else {
        tdir.clone()
    }
    .join(profile_dir);

    let mut src = String::from(
        "/// Every first-party plugin built for this target, generated by build.rs.\npub static PAYLOAD: &[PayloadPlugin] = &[\n",
    );
    for (id, manifest, cargo_toml) in &found {
        let _ = writeln!(src, "    PayloadPlugin {{ id: {id:?}, files: &[");
        let bytes = std::fs::read(manifest).unwrap();
        let _ = writeln!(
            src,
            "        PayloadFile {{ path: \"garage-plugin.toml\", sha256: {:?}, exec: false, bytes: include_bytes!({:?}) }},",
            hex(&bytes),
            manifest.display().to_string()
        );
        if cargo_toml.is_some() {
            let bin = bins.join(format!("{id}{exe_suffix}"));
            let bytes = std::fs::read(&bin)
                .unwrap_or_else(|e| panic!("bundled plugin binary {}: {e}", bin.display()));
            let _ = writeln!(
                src,
                "        PayloadFile {{ path: \"bin/{id}{exe_suffix}\", sha256: {:?}, exec: true, bytes: include_bytes!({:?}) }},",
                hex(&bytes),
                bin.display().to_string()
            );
        }
        src.push_str("    ] },\n");
    }
    src.push_str("];\n");
    src
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let embed = std::env::var_os("CARGO_FEATURE_EMBED_PLUGINS").is_some();
    let payload = if embed {
        for dir in [
            "plugins",
            "crates/plugin-sdk",
            "crates/plugin",
            "crates/protocol",
        ] {
            println!("cargo:rerun-if-changed={}", root.join(dir).display());
        }
        println!(
            "cargo:rerun-if-changed={}",
            root.join("Cargo.lock").display()
        );
        embed_plugins(&root, &root.join("plugins"))
    } else {
        "/// Empty: this build does not embed the plugins.\npub static PAYLOAD: &[PayloadPlugin] = &[];\n"
            .to_owned()
    };
    std::fs::write(
        Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("bundled_payload.rs"),
        payload,
    )
    .unwrap();

    let plugins = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    println!("cargo:rerun-if-changed={}", plugins.display());
    let mut entries: Vec<(String, String)> = std::fs::read_dir(&plugins)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().into_string().ok()?;
            let bytes = std::fs::read(e.path().join("garage-plugin.toml")).ok()?;
            (id != FIXTURE).then(|| {
                (
                    id,
                    Sha256::digest(bytes)
                        .iter()
                        .fold(String::new(), |mut s, b| {
                            let _ = write!(s, "{b:02x}");
                            s
                        }),
                )
            })
        })
        .collect();
    entries.sort();
    let mut src = String::from(
        "/// (plugin id, SHA-256 of its garage-plugin.toml), generated by build.rs.\npub const ALLOWLIST: &[(&str, &str)] = &[\n",
    );
    for (id, sha) in &entries {
        let _ = writeln!(src, "    ({id:?}, {sha:?}),");
    }
    src.push_str("];\n");
    let out = Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("bundled_allowlist.rs");
    std::fs::write(out, src).unwrap();
}
