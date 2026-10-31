# Tasks: p26-windows-linux

Rough; refined when the phase starts.

## 1. Windows backend

- [x] 1.1 Port PR #2's session host (`host.rs`) into a host crate / `claude-garage host` subcommand
- [x] 1.2 Implement `SessionBackend` for the host: create, list, kill, rename, respawn, capture, attach
- [x] 1.3 TUI attach and scrollback through the backend (no tmux calls on Windows)
- [x] 1.4 Backend selection at daemon start (`tmux` / `host`, `GARAGE_BACKEND` override)
- [x] 1.5 Host-backend tests runnable on every OS

## 2. Windows platform module

- [x] 2.1 Toast notifications, folder picker (clipboard stays Unsupported on every OS: nothing reads it)
- [x] 2.2 Paths (`USERPROFILE`), `claude.cmd` shim resolution, hidden console windows (drive letters and `~\` expansion in the add-project form not done)
- [ ] 2.3 Locked-binary update handling in the launcher
- [x] 2.4 (written, unverified on Windows) Shift+Enter and Ctrl+Enter through ConPTY

## 3. Linux

- [x] 3.1 Notifications and picker in the OS module (clipboard Unsupported)
- [x] 3.2 OrbStack Ubuntu machine setup notes for local testing (docs/PLATFORMS.md)

## 4. Release

- [ ] 4.1 cargo-dist targets for macOS, Linux and Windows; npm package selects the binary
- [x] 4.2 Prerequisite checks per OS (no tmux on Windows)

## 5. Verification

- [ ] 5.1 CI matrix green on macOS, Linux and Windows
- [ ] 5.2 Live check on Linux (OrbStack): create, attach, close, restore, notifications
- [ ] 5.3 Live check on Windows 11 by the contributor: create, attach, hard-kill then restore, keys
- [ ] 5.4 Record results in `verification.md`
