//! `pit-pet update | started | cycle | pet | clear <rev> <delay_ms>`.

use std::process::{Command, Stdio};

use garage_plugin_sdk::{Api, read_context, state_dir};
use pit_pet::run::{self, Env};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// `date +%H`; noon when the shell-out fails, so a broken clock never fires
/// the late-night lines all day.
fn local_hour() -> u8 {
    Command::new("date")
        .arg("+%H")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(12)
}

/// Start `pit-pet clear <rev> <delay>` detached, with no pipes, so the run
/// that scheduled it ends at once and the host does not wait for the caption.
fn schedule_clear(rev: u64, delay_ms: i64) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe)
        .args(["clear", &rev.to_string(), &delay_ms.max(0).to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let result = (|| {
        let api = Api::from_env()?;
        let dir = state_dir()?;
        if cmd == "clear" {
            let rev: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let delay: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            std::thread::sleep(std::time::Duration::from_millis(delay));
            let env = Env {
                api: &api,
                dir: &dir,
                now: now_ms(),
                context: &garage_plugin_sdk::Value::Null,
                schedule_clear: &|_, _| {},
                local_hour: &local_hour,
            };
            return run::clear_caption(&env, rev);
        }
        let context = read_context()?;
        let env = Env {
            api: &api,
            dir: &dir,
            now: now_ms(),
            context: &context,
            schedule_clear: &schedule_clear,
            local_hour: &local_hour,
        };
        match cmd {
            "update" => run::update(&env),
            "started" => run::started(&env),
            "cycle" => run::cycle(&env),
            "pet" => run::pet(&env),
            other => {
                eprintln!("pit-pet: unknown command {other:?} (update, started, cycle, pet)");
                std::process::exit(2);
            }
        }
    })();
    if let Err(e) = result {
        eprintln!("pit-pet: {e}");
        std::process::exit(1);
    }
}
