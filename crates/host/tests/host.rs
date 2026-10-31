//! Real host (in a thread of this process) running real commands on a real
//! PTY, driven over the loopback protocol. No tmux, no daemon.
#![cfg(unix)]

use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use garage_host::{proto, registry, server};

fn spawn_host(hosts: &Path, id: &str, command: &[&str]) -> (registry::Entry, std::thread::JoinHandle<i32>) {
    let opts = server::Options {
        id: id.into(),
        dir: std::env::temp_dir().to_string_lossy().into_owned(),
        hosts_dir: hosts.to_path_buf(),
        cols: 80,
        rows: 10,
        command: command.iter().map(|s| s.to_string()).collect(),
    };
    let handle = std::thread::spawn(move || server::run(opts).unwrap());
    let entry = wait_for(|| registry::read(hosts, id), "the host's registry file");
    (entry, handle)
}

fn wait_for<T>(mut f: impl FnMut() -> Option<T>, what: &str) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn read_until(s: &mut std::net::TcpStream, needle: &str, seen: &mut String) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buf = [0u8; 4096];
    while !seen.contains(needle) {
        assert!(Instant::now() < deadline, "never saw {needle:?} in {seen:?}");
        match s.read(&mut buf) {
            Ok(n) if n > 0 => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
            _ => {}
        }
    }
}

#[test]
fn info_reports_the_session_and_a_wrong_token_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let (e, host) = spawn_host(d.path(), "garage/ws/a", &["sleep", "60"]);
    let info = proto::info(&e.addr(), &e.token).unwrap();
    assert_eq!((info.id.as_str(), info.pid, info.child_pid, info.attached), ("garage/ws/a", e.pid, e.child_pid, 0));
    assert!(info.child_pid > 1 && info.created_at > 1_600_000_000_000);

    let bad = "0".repeat(proto::TOKEN_LEN);
    assert!(proto::info(&e.addr(), &bad).is_err(), "a wrong token must not get an answer");

    proto::shutdown(&e.addr(), &e.token).unwrap();
    host.join().unwrap();
    assert_eq!(registry::read(d.path(), "garage/ws/a"), None, "the registry file goes with the host");
    assert!(proto::info(&e.addr(), &e.token).is_err(), "nothing listens after shutdown");
}

#[test]
fn text_capture_history_and_formatted_capture_see_what_the_command_printed() {
    let d = tempfile::tempdir().unwrap();
    // 30 lines on a 10-row screen: most of them scroll into history.
    let (e, host) = spawn_host(d.path(), "garage/ws/c", &["sh", "-c", "i=0; while [ $i -lt 30 ]; do echo row-$i; i=$((i+1)); done; sleep 60"]);
    wait_for(|| proto::capture_text(&e.addr(), &e.token, 100).ok().filter(|t| t.contains("row-29")), "the output");

    let text = proto::capture_text(&e.addr(), &e.token, 100).unwrap();
    assert!(text.starts_with("row-0\n") && text.ends_with("row-29"), "{text:?}");
    assert_eq!(proto::capture_text(&e.addr(), &e.token, 2).unwrap(), "row-28\nrow-29");

    let hist = proto::history_size(&e.addr(), &e.token).unwrap();
    assert!(hist >= 20, "history {hist}");
    let top = proto::capture_formatted(&e.addr(), &e.token, 0, 80, 2).unwrap();
    let top = String::from_utf8_lossy(&top).into_owned();
    assert!(top.contains("row-0") && top.contains("row-1"), "{top:?}");

    proto::shutdown(&e.addr(), &e.token).unwrap();
    host.join().unwrap();
}

#[test]
fn attach_gets_the_screen_then_live_output_and_its_input_reaches_the_command() {
    let d = tempfile::tempdir().unwrap();
    let (e, host) = spawn_host(d.path(), "garage/ws/t", &["sh", "-c", "echo before-attach; cat"]);
    wait_for(|| proto::capture_text(&e.addr(), &e.token, 10).ok().filter(|t| t.contains("before-attach")), "the first output");

    let mut s = proto::connect_attach(&e.addr(), &e.token, 80, 10).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let mut seen = String::new();
    read_until(&mut s, "before-attach", &mut seen); // the snapshot
    assert_eq!(proto::info(&e.addr(), &e.token).unwrap().attached, 1);

    proto::write_frame(&mut s, proto::F_INPUT, b"typed-by-the-tui\n").unwrap();
    read_until(&mut s, "typed-by-the-tui", &mut seen); // echoed by the pty / cat
    proto::write_frame(&mut s, proto::F_RESIZE, &[100, 0, 12, 0]).unwrap();

    drop(s);
    wait_for(|| (proto::info(&e.addr(), &e.token).unwrap().attached == 0).then_some(()), "the detach to register");

    proto::shutdown(&e.addr(), &e.token).unwrap();
    host.join().unwrap();
}

#[test]
fn rename_moves_the_registry_record_and_keeps_the_host_running() {
    let d = tempfile::tempdir().unwrap();
    let (e, host) = spawn_host(d.path(), "garage/ws/old", &["sleep", "60"]);
    proto::rename(&e.addr(), &e.token, "garage/ws/new").unwrap();
    wait_for(|| registry::read(d.path(), "garage/ws/new"), "the renamed record");
    assert_eq!(registry::read(d.path(), "garage/ws/old"), None);
    assert_eq!(proto::info(&e.addr(), &e.token).unwrap().id, "garage/ws/new");
    proto::shutdown(&e.addr(), &e.token).unwrap();
    host.join().unwrap();
    assert_eq!(registry::read(d.path(), "garage/ws/new"), None);
}

#[test]
fn a_command_that_exits_takes_its_host_and_registry_record_with_it() {
    let d = tempfile::tempdir().unwrap();
    let (e, host) = spawn_host(d.path(), "garage/ws/x", &["sh", "-c", "exit 3"]);
    assert_eq!(host.join().unwrap(), 3, "the host exits with the command's code");
    assert_eq!(registry::read(d.path(), "garage/ws/x"), None);
    assert!(proto::info(&e.addr(), &e.token).is_err());
}

#[test]
fn the_title_the_command_sets_reaches_info_and_the_registry() {
    let d = tempfile::tempdir().unwrap();
    let (e, host) = spawn_host(d.path(), "garage/ws/ti", &["sh", "-c", "printf '\\033]0;my-title\\007'; sleep 60"]);
    wait_for(|| proto::info(&e.addr(), &e.token).ok().filter(|i| i.title == "my-title"), "the OSC title");
    wait_for(|| registry::read(d.path(), "garage/ws/ti").filter(|r| r.title == "my-title"), "the title in the registry");
    proto::shutdown(&e.addr(), &e.token).unwrap();
    host.join().unwrap();
}

#[test]
fn send_text_reaches_the_command_as_a_submitted_line() {
    let d = tempfile::tempdir().unwrap();
    // `read` only returns once Enter arrives, so seeing the echo proves both parts landed.
    let (e, host) = spawn_host(d.path(), "garage/ws/q", &["sh", "-c", "read line; echo got:$line; sleep 60"]);
    proto::send_text(&e.addr(), &e.token, "quick message").unwrap();
    wait_for(|| proto::capture_text(&e.addr(), &e.token, 10).ok().filter(|t| t.contains("got:quick message")), "the submitted line");
    proto::shutdown(&e.addr(), &e.token).unwrap();
    host.join().unwrap();
}
