## ADDED Requirements

### Requirement: Overview endpoint
`GET /api/overview` SHALL return `200` with `{generatedAt, github:{state, until}, workspaces:[...]}` where each workspace has `name`, `warnings`, `fetch`, `waiting` and `changes` as described by the `workspace-overview` capability; `?workspace=<name>` SHALL restrict it to one workspace, answering `404 {"error":"unknown workspace: <name>"}` for an unknown name. The route SHALL answer from cache without waiting on `git` or `gh`, SHALL count as a consumer for 60 seconds, and SHALL never fail because GitHub is unreachable. Fields MAY be added; existing fields SHALL NOT change meaning.

#### Scenario: One workspace
- **WHEN** `GET /api/overview?workspace=kowboy` is requested and `kowboy` is registered
- **THEN** the response lists only `kowboy`

#### Scenario: GitHub down
- **WHEN** `gh` cannot reach github.com
- **THEN** the response is 200, `github.state` is `offline`, and local changes are present

#### Scenario: Unknown workspace
- **WHEN** `GET /api/overview?workspace=ghost` is requested
- **THEN** the response is 404 with `{"error":"unknown workspace: ghost"}`

### Requirement: Overview refresh and event
`POST /api/overview/refresh` with an optional `{workspace}` SHALL start a full refresh (local, fetch, GitHub) and answer `202 {ok:true}`; 404 for an unknown workspace. When any refresh completes with a changed model, the daemon SHALL emit `event: overview` with data `{}` on `/api/events`. The route is subject to the existing origin guard.

#### Scenario: Event after refresh
- **WHEN** a client is connected to `/api/events` and a refresh finds a ticked checkbox
- **THEN** the client receives `event: overview` with data `{}`

#### Scenario: Browser origin rejected
- **WHEN** `POST /api/overview/refresh` arrives with an `Origin` not in `GARAGE_UI_ORIGINS`
- **THEN** the daemon answers 403 `{"error":"origin not allowed"}` and starts no refresh
