//! Process-group control for plugin runs (design section 4): every run starts
//! in its own group so a timeout or an uninstall can kill the whole tree. The
//! only place that knows how; other OSes get their own implementation with
//! p26.

/// Start the child in a new process group (it becomes the group leader).
#[cfg(unix)]
pub fn own_group(cmd: &mut tokio::process::Command) {
    cmd.process_group(0);
}

#[cfg(not(unix))]
pub fn own_group(_cmd: &mut tokio::process::Command) {}

/// Signal every process in the group led by `pid`: SIGTERM, or SIGKILL when
/// `kill`. Errors (already gone) are ignored.
#[cfg(unix)]
pub fn signal_group(pid: u32, kill: bool) {
    let Ok(pid) = i32::try_from(pid) else { return };
    if pid <= 1 {
        return; // never signal init or "every process"
    }
    // SAFETY: plain kill(2) on a negative pid, which addresses a process group.
    unsafe {
        libc::kill(-pid, if kill { libc::SIGKILL } else { libc::SIGTERM });
    }
}

#[cfg(not(unix))]
pub fn signal_group(_pid: u32, _kill: bool) {}
