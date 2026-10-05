## Context

`build_groups` returns a flat `Vec<WorkspaceGroup>` in registry order (then synthesized groups), with one final stable partition that floats blocked workspaces. Digits `1`-`9`, the strip tabs and the store all index that flat list. The rail draws one line per row and `rail_target_at` maps click rows against the same layout.

## Goals / Non-Goals

**Goals:** sibling workspaces adjacent, a quiet label naming the shared folder, no change to numbering or the strip.

**Non-Goals:** collapsing families, a tree in the state, labels in the strip, user-defined groups (the `views` feature is separate).

## Decisions

**1. Family key = parent directory of the group's `dir`.** Trailing slash stripped, lowercased, because macOS paths in one registry mix `development` and `Development`. A group with no `dir` (or no parent) is its own singleton family. Synthesized groups use their first session's dir, so they join a family too.

**2. State stays flat.** `build_groups` reorders the flat list so members are contiguous. Family order is the order of each family's first member in insertion order. Alternative: nested `Vec<Family>`. Rejected: every consumer of `state.groups` would change for a cosmetic feature.

**3. Salience at family level.** Stable partition on "any member blocked" over families, then a stable partition inside each family. Session ordering inside a workspace is unchanged.

**4. `family_label: Option<String>` on `WorkspaceGroup`.** Set on the first member (after salience ordering) of a family with two or more members, to the parent folder's basename in that member's original case. The rail draws it as one faint row above the header, with the same one-space indent as headers. Members' headers stay as they are; the rail is not widened.

**5. Click mapping in lockstep.** `rail_lines` and `rail_target_at` both add one row before a header whose group has a label. The label row maps to `None`. No other code counts rail rows (the runtime click path calls `rail_target_at`; the rail has no scroll offset).

## Risks / Trade-offs

- Two unrelated repos that happen to share a parent (for example `~/code`) get a label. Accepted: that is the same signal the user already organized by.
- Test fixtures gave every workspace `/repos/{name}`, which would now be one family. Fixtures use `/repos/{name}/{name}` so existing tests keep their meaning.
