# Verification: p18-rail-folder-groups

## 2026-10-05

**Unit:** `cd wall && cargo test --lib --bins` 514 pass. `cargo clippy --all-targets` clean. `openspec validate p18-rail-folder-groups --strict` valid.

**Build:** `npm run build:tui` (release) succeeded; run by the user.

**Live:** the user restarted their wall on the new binary with the real registry (6 workspaces) and confirmed the rail: cto-playground, then `elite-traders` (et-mobile-app, et-backend, et-admin), then `personal` (mobile-apps, diary-app). The mixed `development`/`Development` paths merged into one family.

**Not run live:** label-row clicks, `1`-`9` across a family, and a blocked sibling floating its family. These are covered by unit tests only:
- `hit_targets.rs` `a_family_label_row_maps_to_none_and_shifts_rows_below_it`
- `salience.rs` `a_family_floats_when_one_member_is_blocked`, `real_registry_groups_siblings_and_labels_first_members`, `family_merge_is_case_insensitive_label_keeps_first_members_case`, `dir_none_is_its_own_singleton_family`, `synthesized_groups_join_a_family_by_their_session_dir`
- `rail.rs` `family_label_row_sits_above_the_first_member_only`
