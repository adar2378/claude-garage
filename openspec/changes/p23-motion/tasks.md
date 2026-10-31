# Tasks: p23-motion

## 1. Motion module (`wall/src/ui/motion.rs`)

- [ ] 1.1 Add `tachyonfx`, `throbber-widgets-tui`, `tui-big-text`, pinned
- [ ] 1.2 Motion clock: request redraws only while an effect is running
- [ ] 1.3 Off switch: config `animations = false`, `GARAGE_NO_MOTION`, `NO_MOTION`, `REDUCE_MOTION`

## 2. Effects

- [ ] 2.1 Startup wordmark reveal, only while the launcher is starting the daemon
- [ ] 2.2 Spinners: tile attach and restore, strip during daemon restart
- [ ] 2.3 `done` fades over 2 minutes
- [ ] 2.4 Tile fade-in on attach, dissolve on close
- [ ] 2.5 One amber pulse when a session becomes `needs-input`, nothing more

## 3. Tests

- [ ] 3.1 Unit: no redraw request when no effect runs
- [ ] 3.2 Unit: with motion off, no effect starts and spinners render static
- [ ] 3.3 Snapshot: `done` fade at 0, 60 and 120 seconds

## 4. Verification

- [ ] 4.1 `cargo test --lib --bins`, clippy green
- [ ] 4.2 Live (user's wall, ask first): startup reveal, attach spinner, done fade; idle CPU of the TUI unchanged from before
- [ ] 4.3 Typing into an engaged tile during an effect shows no added delay
- [ ] 4.4 Record results in `verification.md`
