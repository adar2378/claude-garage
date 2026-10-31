# Design: p26-windows-linux

**Detailed design is written when the phase starts.** It depends on the `SessionBackend` trait and OS module shipped in p21. Decisions already made:

## Decisions

**1. Windows sessions run in a per-session host process.** One detached `claude-garage host` process per session: ConPTY via `portable-pty`, a `vt100` screen with 10k lines of scrollback, a loopback listener guarded by a per-host random token, registered in the garage state folder (`hosts/<id>.json`: port, token, pid). The daemon starts, lists, kills, renames and respawns hosts through the backend trait; the TUI attaches over the socket the way it attaches to tmux. This is the design from PR #2, proven in daily use on Windows 11. Alternative: WSL + tmux. Rejected: a second OS inside the user's OS, and Claude Code installs differ between the two.

**2. The host lives in core, behind the trait.** No `USE_HOST` branches in shared code: the daemon chooses a backend once at start (`tmux` on macOS/Linux, `host` on Windows, `GARAGE_BACKEND=host` to force it anywhere for testing). The host binary is a subcommand of the single `claude-garage` binary from p21.

**3. Same lifetime contract as tmux.** Hosts survive the daemon, the TUI and the terminal window; they do not survive a reboot. Sessions that died come back as restorable, exactly as with tmux.

**4. Linux uses the tmux backend unchanged.** Linux work is the OS module (notifications, picker, clipboard via the crates chosen in p21) and release binaries.

**5. Releases via cargo-dist.** Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`. The npm package installs the binary for the user's platform.

**6. Testing split.** CI (p20 matrix) runs unit and backend tests on all three OSes; host-backend tests run on every OS via `GARAGE_BACKEND=host`. Live checks: Linux in a local OrbStack Ubuntu machine (main session, no real-tmux mutation outside that machine); Windows by the contributor on Windows 11, recorded in `verification.md`.

## Open questions (resolve at phase start)

- Windows on ARM: ship `aarch64-pc-windows-msvc` too, or rely on x64 emulation?
- Should the host backend become the default on macOS/Linux one day, removing the tmux dependency? Not in this phase.

## Resolved (2026-10-31, user: all defaults)

- No Windows ARM binary yet; x64 runs under emulation.
- The host backend is Windows-only; tmux stays the backend on macOS and Linux.
- Windows is verified by hand by the contributor (Sabbir) on a branch the user names, plus CI. No local Windows VM.
