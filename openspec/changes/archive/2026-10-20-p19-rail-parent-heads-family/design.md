## Context

p18 keys each group's family on its dir's parent (`family_key`, trailing slash stripped, lowercased). A workspace registered at the parent folder itself gets the grandparent as its key, so it never joins its children.

## Decisions

**1. Head detection.** A group is a head when its own normalised dir equals some group's `family_key`. Its family key becomes its own dir. Same normalisation as `family_key`, so `/development` and `/Development` still match.

**2. No chaining.** A workspace that is both a head and a child of another head stays the head of its own family. With `/a`, `/a/b`, `/a/b/c`: `b` heads `{b, c}`, and `a` is alone. Alternative: merge the whole subtree. Rejected: one flat family per level is what the label row can name.

**3. Order inside a family: (blocked first, head first, insertion order).** The spec already says blocked workspaces lead a family, so a blocked child goes above an unblocked head. Family order is unchanged: by first member in insertion order.

**4. Label from the family, not from `members[0]`.** With a head, the label is the head's dir basename (original case). Without one, `parent_basename` of the first member, as today. It is stamped on whichever member renders first, for families of 2+. `rail.rs` and `hit_targets.rs` already only read it from the first member.

## Risks / Trade-offs

- Registering a broad folder (for example `~/code`) as a workspace makes it head every workspace under it. Accepted: that matches the folder structure the user chose.
