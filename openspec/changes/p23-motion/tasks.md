# Tasks: p23-motion

## 1. Motion module (`wall/src/ui/motion.rs`)

- [x] 1.1 Add `tachyonfx`, `throbber-widgets-tui`, `tui-big-text`, pinned
- [x] 1.2 Motion clock: request redraws only while an effect is running
- [x] 1.3 Off switch: config `animations = false`, `GARAGE_NO_MOTION`, `NO_MOTION`, `REDUCE_MOTION`

## 2. Effects

- [x] 2.1 Startup wordmark reveal, only while the launcher is starting the daemon
- [x] 2.2 Spinners: tile attach and restore, strip during daemon restart
- [x] 2.3 `done` fades over 2 minutes
- [x] 2.4 Tile fade-in on attach, dissolve on close
- [x] 2.5 One amber pulse when a session becomes `needs-input`, nothing more

## 3. Tests

- [x] 3.1 Unit: no redraw request when no effect runs
- [x] 3.2 Unit: with motion off, no effect starts and spinners render static
- [x] 3.3 Snapshot: `done` fade at 0, 60 and 120 seconds

## 4. Verification

- [x] 4.1 `cargo test --lib --bins`, clippy green
- [ ] 4.2 Live (user's wall, ask first): startup reveal, attach spinner, done fade; idle CPU of the TUI unchanged from before
- [ ] 4.3 Typing into an engaged tile during an effect shows no added delay
- [ ] 4.4 Record results in `verification.md`
