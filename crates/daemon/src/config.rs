//! Daemon configuration, read from the environment (names and meaning as in
//! the JS daemon, plus the new `GARAGE_TMUX_SOCKET`).

use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 4747;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `GARAGE_PORT`, default 4747. The server binds 127.0.0.1 only.
    pub port: u16,
    /// `GARAGE_DIR`, default `~/.garage` on every OS.
    pub garage_dir: PathBuf,
    /// `GARAGE_TMUX_SOCKET`: `tmux -L <name> -f /dev/null`. Unset means the
    /// default server with the user's normal tmux.conf. Setting it also skips
    /// the user's tmux.conf (`-f /dev/null`); that is intended, for tests and
    /// isolation.
    pub tmux_socket: Option<String>,
    /// `GARAGE_TMUX_CONTROL`: `0`, `false`, `off` or `no` (any case) disables
    /// tmux control mode entirely; the poller then runs on its plain 2 s tick
    /// and no `garage-control` session is ever created. Default on.
    ///
    /// Known caveats of control mode, the reasons for this switch: the
    /// `garage-control` session is hidden from garage but not from tmux, so it
    /// shows in `tmux ls` and choose-tree; a bare `tmux attach` in the instant
    /// after daemon start can pick it (it is the most recently created
    /// session); and session-saving plugins (tmux-resurrect and friends) may
    /// save it.
    pub tmux_control: bool,
    /// `GARAGE_UI_ORIGINS`: browser origins allowed past the origin guard.
    pub ui_origins: Vec<String>,
}

impl Config {
    /// Read from the process environment.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Read through `get`, so tests need not touch the process environment.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let port = match get("GARAGE_PORT").filter(|s| !s.is_empty()) {
            Some(p) => p
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("GARAGE_PORT must be a port number, got {p:?}"))?,
            None => DEFAULT_PORT,
        };
        let garage_dir = match get("GARAGE_DIR").filter(|s| !s.is_empty()) {
            Some(d) => PathBuf::from(d),
            None => PathBuf::from(
                get("HOME")
                    .filter(|h| !h.is_empty())
                    .or_else(|| get("USERPROFILE"))
                    .ok_or_else(|| anyhow::anyhow!("HOME is not set; set GARAGE_DIR"))?,
            )
            .join(".garage"),
        };
        let ui_origins = get("GARAGE_UI_ORIGINS")
            .map(|s| {
                s.split(',')
                    .map(|o| o.trim().to_owned())
                    .filter(|o| !o.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            port,
            garage_dir,
            tmux_socket: get("GARAGE_TMUX_SOCKET").filter(|s| !s.is_empty()),
            tmux_control: !matches!(
                get("GARAGE_TMUX_CONTROL")
                    .map(|v| v.trim().to_ascii_lowercase())
                    .as_deref(),
                Some("0" | "false" | "off" | "no")
            ),
            ui_origins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> anyhow::Result<Config> {
        let m: HashMap<_, _> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| m.get(k).cloned())
    }

    #[test]
    fn defaults() {
        let c = cfg(&[("HOME", "/home/u")]).unwrap();
        assert_eq!(c.port, 4747);
        assert_eq!(c.garage_dir, PathBuf::from("/home/u/.garage"));
        assert_eq!(c.tmux_socket, None);
        assert!(c.tmux_control);
        assert!(c.ui_origins.is_empty());
    }

    #[test]
    fn windows_home_comes_from_userprofile_and_home_wins_when_both_are_set() {
        assert_eq!(
            cfg(&[("USERPROFILE", r"C:\Users\u")]).unwrap().garage_dir,
            PathBuf::from(r"C:\Users\u").join(".garage")
        );
        assert_eq!(
            cfg(&[("HOME", "/h"), ("USERPROFILE", "/w")])
                .unwrap()
                .garage_dir,
            PathBuf::from("/h/.garage")
        );
        assert!(cfg(&[]).is_err());
    }

    #[test]
    fn overrides_and_origin_list_parsing() {
        let c = cfg(&[
            ("GARAGE_PORT", "5000"),
            ("GARAGE_DIR", "/tmp/g"),
            ("GARAGE_TMUX_SOCKET", "t1"),
            ("GARAGE_UI_ORIGINS", "http://a.test, http://b.test,"),
        ])
        .unwrap();
        assert_eq!(c.port, 5000);
        assert_eq!(c.garage_dir, PathBuf::from("/tmp/g"));
        assert_eq!(c.tmux_socket.as_deref(), Some("t1"));
        assert_eq!(c.ui_origins, ["http://a.test", "http://b.test"]);
    }

    #[test]
    fn tmux_control_off_switch() {
        for off in ["0", "false", "OFF", "No", " off "] {
            assert!(
                !cfg(&[("HOME", "/h"), ("GARAGE_TMUX_CONTROL", off)])
                    .unwrap()
                    .tmux_control,
                "{off:?}"
            );
        }
        for on in ["1", "true", "on", "", "anything"] {
            assert!(
                cfg(&[("HOME", "/h"), ("GARAGE_TMUX_CONTROL", on)])
                    .unwrap()
                    .tmux_control,
                "{on:?}"
            );
        }
    }

    #[test]
    fn bad_port_is_an_error_naming_the_variable() {
        let e = cfg(&[("HOME", "/h"), ("GARAGE_PORT", "nope")]).unwrap_err();
        assert!(e.to_string().contains("GARAGE_PORT"));
    }
}
