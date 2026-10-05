## Why

The rail lists workspaces in registry order, and any workspace with a blocked session floats to the top. Sibling repos (for example `elite-traders/et-admin`, `et-backend`, `et-mobile-app`) end up scattered with unrelated workspaces between them, so related work is hard to scan.

## What Changes

- Workspaces whose directories share a parent folder form a family. Family members sit next to each other in the rail.
- A family label row (the parent folder's name, faint) appears above the first member of every family with two or more workspaces. Singletons get no label.
- Salience works on families: a family floats to the top if any member is blocked, and blocked members come first inside it.
- Workspace numbers (`1`-`9`) still follow the flat order; label rows take no number and are not clickable.
- Released as 0.6.0.

## Capabilities

### Modified Capabilities
- `tui-triage`: "Salience-first ordering" works on families.
- `tui-wall`: "Rail, strip, and salience ladder" gains the family label row.

## Impact

- **Code:** `wall/src/state/salience.rs` (`build_groups`, `WorkspaceGroup.family_label`), `wall/src/ui/rail.rs`, `wall/src/ui/hit_targets.rs`; test fixtures in `runtime.rs`, `store.rs`, `strip.rs`.
- **Release:** `package.json`, `package-lock.json`, `CHANGELOG.md`.
- No daemon change, no API change.
