## ADDED Requirements

### Requirement: Resume metadata persisted opportunistically
The daemon SHALL persist, for each garage session, its Claude Code `sessionId` (as observed via the status poller/hooks), workspace, and label to `~/.garage/state.json` under a `sessions` map keyed by the garage session id. This metadata SHALL be updated opportunistically whenever a new `sessionId` is observed for that session — recording it SHALL NOT block or fail the session's normal operation.

#### Scenario: sessionId observed and persisted
- **WHEN** the poller observes a Claude Code `sessionId` for a live garage session `garage/kowboy/checkout`
- **THEN** `~/.garage/state.json`'s `sessions` map gains or updates an entry for `garage/kowboy/checkout` containing that `sessionId`, its workspace, and its label

#### Scenario: Metadata persists across daemon restart
- **WHEN** the daemon is restarted after resume metadata for a session has been recorded
- **THEN** `~/.garage/state.json` still contains that session's resume metadata after restart, read from disk

### Requirement: Resume metadata lifecycle on session removal
Resume metadata for a session SHALL be deleted from `~/.garage/state.json` when that session is killed via `DELETE /api/sessions/:id`. Resume metadata SHALL be retained in `~/.garage/state.json` when the underlying tmux session disappears without going through the DELETE endpoint (e.g. the Mac reboots, tmux itself is killed, or the session is killed directly with `tmux kill-session` outside the daemon).

#### Scenario: Deliberate kill deletes metadata
- **WHEN** a client calls `DELETE /api/sessions/:id` for a session with resume metadata recorded
- **THEN** the daemon removes that session's entry from `state.json`'s `sessions` map

#### Scenario: Vanished session retains metadata
- **WHEN** a garage session's tmux process disappears without a DELETE call (e.g. after a reboot)
- **THEN** that session's resume metadata remains in `state.json` unchanged

### Requirement: Restorable sessions reported via GET /api/sessions
`GET /api/sessions` SHALL include, alongside live tmux-derived sessions, any session present in `state.json`'s resume metadata but absent from `tmux ls` output. Such entries SHALL be marked with status `restorable` and SHALL include the workspace and label recorded in their metadata.

#### Scenario: Vanished session appears as restorable
- **WHEN** `state.json` records resume metadata for `garage/kowboy/checkout` but `tmux ls` shows no such session (e.g. after a reboot)
- **THEN** `GET /api/sessions` includes `garage/kowboy/checkout` with status `restorable`

#### Scenario: Live session is not marked restorable
- **WHEN** a session is both recorded in `state.json` and present in `tmux ls`
- **THEN** `GET /api/sessions` reports that session with its live status (`needs-input`/`working`/`done`/`idle`), not `restorable`

### Requirement: Restore a session
The daemon SHALL expose `POST /api/sessions/restore` accepting `{id}` identifying a restorable session, recreating it by running `tmux new-session … claude --resume <sessionId>` in the directory resolved from the session's registered workspace. The restored session SHALL contain the prior conversation — conversation continuity is preserved for the resumed Claude Code process.

#### Scenario: Restoring a session recreates it with conversation continuity
- **WHEN** `POST /api/sessions/restore` is called with `{id:"garage/kowboy/checkout"}` for a restorable session whose metadata records `sessionId` `abc123` and workspace `kowboy` (dir exists)
- **THEN** the daemon runs `tmux new-session -d -s "garage/kowboy/checkout" -c <kowboy's dir> claude --resume abc123`
- **THEN** `GET /api/sessions` subsequently reports `garage/kowboy/checkout` as live (no longer `restorable`), and the terminal shows the prior conversation history rather than a blank session

### Requirement: Restore all restorable sessions
`POST /api/sessions/restore` SHALL accept `{all:true}` to restore every currently restorable session in one call. A failure restoring one session SHALL NOT prevent the others from being restored.

#### Scenario: Restore all after reboot
- **WHEN** three sessions are restorable after a reboot and `POST /api/sessions/restore` is called with `{all:true}`
- **THEN** the daemon restores all three, recreating each via `claude --resume` with its recorded `sessionId`, and `GET /api/sessions` afterward reports all three as live

#### Scenario: Restore all continues past individual failures
- **WHEN** `{all:true}` is called and one of the restorable sessions has a missing workspace while the others restore normally
- **THEN** the daemon restores the sessions that succeed, and the failed one remains marked `restorable` with its metadata retained, without aborting the rest

### Requirement: Restore failure when workspace or directory missing
If the session's recorded workspace no longer exists in the registry, or the registered directory no longer exists on disk, `POST /api/sessions/restore` SHALL respond 409 for that session and SHALL NOT create a tmux session. The session's resume metadata SHALL be retained in `state.json` (not deleted) so a later restore attempt can still succeed once the workspace/dir is fixed.

#### Scenario: Missing workspace
- **WHEN** `POST /api/sessions/restore` is called with `{id:"garage/kowboy/checkout"}` whose recorded workspace `kowboy` no longer exists in the registry
- **THEN** the daemon responds 409 and does not create a tmux session, and `garage/kowboy/checkout`'s resume metadata remains in `state.json`

#### Scenario: Missing directory
- **WHEN** workspace `kowboy` is still registered but its directory has been deleted from disk, and restore is attempted for a session under `kowboy`
- **THEN** the daemon responds 409 and does not create a tmux session, and the resume metadata is retained

### Requirement: Restore blocked by name collision
If a live tmux session already exists with the same garage session name as the one being restored, `POST /api/sessions/restore` SHALL respond 409 and SHALL NOT create a second tmux session, nor delete the resume metadata.

#### Scenario: Restoring into a name already in use
- **WHEN** a live tmux session named `garage/kowboy/checkout` already exists (e.g. spawned fresh under the same label after reboot) and restore is called for a restorable entry also named `garage/kowboy/checkout`
- **THEN** the daemon responds 409, no new tmux session is created, and the existing live session is untouched
