# workspace-registry

## Purpose

Mapping workspace names to project directories, persisted in `~/.garage/state.json`. Spawn-time directory resolution and P3 restore metadata only — never a session store.

## Requirements

### Requirement: Register a workspace
The daemon SHALL expose an endpoint to register a workspace mapping a workspace `name` to a project directory `dir`. `name` MUST match `[a-z0-9-]+`; names failing this pattern SHALL be rejected with 400. `dir` MUST exist on disk at registration time; if it does not exist, the daemon SHALL reject the request with 400 and SHALL NOT write the registry. Registering a `name` that already exists SHALL upsert (overwrite) that workspace's `dir` rather than error.

#### Scenario: Successful registration
- **WHEN** a client registers workspace `{name:"kowboy", dir:"/Users/dev/kowboy"}` and `/Users/dev/kowboy` exists
- **THEN** the daemon responds 201 (or 200) with the stored workspace record `{name:"kowboy", dir:"/Users/dev/kowboy"}`

#### Scenario: Non-existent directory rejected
- **WHEN** a client registers workspace `{name:"ghost", dir:"/Users/dev/does-not-exist"}`
- **THEN** the daemon responds 400 and no workspace named `ghost` is added to the registry

#### Scenario: Invalid name rejected
- **WHEN** a client registers a workspace with `name:"Ko Wboy"` (uppercase and a space)
- **THEN** the daemon responds 400 and no workspace is added to the registry

#### Scenario: Re-registering an existing name upserts the directory
- **WHEN** workspace `kowboy` is already registered at `/Users/dev/kowboy`, and a client registers `{name:"kowboy", dir:"/Users/dev/kowboy-v2"}` where `/Users/dev/kowboy-v2` exists
- **THEN** the daemon responds with success and subsequent lookups of `kowboy` resolve to `/Users/dev/kowboy-v2`, not the original directory

### Requirement: List registered workspaces
The daemon SHALL expose an endpoint that lists all registered workspaces with their `name` and `dir`.

#### Scenario: List reflects registered workspaces
- **WHEN** workspaces `kowboy` and `garage-dev` have been registered
- **THEN** the list endpoint returns both entries, each with its registered `name` and `dir`

#### Scenario: Empty registry returns empty list
- **WHEN** no workspace has ever been registered
- **THEN** the list endpoint responds 200 with an empty list, not an error

### Requirement: Registry persists across daemon restarts
The daemon SHALL persist the workspace registry to `~/.garage/state.json` such that registered workspaces survive a daemon process restart. Writes to `~/.garage/state.json` SHALL be atomic (e.g. write-to-temp-file-then-rename) so a crash or concurrent write cannot leave the file truncated or corrupted.

#### Scenario: Registry survives daemon restart
- **WHEN** workspace `kowboy` is registered, the daemon process is killed and restarted
- **THEN** listing workspaces after restart still includes `kowboy` with its original `dir`, read from `~/.garage/state.json`

#### Scenario: Atomic write leaves no partial state
- **WHEN** the daemon writes an update to `~/.garage/state.json` and the process is killed mid-write
- **THEN** `~/.garage/state.json` on disk is either the previous complete valid JSON or the new complete valid JSON — never a truncated or malformed file

### Requirement: Registry never manages tmux sessions
The workspace registry SHALL only store the name-to-directory mapping. Registering, upserting, or listing workspaces SHALL NOT create, kill, or otherwise mutate any tmux session. tmux session lifecycle remains governed exclusively by the `session-lifecycle` capability.

#### Scenario: Registering a workspace creates no tmux session
- **WHEN** a client registers workspace `{name:"solo", dir:"<existing dir>"}` and no session has been spawned for it
- **THEN** `tmux ls` shows no session named `garage/solo/*` as a result of the registration call alone
