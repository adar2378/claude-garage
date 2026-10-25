# Verification: p19-rail-parent-heads-family

## 2026-10-20

**Unit:** `cd wall && cargo test --lib --bins` 519 pass (514 before, 5 new). `cargo clippy --all-targets` clean. `openspec validate p19-rail-parent-heads-family --strict` valid.

**Build:** `cd wall && cargo build --release` succeeded in the p19 worktree.

New tests in `wall/src/state/salience.rs`:
- `a_head_joins_its_children_family_and_leads_it` (real registry: `elite-traders` at `/development/...`, `et-*` at `/Development/...`)
- `a_blocked_child_leads_an_unblocked_head_and_takes_the_label`
- `a_blocked_head_leads_among_blocked_members`
- `a_head_with_a_trailing_slash_still_merges`
- `nesting_does_not_chain_a_middle_head_owns_its_family`

**Live:** 2026-10-26, the user relaunched the wall on v0.6.2 (daemon restarted by the version gate, tmux sessions intact) and confirmed the rail: `elite-traders` heads its `et-*` family under the `elite-traders` label.
