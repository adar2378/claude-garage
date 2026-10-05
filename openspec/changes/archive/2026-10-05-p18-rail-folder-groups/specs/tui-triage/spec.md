## MODIFIED Requirements

### Requirement: Salience-first ordering
Rail ordering SHALL be stable and salience-sorted over workspace families. A family is the set of workspaces whose directories share a parent folder (compared case-insensitively, trailing slash ignored); a workspace with no known directory is its own family. Family members SHALL be adjacent in the rail, families ordered by their first member's registration (then discovery) order and members keeping that order within the family. A family containing a `needs-input` session SHALL sort before families without one, and within a family workspaces containing a `needs-input` session SHALL sort first. Within a workspace `needs-input` sessions sort first. The strip SHALL show a per-workspace amber dot on each workspace tab that contains a blocked session, and the total blocked count SHALL be visible whenever it is non-zero.

#### Scenario: Blocked family bubbles up
- **WHEN** workspace B (rail position 2), a sibling of workspace C (position 3), gains a needs-input session while workspace A (position 1, a different folder) has none
- **THEN** B renders first, C directly after it, then A, and other relative orders are unchanged

#### Scenario: Siblings are adjacent
- **WHEN** the registry lists `et-mobile-app`, `mobile-apps`, `et-backend` where `et-mobile-app` and `et-backend` share a parent folder
- **THEN** `et-mobile-app` and `et-backend` are adjacent in the rail, with `mobile-apps` after them

#### Scenario: Folder case does not split a family
- **WHEN** two workspaces live under `.../development/x` and `.../Development/x`
- **THEN** they belong to one family
