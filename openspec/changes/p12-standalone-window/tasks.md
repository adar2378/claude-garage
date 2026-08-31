# Tasks: p12-standalone-window

- [x] 1. `GarageCommand::OpenWindow` mapped from `"t"`, declined by the store
      like `Spawn`/`Close`; router shows "no live session to open" for a
      restorable/dead focus and "standalone windows: macOS only for now" off
      macOS.
- [x] 2. `wall/src/ui/window_open.rs`: pure argv builders for iTerm2 and
      Terminal.app (AppleScript via `osascript`), a shell/AppleScript
      escaping helper with adversarial-id unit tests, and the detached
      `Command::spawn` launch wired through `Effect::OpenWindow` in
      `runtime.rs` — success/failure strip notices.
- [x] 3. Help overlay gains the `t` row; one-time manual live smoke (scratch
      daemon + scratch session + scratch tmux outer): press `t`, confirm
      `tmux list-clients` gains a client, clean up.
