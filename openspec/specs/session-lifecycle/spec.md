# session-lifecycle

## Purpose

Spawning, listing, and killing garage-owned tmux sessions. tmux is the sole source of truth; the `garage/<workspace>/<label>` session name is the registry.

## Requirements

### Requirement: Daemon health endpoint
The daemon SHALL expose `GET /api/health` returning HTTP 200 with a JSON body, and SHALL listen on `127.0.0.1` only.

#### Scenario: Health check succeeds locally
- **WHEN** a client requests `GET http://127.0.0.1:<port>/api/health`
- **THEN** the daemon responds 200 with JSON `{"status":"ok"}`

#### Scenario: Daemon is not reachable from other interfaces
- **WHEN** the daemon's listen address is inspected (e.g. `lsof -i -P` for the daemon pid)
- **THEN** the socket is bound to `127.0.0.1`, not `0.0.0.0` or a LAN address

### Requirement: Foreign browser origins rejected
State-changing requests (POST/PUT/PATCH/DELETE) carrying an `Origin` header outside the UI allowlist SHALL be rejected with 403. Requests without an `Origin` header (curl, local scripts) SHALL be allowed — they are ordinary local processes, not confused-deputy browsers.

#### Scenario: Cross-site request forgery blocked
- **WHEN** `POST /api/sessions` arrives with `Origin: http://evil.example`
- **THEN** the daemon responds 403 and no tmux session is created

#### Scenario: Header-less local tooling still works
- **WHEN** `POST /api/sessions` arrives from curl with no `Origin` header
- **THEN** the request is processed normally

### Requirement: Spawn a garage session
The daemon SHALL create sessions via `POST /api/sessions` with body `{workspace, label, dir}`, by running `tmux new-session -d -s "garage/<workspace>/<label>" -c <dir> claude`. `workspace` and `label` MUST match `[a-z0-9-]+`; invalid names SHALL be rejected with 400. A name collision with an existing tmux session SHALL be rejected with 409.

#### Scenario: Successful spawn
- **WHEN** `POST /api/sessions` is called with `{workspace:"garage-dev", label:"main", dir:"<existing dir>"}`
- **THEN** the daemon responds 201 with the session id `garage/garage-dev/main`
- **THEN** `tmux ls` on the host shows a detached session named `garage/garage-dev/main` running in `<existing dir>`

#### Scenario: Duplicate name rejected
- **WHEN** `POST /api/sessions` is called twice with the same workspace and label
- **THEN** the second call responds 409 and no second tmux session is created

#### Scenario: Invalid name rejected
- **WHEN** `POST /api/sessions` is called with a label containing `:`, `.`, `/`, uppercase, or spaces
- **THEN** the daemon responds 400 and no tmux session is created

### Requirement: List garage sessions from tmux
The daemon SHALL expose `GET /api/sessions` whose result is derived solely from parsing `tmux ls` output filtered to names with the `garage/` prefix. Non-garage tmux sessions SHALL never appear. When the tmux server is not running, the daemon SHALL return an empty list, not an error.

#### Scenario: Only garage sessions listed
- **WHEN** tmux hosts sessions `garage/kowboy/checkout` and `personal-scratch`
- **THEN** `GET /api/sessions` returns exactly one entry, with id `garage/kowboy/checkout`, workspace `kowboy`, label `checkout`

#### Scenario: Externally killed session disappears
- **WHEN** a garage session is killed outside the daemon (`tmux kill-session` in iTerm) and `GET /api/sessions` is called again
- **THEN** the killed session is absent from the response with no daemon restart required

#### Scenario: No tmux server
- **WHEN** no tmux server is running and `GET /api/sessions` is called
- **THEN** the daemon responds 200 with `[]`

### Requirement: Kill a garage session
The daemon SHALL expose `DELETE /api/sessions/:id` which runs `tmux kill-session` for that session. Ids outside the `garage/` prefix SHALL be rejected with 403 so the daemon can never kill a user's personal tmux sessions.

#### Scenario: Kill removes the session
- **WHEN** `DELETE /api/sessions/garage%2Fkowboy%2Fcheckout` is called for a live session
- **THEN** the daemon responds 204 and `tmux ls` no longer shows the session

#### Scenario: Non-garage session protected
- **WHEN** `DELETE /api/sessions/personal-scratch` is called
- **THEN** the daemon responds 403 and the tmux session is untouched
