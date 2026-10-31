//! `cli-extras open|install|prune`: the `[[cli]]` commands behind
//! `claude-garage <folder>`, `claude-garage install` and `claude-garage prune`.

use garage_plugin_cli_extras as lib;
use garage_plugin_sdk::{Api, Context};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("open") => open(),
        Some("install") => report(lib::install),
        Some("prune") => prune(args.iter().any(|a| a == "--dry-run")),
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

fn prune(dry_run: bool) -> i32 {
    let api = match api() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cli-extras: {e}");
            return 1;
        }
    };
    match lib::prune(&api, dry_run) {
        Ok(names) if names.is_empty() => {
            println!("no empty projects");
            0
        }
        Ok(names) => {
            let verb = if dry_run { "would remove" } else { "removed" };
            for n in names {
                println!("{verb} {n}");
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
    let Some(dir) = lib::resolve_dir(&req.word, &cwd, home.as_deref()) else {
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
    match std::env::var_os("GARAGE_BIN") {
        Some(bin) => std::process::Command::new(bin)
            .status()
            .map(|s| s.code().unwrap_or(1))
            .unwrap_or_else(|e| {
                eprintln!("cli-extras: cannot start claude-garage: {e}");
                1
            }),
        None => {
            println!("run `claude-garage` to open the wall");
            0
        }
    }
}
