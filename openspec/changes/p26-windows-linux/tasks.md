# Tasks: p26-windows-linux

Rough; refined when the phase starts.

## 1. Windows backend

- [ ] 1.1 Port PR #2's session host (`host.rs`) into a host crate / `claude-garage host` subcommand
- [ ] 1.2 Implement `SessionBackend` for the host: create, list, kill, rename, respawn, capture, attach
- [ ] 1.3 TUI attach and scrollback through the backend (no tmux calls on Windows)
- [ ] 1.4 Backend selection at daemon start (`tmux` / `host`, `GARAGE_BACKEND` override)
- [ ] 1.5 Host-backend tests runnable on every OS

## 2. Windows platform module

- [ ] 2.1 Toast notifications, folder picker, clipboard
- [ ] 2.2 Paths (`USERPROFILE`, drive letters, `~\`), `claude.cmd` shim resolution, hidden console windows
- [ ] 2.3 Locked-binary update handling in the launcher
- [ ] 2.4 Shift+Enter and Ctrl+Enter through ConPTY

## 3. Linux

- [ ] 3.1 Notifications, picker, clipboard in the OS module
- [ ] 3.2 OrbStack Ubuntu machine setup notes for local testing

## 4. Release

- [ ] 4.1 cargo-dist targets for macOS, Linux and Windows; npm package selects the binary
- [ ] 4.2 Prerequisite checks per OS (no tmux on Windows)

## 5. Verification

- [ ] 5.1 CI matrix green on macOS, Linux and Windows
- [ ] 5.2 Live check on Linux (OrbStack): create, attach, close, restore, notifications
- [ ] 5.3 Live check on Windows 11 by the contributor: create, attach, hard-kill then restore, keys
- [ ] 5.4 Record results in `verification.md`
