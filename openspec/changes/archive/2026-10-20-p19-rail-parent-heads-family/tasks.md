# Tasks: p19-rail-parent-heads-family

## 1. State (`wall/src/state/salience.rs`)

- [x] 1.1 Head detection: own dir key matches another group's family key; head's family key is its own dir
- [x] 1.2 Order inside a family: blocked first, then head, then insertion order
- [x] 1.3 Family label from the head's basename, else the first member's parent basename; stamped on the first rendered member
- [x] 1.4 Unit tests: real registry with mixed-case paths, blocked child vs head, blocked head, trailing slash, nesting

## 2. Verification

- [x] 2.1 `cd wall && cargo test --lib --bins` and `cargo clippy --all-targets` green; `openspec validate p19-rail-parent-heads-family --strict`
- [x] 2.2 Check the rail with the real registry (user relaunches the wall on the new binary)
- [x] 2.3 Record results in `verification.md`
