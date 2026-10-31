# Platforms

| OS | Session backend | Notifications | Folder picker | Prebuilt binary |
|---|---|---|---|---|
| macOS arm64 / x64 | tmux | terminal-notifier or osascript | osascript | arm64 ships today |
| Linux x64 / arm64 | tmux | `notify-send` if installed | `zenity` or `kdialog` if installed | planned (cargo-dist) |
| Windows x64 | session host (below) | PowerShell toast | PowerShell WinForms dialog | planned (cargo-dist); x64 runs under emulation on ARM |

A feature the OS lacks answers 501 and the TUI says so; nothing else changes.

## Session host (Windows, or `GARAGE_BACKEND=host` anywhere)

The daemon picks its backend once at start (`GARAGE_BACKEND=tmux|host`; unset
means tmux on macOS and Linux, host on Windows). With the host backend each
session is one detached `claude-garage host` process:

- it runs `claude` on a ConPTY (a plain PTY on Unix), mirrors the output into a
  vt100 screen with 10,000 lines of scrollback, and listens on 127.0.0.1;
- it writes `<garage dir>/hosts/<id>.json` (port, token, pid). The directory is
  0700 and the files 0600 on Unix; on Windows the per-user profile directory is
  the protection. File names escape every byte outside `[A-Za-z0-9_-]` as `%XX`;
- every connection starts with the token; a control connection answers info,
  history, capture, rename and shutdown;
- hosts outlive the daemon, the TUI and the terminal, but not a reboot. A record
  whose port refuses connections is stale and is removed by the daemon.

The daemon never sends the token over HTTP. `GET /api/attach?id=` answers
`{"kind":"tmux","socket":"name"}` or `{"kind":"host","id":"..."}`; the TUI builds
the tmux command itself (socket names are `[A-Za-z0-9_-]{1,64}`) or reads the
host's record from the garage directory, and only dials loopback addresses.

Every daemon route also checks the `Host` header (127.0.0.1, localhost or [::1]
on the daemon's port) so a web page cannot reach it by DNS rebinding.

Testing the host backend on macOS or Linux:

```sh
GARAGE_BACKEND=host claude-garage daemon
cargo test -p garage-host -p claude-garage   # real host processes in temp dirs, no tmux
```

## Linux packaging notes

- Build targets: `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. The
  workspace is pure Rust (no C dependency): `cargo check --workspace --all-targets
  --target <triple>` passes from macOS with only `rustup target add`.
- Runtime: `tmux` (required), `claude`; optional `notify-send` (libnotify-bin),
  `zenity` or `kdialog`.
- Binaries to ship per target: `claude-garage` and `garage-wall` side by side
  (the launcher falls back to a `garage-wall` beside itself); the npm package
  names the TUI `wall/dist/garage-wall-linux-x64` / `garage-wall-linux-arm64`
  (`process.platform-process.arch`). `build:tui` in package.json produces that name.
- glibc: build on the oldest distro you support (e.g. an Ubuntu 22.04 runner) or
  the binary needs the newer glibc of the build machine.
- Live check (task 5.2) in an OrbStack Ubuntu machine: `sudo apt install tmux
  libnotify-bin zenity`, build, then create, attach, close and restore a session
  and trigger a needs-input notification.

## Windows: what a contributor should check by hand

Compiles for `x86_64-pc-windows-msvc` (`cargo check --workspace --all-targets`) but
has not been run on Windows:

1. `claude-garage` starts the daemon detached (no console window) and the TUI.
2. `n` creates a session: a `claude-garage host` process appears, `claude` (an npm
   `claude.cmd` shim or `claude.exe`) runs in it, the tile shows it.
3. Typing, Shift+Enter (newline), Ctrl+Enter (send now), resize, scrollback.
4. Quit the TUI: sessions keep running. Kill a host in Task Manager: the session
   shows as restorable.
5. Toast on needs-input, the folder picker, `x x` (taskkill of the tree).
6. `claude agents --json` is run by name: confirm it resolves when `claude` is a `.cmd` shim.
