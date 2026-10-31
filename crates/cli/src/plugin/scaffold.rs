//! `claude-garage plugin new <name> [--lang rust|ts|shell]` (p27).
//!
//! The templates live in `crates/cli/templates/` and are embedded; the only
//! logic here is turning a name into an id and a title, filling the
//! placeholders, refusing to overwrite anything, and checking that what was
//! written passes the same manifest checker `plugin link` uses.

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use garage_plugin::{check_host, parse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Lang {
    Rust,
    Ts,
    Shell,
}

impl Lang {
    fn label(self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::Ts => "TypeScript",
            Lang::Shell => "shell",
        }
    }

    fn test_cmd(self) -> &'static str {
        match self {
            Lang::Rust => "cargo test",
            Lang::Ts => "npm test",
            Lang::Shell => "sh test.sh",
        }
    }

    fn notes(self) -> &'static str {
        match self {
            Lang::Rust => {
                "This one is Rust: `src/main.rs` uses `garage-plugin-sdk`, and `[[build]]` runs `cargo build --release`."
            }
            Lang::Ts => {
                "This one is TypeScript on Node.js 20+: `src/` compiles to `dist/` with `npm run build`, which `[[build]]` runs; commit `package-lock.json`, never `dist/`."
            }
            Lang::Shell => {
                "This one is a POSIX shell script that needs only `sh` and `curl`; `test.sh` runs it against a fake `curl`."
            }
        }
    }

    /// (relative path, template) pairs for this language, before the shared files.
    fn files(self) -> Vec<(&'static str, &'static str)> {
        match self {
            Lang::Rust => vec![
                (
                    "garage-plugin.toml",
                    include_str!("../../templates/rust/garage-plugin.toml"),
                ),
                (
                    "Cargo.toml",
                    include_str!("../../templates/rust/Cargo.toml"),
                ),
                (
                    "src/main.rs",
                    include_str!("../../templates/rust/src/main.rs"),
                ),
            ],
            Lang::Ts => vec![
                (
                    "garage-plugin.toml",
                    include_str!("../../templates/ts/garage-plugin.toml"),
                ),
                (
                    "package.json",
                    include_str!("../../templates/ts/package.json"),
                ),
                (
                    "tsconfig.json",
                    include_str!("../../templates/ts/tsconfig.json"),
                ),
                (
                    "src/plugin.ts",
                    include_str!("../../templates/ts/src/plugin.ts"),
                ),
                (
                    "src/main.ts",
                    include_str!("../../templates/ts/src/main.ts"),
                ),
                (
                    "src/plugin.test.ts",
                    include_str!("../../templates/ts/src/plugin.test.ts"),
                ),
            ],
            Lang::Shell => vec![
                (
                    "garage-plugin.toml",
                    include_str!("../../templates/shell/garage-plugin.toml"),
                ),
                ("plugin.sh", include_str!("../../templates/shell/plugin.sh")),
                ("test.sh", include_str!("../../templates/shell/test.sh")),
            ],
        }
    }
}

/// Where the Rust template finds the SDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdkSource {
    /// The tagged release of this garage version (the SDK is not on crates.io).
    Git,
    /// A local checkout, for working on the SDK itself.
    Path(PathBuf),
}

impl SdkSource {
    fn dep(&self) -> String {
        match self {
            SdkSource::Git => format!(
                "git = \"https://github.com/adar2378/claude-garage\", tag = \"v{}\"",
                env!("CARGO_PKG_VERSION")
            ),
            SdkSource::Path(p) => format!("path = {:?}", p.to_string_lossy()),
        }
    }
}

pub struct NewPlugin<'a> {
    pub name: &'a str,
    pub lang: Lang,
    pub sdk: SdkSource,
}

/// "usage-bar" becomes "Usage bar".
pub fn title_of(id: &str) -> String {
    let spaced = id.replace('-', " ");
    let mut chars = spaced.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn check_name(name: &str) -> Result<(), String> {
    if !garage_plugin::validate::valid_plugin_id(name) {
        return Err(format!(
            "{name:?} is not a valid plugin id: use 2 to 31 characters, lowercase letters, digits and '-', starting with a letter (for example `usage-bar`)"
        ));
    }
    Ok(())
}

fn fill(template: &str, p: &NewPlugin) -> String {
    template
        .replace("@@ID@@", p.name)
        .replace("@@TITLE@@", &title_of(p.name))
        .replace("@@LANG@@", p.lang.label())
        .replace("@@LANG_NOTES@@", p.lang.notes())
        .replace("@@TEST_CMD@@", p.lang.test_cmd())
        .replace("@@SDK_SRC@@", &p.sdk.dep())
}

/// Write the plugin into `parent/<name>` and return that folder.
pub fn create(parent: &Path, p: &NewPlugin) -> Result<PathBuf, String> {
    check_name(p.name)?;
    let dir = parent.join(p.name);
    if dir.exists() {
        return Err(format!(
            "{} already exists; pick another name or remove it first",
            dir.display()
        ));
    }
    let mut files: Vec<(&str, &str)> = p.lang.files();
    files.extend([
        (
            "README.md",
            include_str!("../../templates/common/README.md"),
        ),
        (
            "CLAUDE.md",
            include_str!("../../templates/common/CLAUDE.md"),
        ),
        (
            ".gitignore",
            include_str!("../../templates/common/gitignore"),
        ),
    ]);
    // Render everything and check the manifest before writing a single file.
    let rendered: Vec<(&str, String)> = files.iter().map(|(n, t)| (*n, fill(t, p))).collect();
    let manifest = rendered
        .iter()
        .find(|(n, _)| *n == "garage-plugin.toml")
        .map(|(_, t)| t.as_str())
        .expect("every language has a manifest");
    check_manifest(manifest)?;

    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for (rel, text) in &rendered {
        let path = dir.join(rel);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)
                .map_err(|e| format!("cannot create {}: {e}", d.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    #[cfg(unix)]
    if p.lang == Lang::Shell {
        use std::os::unix::fs::PermissionsExt;
        for f in ["plugin.sh", "test.sh"] {
            let path = dir.join(f);
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(dir)
}

/// The same checks `plugin link` runs, so a template bug fails here, not for the user.
fn check_manifest(text: &str) -> Result<(), String> {
    let m = parse(text).map_err(|ps| {
        ps.iter()
            .map(|p| format!("{}: {}", p.path, p.message))
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let host = check_host(&m);
    if host.is_empty() {
        Ok(())
    } else {
        Err(host
            .iter()
            .map(|p| format!("{}: {}", p.path, p.message))
            .collect::<Vec<_>>()
            .join("; "))
    }
}

/// What to print after a successful `plugin new`.
pub fn next_steps(dir: &Path, p: &NewPlugin) -> String {
    let shown = dir.display();
    format!(
        "Created {shown}\n\nNext steps:\n  cd {name}\n  claude-garage plugin dev .      # link it, rebuild and reload on change, tail its log\n  (or: claude-garage plugin link .  to link without watching)\n\nThen run \"Say hello\" ({name}:greet) from the palette. Test: {test}\nPublish: push to GitHub and add the topic `claude-garage-plugin`. See README.md and CLAUDE.md.",
        name = p.name,
        test = p.lang.test_cmd(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(name: &str, lang: Lang) -> NewPlugin<'_> {
        NewPlugin {
            name,
            lang,
            sdk: SdkSource::Git,
        }
    }

    #[test]
    fn titles_are_made_from_the_id() {
        assert_eq!(title_of("usage-bar"), "Usage bar");
        assert_eq!(title_of("x1"), "X1");
    }

    #[test]
    fn bad_names_are_refused_with_the_rule() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["Hello", "a", "1abc", "has space", "../x", "plugin"] {
            let e = create(d.path(), &new(bad, Lang::Shell)).unwrap_err();
            assert!(
                e.contains("not a valid plugin id") || e.contains("reserved"),
                "{bad}: {e}"
            );
        }
        assert_eq!(
            std::fs::read_dir(d.path()).unwrap().count(),
            0,
            "nothing written"
        );
    }

    #[test]
    fn an_existing_folder_is_never_touched() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("taken")).unwrap();
        std::fs::write(d.path().join("taken/keep.txt"), "mine").unwrap();
        let e = create(d.path(), &new("taken", Lang::Rust)).unwrap_err();
        assert!(e.contains("already exists"), "{e}");
        assert_eq!(
            std::fs::read_to_string(d.path().join("taken/keep.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn every_language_writes_a_manifest_that_passes_the_checker_and_no_placeholder_is_left() {
        for lang in [Lang::Rust, Lang::Ts, Lang::Shell] {
            let d = tempfile::tempdir().unwrap();
            let dir = create(d.path(), &new("my-plug", lang)).unwrap();
            let manifest = std::fs::read_to_string(dir.join("garage-plugin.toml")).unwrap();
            let m = parse(&manifest).unwrap_or_else(|e| panic!("{lang:?}: {e:?}"));
            assert!(check_host(&m).is_empty(), "{lang:?}");
            assert_eq!(m.id, "my-plug");
            assert!(m.action("greet").is_some() && m.widget("status").is_some());
            for f in ["README.md", "CLAUDE.md", ".gitignore"] {
                assert!(dir.join(f).is_file(), "{lang:?} {f}");
            }
            // Every file, not only the manifest, is free of unfilled placeholders.
            let mut stack = vec![dir.clone()];
            while let Some(p) = stack.pop() {
                for e in std::fs::read_dir(p).unwrap().flatten() {
                    let path = e.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else {
                        let t = std::fs::read_to_string(&path).unwrap();
                        assert!(
                            !t.contains("@@"),
                            "{}: unfilled placeholder",
                            path.display()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_rust_template_points_at_the_tagged_sdk_or_a_local_path() {
        let d = tempfile::tempdir().unwrap();
        let dir = create(d.path(), &new("git-one", Lang::Rust)).unwrap();
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(
            cargo.contains(&format!("tag = \"v{}\"", env!("CARGO_PKG_VERSION"))),
            "{cargo}"
        );
        let local = NewPlugin {
            name: "path-one",
            lang: Lang::Rust,
            sdk: SdkSource::Path("/src/claude-garage/crates/plugin-sdk".into()),
        };
        let dir = create(d.path(), &local).unwrap();
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(
            cargo.contains("path = \"/src/claude-garage/crates/plugin-sdk\""),
            "{cargo}"
        );
        assert!(cargo.contains("features = [\"testing\"]"));
    }

    #[test]
    fn next_steps_name_the_dev_and_link_commands() {
        let p = new("my-plug", Lang::Ts);
        let text = next_steps(Path::new("/x/my-plug"), &p);
        assert!(
            text.contains("plugin dev")
                && text.contains("plugin link")
                && text.contains("npm test")
        );
    }

    /// The generated shell plugin's own test passes: this runs real `sh`, with
    /// a fake `curl`, so the template cannot rot unnoticed.
    #[cfg(unix)]
    #[test]
    fn the_generated_shell_plugin_passes_its_own_test() {
        let d = tempfile::tempdir().unwrap();
        let dir = create(d.path(), &new("shell-one", Lang::Shell)).unwrap();
        let out = std::process::Command::new("sh")
            .arg(dir.join("test.sh"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
