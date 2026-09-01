# Tasks: p13-title-as-name

- [x] 1. `WallSession::display_name()` — title (non-empty) else label
- [x] 2. `tile.rs::title_line` — name in the label slot, fitted; trailing dim label id; ladder per spec
- [x] 3. `rail.rs::session_row` — display_name in the row
- [x] 4. `triage.rs::row_line` — identity uses display_name; drop the now-duplicate trailing subtitle
- [x] 5. Update unit tests in all three modules
- [x] 6. `cargo test` green (387 passed); rebuilt `wall/dist` binary; live TUI spot-check: tile bar rendered `◐ ✳ George employment history ⎇ main 1:19 ▰▱▱▱ 9% claude-1`, rail showed the title. Bonus find: `build:tui`'s same-inode `cp` gets fresh execs SIGKILLed by macOS while an old instance runs — script now `rm -f`s first.
