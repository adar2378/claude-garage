//! `openspec-board`: a read-only overlay of the OpenSpec changes of a garage
//! workspace. This group has the local changes only: parsers, config, the
//! local collector and the pane. Everything that decides something is a plain
//! function, tested without a daemon or a terminal.

pub mod config;
pub mod fetch;
pub mod local;
pub mod pane;
pub mod parser;
pub mod remote;

#[cfg(test)]
mod manifest_tests {
    //! The manifest is checked against what group 1 may declare: a pane, its
    //! action and a build step, only `ui.panes`, nothing else. (`claude-garage
    //! plugin schema` takes no argument; `plugin link` validates the manifest
    //! too, and is the by-hand check.)

    fn manifest() -> toml::Table {
        include_str!("../garage-plugin.toml").parse().unwrap()
    }

    #[test]
    fn manifest_declares_the_peek_pane_and_nothing_ahead_of_it() {
        let m = manifest();
        assert_eq!(m["id"].as_str(), Some("openspec-board"));
        assert_eq!(m["api_version"].as_integer(), Some(1));
        let perms: Vec<_> = m["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(perms, ["ui.panes"]);
        // Nothing declared ahead of the group that implements it.
        for key in ["widgets", "events", "cli", "release", "requires"] {
            assert!(m.get(key).is_none(), "{key} must not be declared yet");
        }
        let pane = &m["panes"].as_array().unwrap()[0];
        assert_eq!(pane["id"].as_str(), Some("board"));
        assert_eq!(pane["placement"].as_str(), Some("overlay"));
        let cmd: Vec<_> = pane["command"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(cmd, ["{plugin_dir}/target/release/openspec-board", "ui"]);
        let action = &m["actions"].as_array().unwrap()[0];
        assert_eq!(action["pane"].as_str(), Some("board"));
        assert_eq!(action["default_key"].as_str(), Some("o"));
        let build: Vec<_> = m["build"].as_array().unwrap()[0]["command"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(build, ["cargo", "build", "--release"]);
        assert!(!include_str!("../garage-plugin.toml").contains("{bin}"));
    }

    #[test]
    fn the_crate_and_the_manifest_share_a_version() {
        assert_eq!(
            manifest()["version"].as_str(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
}
