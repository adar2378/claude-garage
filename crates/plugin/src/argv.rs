//! Argv substitution (design section 2): `{plugin_dir}`, `{bin}`,
//! `{state_dir}`, `{platform}` are replaced inside argv elements. Nothing goes
//! through a shell, and any other braces are left alone.

use std::path::{Path, PathBuf};

pub struct Vars<'a> {
    pub plugin_id: &'a str,
    pub plugin_dir: &'a Path,
    pub state_dir: &'a Path,
    pub platform: &'a str,
}

/// `macos-aarch64`, `linux-x86_64`, ...
pub fn platform_id() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// The OS half of [`platform_id`], as the manifest spells `platforms`.
pub fn current_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

/// Where a downloaded release asset lives: `<plugin dir>/bin/<id>`.
pub fn bin_path(plugin_dir: &Path, plugin_id: &str) -> PathBuf {
    plugin_dir.join("bin").join(plugin_id)
}

pub fn substitute(argv: &[String], vars: &Vars) -> Vec<String> {
    let bin = bin_path(vars.plugin_dir, vars.plugin_id);
    argv.iter()
        .map(|a| {
            a.replace("{plugin_dir}", &vars.plugin_dir.to_string_lossy())
                .replace("{bin}", &bin.to_string_lossy())
                .replace("{state_dir}", &vars.state_dir.to_string_lossy())
                .replace("{platform}", vars.platform)
        })
        .collect()
}

/// A program starting `./` is looked up in the plugin directory; an absolute
/// path (after substitution) is used as is; a bare name is left for `PATH`.
pub fn resolve_program(program: &str, plugin_dir: &Path) -> PathBuf {
    match program.strip_prefix("./") {
        Some(rest) => plugin_dir.join(rest),
        None => PathBuf::from(program),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(dir: &'a Path, state: &'a Path) -> Vars<'a> {
        Vars { plugin_id: "usage-bar", plugin_dir: dir, state_dir: state, platform: "macos-aarch64" }
    }

    #[test]
    fn replaces_the_four_variables_inside_elements_and_leaves_other_braces() {
        let (d, s) = (Path::new("/p/usage-bar"), Path::new("/data/usage-bar"));
        let out = substitute(
            &[
                "{plugin_dir}/bin/x".into(),
                "--state={state_dir}".into(),
                "{bin}".into(),
                "{platform}".into(),
                "{unknown} ${HOME} {}".into(),
                "a; rm -rf / # {plugin_dir}".into(),
            ],
            &vars(d, s),
        );
        assert_eq!(
            out,
            [
                "/p/usage-bar/bin/x",
                "--state=/data/usage-bar",
                "/p/usage-bar/bin/usage-bar",
                "macos-aarch64",
                "{unknown} ${HOME} {}",
                // no shell: metacharacters stay one inert argv element
                "a; rm -rf / # /p/usage-bar",
            ]
        );
    }

    #[test]
    fn dot_slash_programs_resolve_into_the_plugin_dir_others_do_not() {
        let d = Path::new("/p/x");
        assert_eq!(resolve_program("./run.sh", d), PathBuf::from("/p/x/run.sh"));
        assert_eq!(resolve_program("/usr/bin/env", d), PathBuf::from("/usr/bin/env"));
        assert_eq!(resolve_program("python3", d), PathBuf::from("python3"));
    }

    #[test]
    fn platform_id_is_os_dash_arch() {
        let p = platform_id();
        assert!(p.starts_with(current_os()) && p.contains('-'), "{p}");
    }
}
