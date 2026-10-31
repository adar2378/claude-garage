//! The host backend end to end with real `claude-garage host` processes
//! (the same ones `GARAGE_BACKEND=host` starts), in a temp garage dir. No tmux.
#![cfg(unix)]

use std::path::Path;
use std::time::{Duration, Instant};

use garage_daemon::backend::host::HostBackend;
use garage_daemon::backend::SessionBackend;
use garage_protocol::AttachSpec;

fn backend(garage: &Path) -> HostBackend {
    let exe = env!("CARGO_BIN_EXE_claude-garage").to_string();
    HostBackend::new(garage.join("hosts"), vec![exe, "host".into()])
}

async fn eventually<F: std::future::Future<Output = bool>>(what: &str, mut f: impl FnMut() -> F) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Stops any host the test left behind, pass or fail.
struct Cleanup(HostBackend);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let b = self.0.clone();
        let _ = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                for s in b.list().await.unwrap_or_default() {
                    let _ = b.kill(&s.id).await;
                }
            })
        })
        .join();
    }
}

#[tokio::test]
async fn lifecycle_create_list_has_rename_capture_kill() {
    let g = tempfile::tempdir().unwrap();
    let b = backend(g.path());
    let _cleanup = Cleanup(b.clone());
    b.init().await.unwrap();
    assert!(b.list().await.unwrap().is_empty());
    assert!(!b.has("garage/ws/a").await.unwrap());

    let work = g.path().canonicalize().unwrap();
    let args = vec!["-c".to_string(), "echo $GARAGE_T marker-xyz; sleep 60".to_string()];
    b.create("garage/ws/a", &work, "sh", &args, &[("GARAGE_T".into(), "hello".into())]).await.unwrap();

    let list = b.list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "garage/ws/a");
    assert_eq!(Path::new(&list[0].dir), work);
    assert!(!list[0].attached);
    assert!(list[0].created_at.is_some_and(|c| c > 1_600_000_000_000));
    assert!(b.has("garage/ws/a").await.unwrap());
    assert!(!b.has("garage/ws").await.unwrap(), "exact match only");

    let panes = b.panes().await.unwrap();
    assert!(panes["garage/ws/a"].pid.is_some_and(|p| p > 1));
    assert_eq!(Path::new(&panes["garage/ws/a"].cwd), work);

    // env reaches the command; capture sees its output
    eventually("the command's output", || async { b.capture("garage/ws/a", 20).await.is_ok_and(|t| t.contains("hello marker-xyz")) }).await;

    let dup = b.create("garage/ws/a", &work, "sleep", &["60".into()], &[]).await.unwrap_err();
    assert!(dup.to_string().contains("duplicate"), "{dup}");

    b.rename("garage/ws/a", "garage/ws/b").await.unwrap();
    assert!(!b.has("garage/ws/a").await.unwrap());
    assert!(b.has("garage/ws/b").await.unwrap());
    assert_eq!(b.attach_spec("garage/ws/b"), AttachSpec::Host { id: "garage/ws/b".into() });
    assert_eq!(b.attach_spec("garage/ws/a"), AttachSpec::Host { id: String::new() }, "the old name is gone");

    b.kill("garage/ws/b").await.unwrap();
    assert!(b.list().await.unwrap().is_empty());
    assert!(b.kill("garage/ws/b").await.is_err());
    assert!(b.capture("garage/ws/b", 5).await.is_err());
}

#[tokio::test]
async fn respawn_swaps_the_process_under_the_same_id_and_fails_for_a_missing_one() {
    let g = tempfile::tempdir().unwrap();
    let b = backend(g.path());
    let _cleanup = Cleanup(b.clone());
    let work = g.path().canonicalize().unwrap();
    b.create("garage/ws/r", &work, "sleep", &["60".into()], &[]).await.unwrap();
    let before = b.panes().await.unwrap()["garage/ws/r"].pid.unwrap();
    b.respawn("garage/ws/r", &work, "sleep", &["61".into()], &[]).await.unwrap();
    let after = b.panes().await.unwrap()["garage/ws/r"].pid.unwrap();
    assert_ne!(before, after);
    assert_eq!(b.list().await.unwrap().len(), 1);
    assert!(b.respawn("garage/ws/none", &work, "sleep", &["60".into()], &[]).await.is_err());
}

#[tokio::test]
async fn a_session_whose_command_exits_disappears_and_a_stale_record_is_pruned() {
    let g = tempfile::tempdir().unwrap();
    let b = backend(g.path());
    let _cleanup = Cleanup(b.clone());
    let work = g.path().canonicalize().unwrap();
    b.create("garage/ws/short", &work, "sh", &["-c".into(), "sleep 1".into()], &[]).await.unwrap();
    eventually("the host to exit with its command", || async { !b.has("garage/ws/short").await.unwrap() }).await;

    // A record left by a host that was killed hard: nothing listens on its port.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    garage_host::registry::write(
        &g.path().join("hosts"),
        &garage_host::registry::Entry {
            id: "garage/ws/ghost".into(), pid: 1, child_pid: 1, port, token: "t".repeat(32),
            dir: "/w".into(), created_at: 1, title: String::new(),
        },
    )
    .unwrap();
    assert!(b.list().await.unwrap().is_empty(), "a dead record is not a session");
    assert!(garage_host::registry::read(&g.path().join("hosts"), "garage/ws/ghost").is_none(), "and it is deleted");
}

#[tokio::test]
async fn a_host_that_cannot_start_reports_why() {
    let g = tempfile::tempdir().unwrap();
    let b = backend(g.path());
    let e = b.create("garage/ws/bad", g.path(), "/nonexistent/program-for-tests", &[], &[]).await.unwrap_err();
    assert!(e.to_string().contains("session host"), "{e}");
    assert!(b.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn attached_reflects_a_connected_client() {
    let g = tempfile::tempdir().unwrap();
    let b = backend(g.path());
    let _cleanup = Cleanup(b.clone());
    b.create("garage/ws/at", g.path(), "sleep", &["60".into()], &[]).await.unwrap();
    let e = garage_host::registry::read(&g.path().join("hosts"), "garage/ws/at").unwrap();
    let _client = garage_host::proto::connect_attach(&e.addr(), &e.token, 80, 24).unwrap();
    eventually("attached=true", || async { b.list().await.unwrap()[0].attached }).await;
    drop(_client);
    eventually("attached=false", || async { !b.list().await.unwrap()[0].attached }).await;
}
