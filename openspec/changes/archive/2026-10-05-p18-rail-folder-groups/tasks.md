# Tasks: p18-rail-folder-groups

## 1. State: families (`wall/src/state/salience.rs`)

- [x] 1.1 `family_key` (parent dir, trailing slash stripped, lowercased; `None` for no dir) and `WorkspaceGroup.family_label`
- [x] 1.2 `build_groups`: contiguous families, family-level blocked float, blocked-first inside a family, label on the first member of a family of 2+
- [x] 1.3 Unit tests: real registry order and labels, case-insensitive merge, `dir: None` singleton, family floats, blocked member first, synthesized group joins by session dir

## 2. Rail (`wall/src/ui/`)

- [x] 2.1 `rail.rs`: faint label row above the first member's header
- [x] 2.2 `hit_targets.rs`: `rail_target_at` skips the label row (returns `None`), rows below stay correct
- [x] 2.3 Tests for both; fix fixtures that put every workspace under one parent (`rail.rs`, `store.rs`, `runtime.rs`, `strip.rs`)

## 3. Release prep

- [x] 3.1 `package.json` / `package-lock.json` to 0.6.0, `CHANGELOG.md` entry

## 4. Verification

- [x] 4.1 `cd wall && cargo test --lib --bins` and `cargo clippy --all-targets` green
- [x] 4.2 Check the rail with the real registry shape (user, live wall)
- [x] 4.3 Not run live; covered by unit tests (see verification.md). Planned live check was: click a label row (nothing happens), click headers and sessions below it, press `1`-`9` across a family, block a sibling session and watch its family float (main session only, ask first)
- [x] 4.4 Record results in `verification.md`
