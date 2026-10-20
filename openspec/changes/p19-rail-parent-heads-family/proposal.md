## Why

A workspace registered at a folder that also holds other registered workspaces lands in the wrong family. `elite-traders` (`.../devmonks/elite-traders`) groups with the `devmonks` workspaces, while its own repos (`et-backend`, `et-mobile-app`, `et-admin`, under `.../elite-traders/`) form an `elite-traders` family without it.

## What Changes

- A workspace whose directory is the parent folder of other workspaces' directories is the head of their family. It joins that family instead of its own parent's.
- Inside a family, blocked workspaces still come first. Among the rest, the head comes first, then insertion order.
- The family label is the head's folder name when the family has a head, otherwise the shared parent folder's name as today.
- Workspaces with no children behave as before.

## Capabilities

### Modified Capabilities
- `tui-triage`: "Salience-first ordering" defines the family head.
- `tui-wall`: "Rail, strip, and salience ladder" names the label after the family folder.

## Impact

- **Code:** `wall/src/state/salience.rs` (`arrange_families` and its tests).
- No rail, hit-target, daemon or API change.
