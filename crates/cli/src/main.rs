//! The single `claude-garage` binary (design.md decision 9).
//! Only `daemon` does anything yet; the other subcommands are stubs.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

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
    /// Reserved for the plugin host.
    Plugin,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command.unwrap_or(Command::Tui) {
        Command::Daemon { port } => {
            let mut config = garage_daemon::Config::from_env()?;
            if let Some(p) = port {
                config.port = p;
            }
            garage_daemon::run(config).await
        }
        Command::Tui => bail!("`tui` is not ported yet; use the npm launcher"),
        Command::Restart { .. } => bail!("`restart` is not ported yet; use the npm launcher"),
        Command::Plugin => bail!("plugins: coming in a later release"),
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
        assert_eq!(parse(&["daemon", "--port", "5001"]), Command::Daemon { port: Some(5001) });
        assert_eq!(parse(&["restart", "--sessions", "--all"]), Command::Restart { sessions: true, all: true });
    }

    #[test]
    fn out_of_range_port_is_rejected() {
        assert!(Cli::try_parse_from(["claude-garage", "daemon", "--port", "70000"]).is_err());
    }
}
