## Why

garage runs only on macOS. Sessions live in tmux, which has no native Windows build, and several features (notifications, folder picker, clipboard) are macOS-only. Only a macOS arm64 binary ships. Linux users get a self-build path with missing features; Windows users get nothing. A contributor (PR #2) proved a native Windows design works in daily use; it needs to land in core behind the session backend seam from p21, not as `if windows` branches through shared code.

## What Changes

- **Windows session backend.** A per-session detached host process runs `claude` on a ConPTY, keeps a vt100 screen and scrollback, and listens on loopback for attach. Sessions outlive the daemon and the terminal, like tmux. It implements the p21 `SessionBackend` trait; the tmux backend stays the default on macOS and Linux.
- **Windows platform module.** Toast notifications, folder picker, clipboard, `%USERPROFILE%`/drive-letter paths, `claude.cmd` shim resolution, no console-window flashes, locked-binary update handling.
- **Keys through ConPTY.** Shift+Enter inserts a newline; Ctrl+Enter sends a queued message now.
- **Linux parity.** tmux backend as-is; the OS module provides notifications, picker and clipboard on Linux.
- **Release binaries** for macOS arm64/x64, Linux x64/arm64 and Windows x64, built by cargo-dist; the npm package picks the right one.
- **Testing.** CI runs the full matrix on every PR; Linux is checked live in a local OrbStack Ubuntu machine; Windows is checked live by the contributor on Windows 11.

## Capabilities

### New Capabilities
- `platform-support`: supported operating systems, session backend per OS, per-OS behaviour of notifications, picker, clipboard and keys.

### Modified Capabilities
- `packaging`: prerequisites depend on the OS (tmux not required on Windows); prebuilt binaries for all supported targets.

## Impact

- **Code:** new Windows backend and host crate (from PR #2's `host.rs`, reworked behind the trait); OS module implementations for Windows and Linux; launcher and release config.
- **Depends on:** p21 (Rust daemon, `SessionBackend` trait, OS module), p20 (three-OS CI).
- **Not in scope:** PR #2's shared UI features (closed chats, usage bar, quick send, turn markers, draft recovery). Those land as plugins in p25, or in core where p25 says so.
