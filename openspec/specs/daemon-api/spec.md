# daemon-api Specification

## Purpose
TBD - created by archiving change p21-rust-daemon. Update Purpose after archive.

## Requirements

### Requirement: Loopback server and port
The daemon SHALL listen on `127.0.0.1` only, on the port given by `GARAGE_PORT` (default `4747`). All routes SHALL live under `/api`. If the port is taken, the daemon SHALL exit non-zero with a message naming the port and `GARAGE_PORT`.

#### Scenario: Default port
- **WHEN** the daemon starts with no `GARAGE_PORT` set
- **THEN** `GET http://127.0.0.1:4747/api/health` answers 200 and the daemon accepts no connection on any non-loopback address

#### Scenario: Port taken
- **WHEN** port 4747 is already bound and the daemon starts without `GARAGE_PORT`
- **THEN** it exits non-zero and its message names port 4747 and `GARAGE_PORT`

### Requirement: Origin guard on state-changing requests
For `POST`, `PUT`, `PATCH` and `DELETE`, the daemon SHALL reject any request that carries an `Origin` header not listed in `GARAGE_UI_ORIGINS` (comma-separated, empty by default) with `403 {"error":"origin not allowed"}`. Requests with no `Origin` header SHALL be allowed. `GET` SHALL never be rejected by this guard.

#### Scenario: Browser origin rejected
- **WHEN** `POST /api/sessions` arrives with `Origin: https://evil.example` and `GARAGE_UI_ORIGINS` is unset
- **THEN** the daemon answers 403 with body `{"error":"origin not allowed"}` and creates nothing

#### Scenario: CLI request allowed
- **WHEN** `POST /api/sessions` arrives with no `Origin` header
- **THEN** the origin guard lets it through

### Requirement: Error body shape
Every non-2xx response produced by a route SHALL have a JSON body with a string `error` field. Routes MAY add fields (`stderr`, `failedSessions`) as named in their requirements.

#### Scenario: Unknown workspace
- **WHEN** `POST /api/sessions` names a workspace that is not registered
- **THEN** the response is 404 with body `{"error":"unknown workspace: <name>"}`

### Requirement: Names and session ids
Workspace names and session labels SHALL match `^[a-z0-9-]+$`. A session id SHALL be `garage/<workspace>/<label>`. The daemon SHALL only list, create, kill, rename or respawn sessions whose id starts with `garage/`.

#### Scenario: Bad label
- **WHEN** `POST /api/sessions` has `label: "Bad Label"`
- **THEN** the response is 400 with `{"error":"workspace and label must match [a-z0-9-]+"}`

#### Scenario: Non-garage session untouched
- **WHEN** `DELETE /api/sessions/work%2Fmain` is requested
- **THEN** the response is 403 `{"error":"refusing to touch non-garage sessions"}` and no session is killed

### Requirement: Health
`GET /api/health` SHALL return `200 {"status":"ok","version":<string|null>,"pid":<number>}` where `version` is the running build's version and `pid` the daemon's process id. The launcher uses it to detect a stale daemon.

#### Scenario: Health payload
- **WHEN** `GET /api/health` is requested
- **THEN** the body has `status` `"ok"`, the daemon's version string and its pid

### Requirement: Live event stream
`GET /api/events` SHALL answer `200` with `Content-Type: text/event-stream`, write the comment line `: connected` first, then stream events until the client disconnects:
- `event: status` with data `{"id":<session id>,"status":<state>,"since":<epoch ms|null>}` on every status transition;
- `event: sessions` with data `{}` whenever the set of live garage sessions or any pane title changes, and after a successful session restart;
- the comment line `: keepalive` every 15 seconds.
On shutdown the daemon SHALL end all open streams.

#### Scenario: Status transition streamed
- **WHEN** a client is connected to `/api/events` and session `garage/ws/a` goes from `working` to `needs-input`
- **THEN** the client receives `event: status` with data `{"id":"garage/ws/a","status":"needs-input","since":<ms>}`

#### Scenario: New session announced
- **WHEN** a client is connected and a new garage session appears
- **THEN** the client receives `event: sessions` with data `{}` within one poll interval

### Requirement: Session status rules
Session state SHALL be one of `idle`, `working`, `needs-input`, `done`. An unobserved session SHALL read as `idle` with `since: null`. `since` SHALL change only on a real transition. `message` SHALL be non-null only while the state is `needs-input`; it SHALL keep the previous needs-input message when a later needs-input update carries none. A poll reporting the agent idle SHALL NOT downgrade `needs-input` or `done` to `idle`. Agent statuses map `busy` → `working`, `waiting` → `needs-input`, anything else → `idle`.

#### Scenario: Idle poll keeps needs-input
- **WHEN** a session is `needs-input` and the next poll reports the agent idle
- **THEN** the session stays `needs-input` with its message and `since` unchanged

### Requirement: List sessions
`GET /api/sessions` SHALL return an array of live sessions followed by restorable ones. A live entry SHALL be `{id, workspace, label, dir, attached, createdAt, status, message, since, branch, title, context}` where `dir` is the pane's current path (falling back to the session path), `since` falls back to `createdAt`, `branch` is the branch of `dir` (or `@<short sha>` when detached, `null` outside git), `title` is the pane title with tmux defaults (empty, hostname, bare shell names) normalised to `null`, and `context` is `{usedPercentage, source:"statusline"|"transcript"}` or `null`. A restorable entry exists for each resume-metadata record with no live session: `{id, workspace, label, dir, attached:false, status:"restorable", since:null, message:null, title:null, restorable:true, branch, context:null}`, with `dir` resolved from the registered workspace. The route SHALL NOT block on transcript file reads.

#### Scenario: Live and restorable together
- **WHEN** `garage/ws/a` is live and `garage/ws/b` has resume metadata but no tmux session
- **THEN** the array lists `garage/ws/a` with its live fields, then `garage/ws/b` with `status:"restorable"` and `restorable:true`

### Requirement: Create session
`POST /api/sessions` with `{workspace, label, worktree?:boolean}` SHALL start `GARAGE_CLAUDE_CMD` (default `claude`) in a new detached session `garage/<workspace>/<label>` in the registered directory, or in a new git worktree when `worktree` is true. It SHALL answer `201 {id, workspace, label, dir, worktree}` where `worktree` is `{path, branch}` or `null`. Errors: 400 bad names, 404 unknown workspace, 400 registered directory missing, 409 session exists, 400 with git's message when the worktree cannot be created (nothing spawned).

#### Scenario: Worktree session
- **WHEN** `POST /api/sessions` has `{workspace:"ws", label:"fix", worktree:true}` for a git repo
- **THEN** a worktree is created at `<GARAGE_DIR>/worktrees/ws/fix` on branch `garage/fix`, the session starts there, the response is 201 with `worktree:{path, branch}`, and the worktree record is saved in the session's resume metadata

### Requirement: Delete session
`DELETE /api/sessions/<url-encoded id>` SHALL kill a live garage session and remove its resume metadata, answering `200 {deleted:true, worktree}` where `worktree` is `{path, branch, repoDir, target}` (target = branch checked out in `repoDir`) or `null`. With `?meta=1` it SHALL remove only the resume metadata of a non-live session, answering `200 {deleted:true, meta:true, worktree}`; 409 if the session is live, 404 if there is no metadata. Without `meta`, 404 when no such live session.

#### Scenario: Discard restorable entry
- **WHEN** `DELETE /api/sessions/garage%2Fws%2Fb?meta=1` is sent and `garage/ws/b` is not live but has metadata
- **THEN** the metadata is removed and the response is `{deleted:true, meta:true, worktree:null}`

### Requirement: Restore sessions
`POST /api/sessions/restore` with `{id}` or `{all:true}` SHALL recreate sessions from resume metadata, starting `claude --resume <claudeSessionId>` in the worktree path when recorded, else the registered directory. If the resumed process exits within about 3.5 seconds, or no `claudeSessionId` exists, it SHALL start a plain `claude` and report `resumed:false`. Targets SHALL be independent and run concurrently. The response SHALL be `{restored:[{id, workspace, label, dir, worktree, resumed}], failed:[{id, reason}]}` with status 201 (single), 200 (all) when anything was restored, else 409. Failure reasons: `workspace no longer registered`, `worktree missing`, `registered dir no longer exists`, `session already running`. Metadata SHALL be kept on failure. 400 when neither `id` nor `all`; 404 when `id` has no metadata.

#### Scenario: One target fails, others restore
- **WHEN** restore-all runs with two records, one whose workspace was unregistered
- **THEN** the other session is restored, the response is 200, and `failed` lists the first with reason `workspace no longer registered`

### Requirement: Restart sessions in place
`POST /api/sessions/restart` with `{id}` or `{all:true}` and optional `force` SHALL respawn the pane of each targeted live session (same session, new process) with the same resume-then-fallback rule as restore. Sessions in `working`, `needs-input` or an unobserved (`unknown`) state SHALL be skipped unless `force`. If any targeted session has no status yet, the daemon SHALL run one poll before planning. After a restart the session's status and context SHALL be cleared, and a `sessions` event SHALL be sent. Response: `200 {restarted:[{id, resumed}], skipped:[{id, status}], failed:[{id, error}]}`. 400 when neither `id` nor `all`; 404 when `id` is not live.

#### Scenario: Busy session skipped
- **WHEN** restart-all runs without `force` while `garage/ws/a` is `working`
- **THEN** `garage/ws/a` appears in `skipped` with `status:"working"` and keeps running untouched

### Requirement: Workspaces
- `GET /api/workspaces` SHALL return `[{name, dir, branch}]`.
- `PUT /api/workspaces` with `{name, dir}` SHALL register or update a workspace and answer `200 {name, dir}`; 400 for a bad name or a `dir` that is not a directory.
- `PATCH /api/workspaces/<name>` with `{name:<new>}` SHALL rename the workspace, its live sessions and their resume metadata, answering `200 {name, dir, renamedSessions:[{from, to}], failedSessions?:[{id, error}]}`; 404 unknown, 400 bad name, 409 new name taken.
- `DELETE /api/workspaces/<name>` SHALL unregister it and answer 204, leaving sessions running; with `?sessions=kill` it SHALL also kill its live sessions and answer `200 {removed, killedSessions:[id], failedSessions?:[{id, error}]}`; 404 unknown.

#### Scenario: Rename carries sessions
- **WHEN** `PATCH /api/workspaces/old` has `{name:"new"}` and `garage/old/a` is live
- **THEN** the tmux session becomes `garage/new/a`, its resume metadata moves too, and `renamedSessions` is `[{from:"garage/old/a", to:"garage/new/a"}]`

### Requirement: Claude Code hook intake
`POST /api/hooks/claude?token=<hook token>` SHALL accept Claude Code hook payloads. A missing or wrong token SHALL get 401. `Notification` SHALL set `needs-input` with the payload's `message`, except messages matching "waiting for your input" (answered `{ok:true, ignored:"idle-reminder"}`). `Stop` SHALL set `done`. Other events SHALL answer `{ok:true, ignored:true}`. Sessions SHALL be resolved from `session_id` (via the agent's pid) or else from `cwd`. Success answers `{ok:true, applied:[ids]}`. The token SHALL be compared in constant time.

#### Scenario: Permission prompt
- **WHEN** a valid `Notification` hook arrives with `message:"Claude needs your permission to use Bash"` for a session in `garage/ws/a`
- **THEN** `garage/ws/a` becomes `needs-input` with that message and a `status` event is streamed

### Requirement: Hook and statusline install
- `GET /api/hooks/snippet` SHALL return the `settings.json` fragment (`hooks.Notification`, `hooks.Stop` with the tokenised URL, `allowedHttpHookUrls`).
- `POST /api/hooks/install` SHALL merge it into `<claude home>/settings.json` without duplicating existing groups, writing atomically with a timestamped backup, and answer `{ok:true, installed:true, alreadyInstalled, backup}`.
- `GET /api/statusline/snippet` SHALL return `{statusLine:{type:"command", command}, script}`.
- `POST /api/statusline/install` SHALL write the wrapper script (posting to `/api/statusline/claude`, chaining any existing statusline command) and set `statusLine`, answering `{ok:true, installed:true, alreadyInstalled, chained, backup}`.
- Invalid or non-object `settings.json` SHALL get 422 and the file SHALL be left untouched.

#### Scenario: Install twice
- **WHEN** `POST /api/hooks/install` runs twice
- **THEN** the second call answers `alreadyInstalled:true`, `backup:null`, and `settings.json` is unchanged

### Requirement: Statusline intake and usage
`POST /api/statusline/claude?token=` SHALL take Claude Code's statusline JSON: store `context_window.used_percentage` (clamped 0–100) per resolved session as `source:"statusline"` context, record the model's context window size, and update rate limits from `rate_limits.five_hour` / `seven_day`. Unresolved sessions answer `{ok:true, ignored:true}`. `GET /api/usage` SHALL return `{fiveHour, sevenDay}`, each `{usedPercentage, resetsAt}` or `null`.

#### Scenario: Usage after a post
- **WHEN** a statusline post carries `rate_limits.five_hour.used_percentage: 42`
- **THEN** `GET /api/usage` returns `fiveHour.usedPercentage` 42

### Requirement: UI visibility and notifications
`POST /api/ui/visibility` with `{clientId:string, visible:boolean}` SHALL record the client as seen now and answer `{ok:true}`; 400 otherwise. On a transition to `needs-input`, the daemon SHALL send a desktop notification only when no client reported `visible:true` within the last 45 seconds.

#### Scenario: Wall in view
- **WHEN** a client posted `visible:true` 10 seconds ago and a session becomes `needs-input`
- **THEN** no desktop notification is sent

### Requirement: Daemon self-restart
`POST /api/daemon/restart` SHALL start a successor daemon detached (logging to `<GARAGE_DIR>/daemon.log`) with `GARAGE_PREDECESSOR_PID` set, answer `202 {pid}`, then shut down. The successor SHALL wait up to 10 seconds for the predecessor's port to free before binding, and exit non-zero naming the port and pid if it does not. tmux sessions SHALL be unaffected.

#### Scenario: Handoff
- **WHEN** `POST /api/daemon/restart` is called
- **THEN** within 15 seconds `GET /api/health` reports a new pid, and every garage session listed before is still live

### Requirement: Open editor and pick directory
- `POST /api/open-editor` with `{workspace, file?, line?}` SHALL run `GARAGE_EDITOR_CMD` (default `code`) on the workspace directory, or with `--goto <abs file>:<line|1>`; a `file` resolving outside the workspace gets 400; an unknown workspace 404; a missing editor CLI 501; success `{ok:true}`.
- `POST /api/pick-directory` SHALL show a native folder picker and answer `{dir}` (trailing slash stripped) or `{cancelled:true}`; 501 where the OS has no picker.

#### Scenario: Path escape refused
- **WHEN** `POST /api/open-editor` has `file:"../../etc/passwd"`
- **THEN** the response is 400 `{"error":"file escapes workspace directory"}` and no editor starts

### Requirement: Finish a worktree
`POST /api/worktrees/finish` with `{worktree:{path, branch, repoDir}, action:"merge"|"discard"|"keep"}` SHALL: for `keep`, change nothing; for `merge`, refuse a dirty worktree (409), merge `--no-ff` into the branch checked out in `repoDir`, abort and answer 409 on conflict, then remove the worktree and delete the branch; for `discard`, force-remove the worktree and branch. Only branches starting `garage/` SHALL be touched (400 otherwise). Success: `{action, path, branch, merged?|discarded?}`. Errors MAY include git's `stderr`.

#### Scenario: Conflict leaves repo clean
- **WHEN** a merge conflicts
- **THEN** the merge is aborted, the response is 409, and `repoDir` has no merge in progress

### Requirement: Diff route not served
The daemon SHALL NOT serve `GET /api/diff/:workspace`; it SHALL answer 404 like any unknown route.

#### Scenario: Diff request
- **WHEN** `GET /api/diff/ws` is requested
- **THEN** the response is 404
