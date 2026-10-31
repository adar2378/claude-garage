//! `cli-extras open|install|prune`: the `[[cli]]` commands behind
//! `claude-garage <folder>`, `claude-garage install` and `claude-garage prune`.

use garage_plugin_cli_extras as lib;
use garage_plugin_sdk::{Api, Context};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("open") => open(),
        Some("install") => report(lib::install),
        Some("prune") => prune(
            args.iter().any(|a| a == "--dry-run"),
            args.iter().any(|a| a == "--yes" || a == "-y"),
        ),
        other => {
            eprintln!("cli-extras: unknown command {other:?} (open | install | prune)");
            2
        }
    };
    std::process::exit(code);
}

fn api() -> Result<Api, String> {
    Api::from_env().map_err(|e| e.to_string())
}

fn report(f: impl FnOnce(&Api) -> Vec<String>) -> i32 {
    let api = match api() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cli-extras: {e}");
            return 1;
        }
    };
    let lines = f(&api);
    let failed = lines.iter().any(|l| l.contains("FAILED"));
    for l in lines {
        println!("{l}");
    }
    i32::from(failed)
}

fn prune(dry_run: bool, yes: bool) -> i32 {
    use std::io::{BufRead, IsTerminal, Write};
    let api = match api() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cli-extras: {e}");
            return 1;
        }
    };
    let plan = match lib::plan_prune(&api) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cli-extras: {e}");
            return 1;
        }
    };
    if !plan.keep_for_history.is_empty() {
        println!(
            "keeping {} (no sessions, but closed chats you can still restore)",
            plan.keep_for_history.join(", ")
        );
    }
    if plan.remove.is_empty() {
        println!("no empty projects");
        return 0;
    }
    let verb = if dry_run {
        "would remove"
    } else {
        "will remove"
    };
    for n in &plan.remove {
        println!("{verb} {n}");
    }
    if dry_run {
        return 0;
    }
    match lib::confirm_mode(yes, std::io::stdin().is_terminal()) {
        lib::Confirm::Go => {}
        lib::Confirm::Refuse => {
            eprintln!("cli-extras: not removing anything without --yes (or a terminal to ask on)");
            return 1;
        }
        lib::Confirm::Ask => {
            print!("Remove {} project(s)? [y/N] ", plan.remove.len());
            let _ = std::io::stdout().flush();
            let mut answer = String::new();
            if std::io::stdin().lock().read_line(&mut answer).is_err() || !lib::is_yes(&answer) {
                println!("nothing removed");
                return 0;
            }
        }
    }
    match lib::prune(&api, &plan.remove) {
        Ok(done) => {
            for (name, what) in done {
                match what {
                    lib::Pruned::Removed => println!("removed {name}"),
                    lib::Pruned::Kept(why) => println!("kept {name}: {why}"),
                }
            }
            0
        }
        Err(e) => {
            eprintln!("cli-extras: {e}");
            1
        }
    }
}

fn open() -> i32 {
    let ctx = match Context::read() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cli-extras: {e}");
            return 1;
        }
    };
    let words = ctx.argv();
    let Some(req) = lib::parse_open(&words) else {
        eprintln!("usage: claude-garage <folder> [--no-open]");
        return 2;
    };
    let cwd = ctx
        .cwd()
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let dir = match garage_plugin_sdk::folder::resolve_folder(&req.word, &cwd, home.as_deref()) {
        Ok(d) => d,
        Err(garage_plugin_sdk::folder::FolderError::NotADirectory) => {
            if req.fallback {
                // A fallback swallows every typo: give the usual clap error.
                eprintln!(
                    "error: unrecognized subcommand '{}'\n\nUsage: claude-garage [COMMAND]\n\nFor more information, try '--help'.",
                    req.word
                );
                return 2;
            }
            eprintln!("not a directory: {}", req.word);
            return 1;
        }
        Err(e) => {
            eprintln!("{}: {e}", req.word);
            return 1;
        }
    };
    let api = match api() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cli-extras: {e}");
            return 1;
        }
    };
    let opened = match lib::open_folder(&api, &dir) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cli-extras: {e}");
            return 1;
        }
    };
    println!("{}", opened.summary());
    if req.no_open {
        return 0;
    }
    // The launcher opens the wall once this command has exited: this plugin
    // starts nothing itself.
    match garage_plugin_sdk::request_wall() {
        Ok(true) => 0,
        Ok(false) => {
            println!("run `claude-garage` to open the wall");
            0
        }
        Err(e) => {
            eprintln!("cli-extras: {e}");
            1
        }
    }
}
