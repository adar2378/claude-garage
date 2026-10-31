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
    /// `GARAGE_TMUX_SOCKET`: `tmux -L <name>`. Unset means the default server.
    pub tmux_socket: Option<String>,
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
                get("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set; set GARAGE_DIR"))?,
            )
            .join(".garage"),
        };
        let ui_origins = get("GARAGE_UI_ORIGINS")
            .map(|s| s.split(',').map(|o| o.trim().to_owned()).filter(|o| !o.is_empty()).collect())
            .unwrap_or_default();
        Ok(Self {
            port,
            garage_dir,
            tmux_socket: get("GARAGE_TMUX_SOCKET").filter(|s| !s.is_empty()),
            ui_origins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> anyhow::Result<Config> {
        let m: HashMap<_, _> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from_lookup(|k| m.get(k).cloned())
    }

    #[test]
    fn defaults() {
        let c = cfg(&[("HOME", "/home/u")]).unwrap();
        assert_eq!(c.port, 4747);
        assert_eq!(c.garage_dir, PathBuf::from("/home/u/.garage"));
        assert_eq!(c.tmux_socket, None);
        assert!(c.ui_origins.is_empty());
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
    fn bad_port_is_an_error_naming_the_variable() {
        let e = cfg(&[("HOME", "/h"), ("GARAGE_PORT", "nope")]).unwrap_err();
        assert!(e.to_string().contains("GARAGE_PORT"));
    }
}
