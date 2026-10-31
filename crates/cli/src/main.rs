//! The single `claude-garage` binary (design.md decision 9).
//! `tui` and `restart` are the ported launcher (`bin/garage.js`).

mod host;
mod launcher;
mod plugin;
mod tui_bin;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "claude-garage", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
enum Command {
    /// Open the TUI (default): start or reuse the daemon, then run the wall.
    Tui,
    /// Run the daemon in the foreground.
    Daemon {
        /// Port to listen on (overrides GARAGE_PORT).
        #[arg(long)]
        port: Option<u16>,
    },
    /// Restart the daemon, and optionally sessions.
    Restart {
        #[arg(long)]
        sessions: bool,
        #[arg(long)]
        all: bool,
    },
    /// Run one session host (started by the daemon, not by hand).
    #[command(hide = true)]
    Host {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Install, link, update and manage plugins.
    Plugin {
        #[command(subcommand)]
        cmd: plugin::cmd::PluginCmd,
    },
    /// A plugin's `[[cli]]` subcommand, or any unknown first word (for
    /// example a folder) handed to the plugin that declares `fallback = true`.
    #[command(external_subcommand)]
    External(Vec<String>),
}

/// The launcher talks to the daemon on `GARAGE_PORT` (default 4747).
fn real_host() -> Result<host::RealHost> {
    let config = garage_daemon::Config::from_env()?;
    Ok(host::RealHost { port: config.port })
}

fn exit_fail(f: launcher::Fail) -> i32 {
    eprintln!("{}", f.0);
    std::process::exit(1)
}

fn main() -> Result<()> {
    // Before any thread exists: the daemon scrubs its restart hand-off
    // variable from the environment here (see the function's note).
    garage_daemon::take_predecessor_env();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_cli())
}

async fn run_cli() -> Result<()> {
    // `host` takes its own flags and a `-- <command>` tail: hand them over
    // untouched instead of letting clap interpret them.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("host") {
        std::process::exit(garage_host::server::run_cli(&raw[1..]));
    }
    match Cli::parse().command.unwrap_or(Command::Tui) {
        Command::Daemon { port } => {
            let mut config = garage_daemon::Config::from_env()?;
            if let Some(p) = port {
                config.port = p;
            }
            garage_daemon::run(config).await
        }
        Command::Tui => {
            let h = real_host()?;
            std::process::exit(launcher::tui_main(&h).unwrap_or_else(exit_fail));
        }
        Command::Restart { sessions, all } => {
            let h = real_host()?;
            launcher::restart_main(&h, sessions, all).unwrap_or_else(|f| {
                exit_fail(f);
            });
            Ok(())
        }
        Command::Host { .. } => unreachable!("handled before parsing"),
        Command::Plugin { cmd } => match plugin::cmd::run(cmd) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        },
        Command::External(argv) => match plugin::cmd::dispatch_external(&argv) {
            Ok(Some(code)) => std::process::exit(code),
            Ok(None) => Cli::command()
                .error(
                    clap::error::ErrorKind::InvalidSubcommand,
                    format!("unrecognized subcommand '{}'", argv[0]),
                )
                .exit(),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Command {
        Cli::try_parse_from(std::iter::once("claude-garage").chain(args.iter().copied()))
            .unwrap()
            .command
            .unwrap_or(Command::Tui)
    }

    #[test]
    fn no_subcommand_means_tui() {
        assert_eq!(parse(&[]), Command::Tui);
    }

    #[test]
    fn daemon_port_and_restart_flags_parse() {
        assert_eq!(
            parse(&["daemon", "--port", "5001"]),
            Command::Daemon { port: Some(5001) }
        );
        assert_eq!(
            parse(&["restart", "--sessions", "--all"]),
            Command::Restart {
                sessions: true,
                all: true
            }
        );
    }

    #[test]
    fn unknown_words_reach_the_external_arm_but_core_names_never_do() {
        assert_eq!(
            parse(&["/home/me/app", "-x"]),
            Command::External(vec!["/home/me/app".into(), "-x".into()])
        );
        assert_eq!(
            parse(&["usage", "--week"]),
            Command::External(vec!["usage".into(), "--week".into()])
        );
        assert!(matches!(parse(&["plugin", "list"]), Command::Plugin { .. }));
        assert!(matches!(parse(&["restart"]), Command::Restart { .. }));
        assert!(matches!(parse(&["tui"]), Command::Tui));
    }

    #[test]
    fn out_of_range_port_is_rejected() {
        assert!(Cli::try_parse_from(["claude-garage", "daemon", "--port", "70000"]).is_err());
    }
}
