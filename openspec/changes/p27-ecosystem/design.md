# Design: p27-ecosystem

**Detailed design is written when the phase starts.** It depends on the manifest and API shaped by p24 and frozen in p25. Decisions already made:

## Decisions

**1. API v1 is frozen only after p25.** First-party plugins must have used every endpoint, event and slot before the version promise is made. Freezing earlier would freeze guesses.

**2. One schema, many clients.** The protocol crate is the source of truth. The API schema, the Rust SDK and the TypeScript SDK are generated from it, so they cannot drift.

**3. Versioning by API version and capability, not garage version.** A manifest names `api = "1"` and may list optional capabilities it uses. garage refuses to load a plugin that needs a version or capability it lacks, with a message naming what is missing.

**4. Discovery through a GitHub topic.** Publishing is: public repo, `garage-plugin.toml`, topic `claude-garage-plugin`. `plugin search` uses the GitHub search API. No submission or review queue; the install preview (p24) shows commands and permissions before anything runs.

**5. Claude Code is a first-class authoring path.** The scaffold ships a `CLAUDE.md` describing the manifest, slots, actions, permissions and the dev loop, and a skill turns one sentence into a plugin skeleton the user can run with `plugin dev`.

## Open questions (resolve at phase start)

- Docs location: a `docs/` site in this repo, or a separate site?
- Should `plugin search` rank by stars, recent activity, or both?
- Deprecation window length for a superseded API version.

---

# Part A: API freeze, docs and SDKs (detailed design)

This part covers p25 task 4.2 (freeze v1) and p27 tasks 1.1, 1.3, 2.1, 2.2 (the first four are done by hand-written, test-checked code rather than code generation; see decision A3) and 4.2 (docs). `plugin new`, `plugin dev`, `plugin search`, the skill and the example plugin are other parts.

## A1. What "v1" means

`api_version = 1` in a manifest (and `api_version: 1` in the run context, `GARAGE_API_VERSION=1`) names one frozen contract. The contract is exactly these surfaces:

| Surface | Source of truth | Frozen as |
|---|---|---|
| Manifest keys (tables and fields) | `crates/plugin/src/manifest.rs`, `schemas/garage-plugin.schema.json` | names, types, meaning |
| Permissions | `features::PERMISSIONS` | names, meaning, which are dangerous |
| Events and their payload keys | `features::EVENTS`, `DaemonEvent::plugin_event` | names, payload keys |
| Slots, widget wire format, theme tokens, limits | `garage_protocol::widgets` | names, field names, scope rules |
| HTTP routes a plugin may call | `features::ROUTES_V1` | method, path, permission |
| Run context keys, environment variables | `daemon/src/plugins/context.rs`, `mod.rs` | names |
| Core action ids a plugin may invoke | `core_actions` | ids |
| CLI contract (`[[cli]]`, fallback, hand-over file) | manifest + SDK | behaviour |

Not frozen: anything not listed (the TUI's look, log format, `plugins.json`, core routes that are not in `ROUTES_V1`).

### The freeze rule

Within v1 garage may only add: a manifest key, an optional request field, a response field, an event, a payload key, a slot, a permission, a theme token, a core action id, a route, a feature flag. It must not remove, rename or change the meaning of anything in the table, and must not make an optional thing required. Limits may be raised, never lowered. A new permission never gates something that was free before.

A plugin must ignore unknown keys in context, event payloads and responses. The docs say so.

### Breaking change

A change that breaks the rule ships as `api_version = 2`. garage then loads v1 and v2 together (`API_VERSIONS = [1, 2]`) for at least two minor garage releases and at least six months after v2 ships (the open question on the deprecation window: this is the answer). During that time `plugin list` marks v1 plugins "deprecated API" and the install preview says so. After it, garage may stop loading v1 and says "needs plugin API v1; this garage supports 2".

### Where the freeze is enforced

`crates/plugin/api-v1.golden.json` is a hand-edited copy of the v1 surface (not generated; that is the point). Tests:

1. `garage-plugin`: every golden event, slot, permission, dangerous permission, pane placement, theme token, core action id, feature key, route and manifest field must still exist in the code. Extra items in code are fine (additive). Failure message: "v1 removed or renamed X; this needs api_version 2".
2. `garage-plugin`: golden manifest fields are checked against the committed JSON schema, so a renamed field fails even if the Rust type was edited together with the schema.
3. `garage-daemon`: every golden route answers (not 404 `not found`, not 405) on the real router; the real run context contains every golden top-level and nested key; every golden event payload key is present in `plugin_event`.
4. `garage-protocol`: the API schema has every golden wire type and property.

Updating the golden file is a deliberate act in a PR that says "additive". Removing a line from it is the thing review must refuse.

`/api/plugin/features` reflects the same set: it gains `permissions`, `dangerous_permissions`, `theme_tokens`, `routes` and `api.v1.frozen = true`. `requires = ["api.v1.frozen"]` is legal but pointless; it exists so tools can detect a freeze-aware garage.

## A2. Docs (`docs/plugins/`)

Plain, short sentences (STE-lite). One page per topic, each starts with what it is for and ends with a runnable or copyable example.

`README.md` (overview), `quick-start.md`, `manifest.md`, `permissions.md`, `events.md`, `widgets.md` (slots and widget format), `actions.md` (and the confirm rule), `panes.md`, `cli.md`, `storage.md`, `security.md`, `versioning.md`, `api.md` (HTTP reference, environment, context).

`manifest.md` has a field table generated from `schemas/garage-plugin.schema.json` between `<!-- generated:manifest -->` markers. A `garage-plugin` unit test regenerates the block and fails when the committed page differs (`UPDATE_DOCS=1` rewrites it, like the schema). This keeps the one page that drifts fastest honest; the prose pages are hand-written, and a second test checks that every event, slot, permission and route in the golden list is mentioned by name in the docs.

Docs must not name other products, and they say plainly: no sandbox.

## A3. SDKs

Decision: the Rust SDK stays hand-written on top of `garage-protocol` (its types are the source of truth, so they cannot drift), and the TypeScript SDK is hand-written but its types are generated from a JSON schema derived from the same Rust types. The original text said "generated SDK"; code generation of a client for a dozen routes costs more than it saves, while the types are where drift happens.

### API schema

`garage-protocol` gets `schema::api_schema_json()`: a draft-07 schema with `definitions` for the wire types: `WidgetPut`, `WidgetItem`, `Segment`, `Style`, `Click`, `Placement`, `WidgetSlot`, `PluginList`, `WidgetsSnapshot`, `PanesResponse`, `UiActionFrame`, and request/response types that only existed as `json!` (`UiActionRequest`, `NoticeRequest`, `SessionInputRequest`, `OkResponse`, `PutWidgetResponse`, `OpenPaneResponse`, `RunContext`, `FeaturesResponse`, event payloads). Committed as `schemas/garage-plugin-api.schema.json`; a test fails when stale (`UPDATE_SCHEMA=1`), same as the manifest schema. The request/response structs are the real types where the daemon already had them, and new structs the daemon starts using where it did not (so they cannot differ from the code).

### Rust SDK (`crates/plugin-sdk`, package `garage-plugin-sdk`)

Goal: publishable on crates.io, API matching the docs.

- `Cargo.toml`: `license = "MIT"`, `repository`, `homepage`, `documentation`, `readme`, `keywords`, `categories`, `include`, `rust-version`. `garage-protocol` becomes a versioned path dependency (`version = "=0.6.2"`) so `cargo publish` can resolve it; `garage-protocol` also gets publishable metadata, since it is a dependency (publishing order: protocol, then sdk). The workspace version stays in step with `package.json`.
- Crate docs (`//!`) from `README.md` with `#![doc = include_str!("../README.md")]`, so the README examples are doc-tests.
- Examples: `examples/widget.rs` (push a widget), `examples/action.rs` (read context, notice), `examples/pane.rs` (feature `tui`), compiled in CI by `cargo test --examples`.
- API additions to match the docs: `Api::features()`, `Api::open_pane/close_pane`, `Api::sessions()`, `Api::workspaces()`, `Context::trigger_*`, typed `Context` accessors for focus, selection, trigger and worktree. Existing signatures stay.
- No `unsafe`, `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]` for the public surface.

### TypeScript SDK (`sdk/ts`, package `@claude-garage/plugin-sdk`)

Thin typed client for the daemon API; ESM only; **zero runtime dependencies** (uses global `fetch`, Node 20+).

- `src/types.gen.ts`: generated from the API schema by `json-schema-to-typescript` (a dev dependency; the output is committed, and `npm run check:gen` fails when it is stale).
- `src/index.ts`: `Api` (`fromEnv()`, constructor with `{base, plugin, token, fetch?}`), `readContext()`, `Context` accessors, `features()`, `listPlugins()`, `putWidget/clearWidget`, `notice`, `uiAction`, `sessionInput` (with `ifStatus.not`), `openPane/closePane`, `sessions()`, `workspaces()`, `text widget` helper, `GarageError` (status + body), and a generic `request()` escape hatch.
- Tests (`node --test`, compiled with `tsc`): a fake daemon (a `node:http` server that records requests and returns canned replies), covering URL building with encoded ids, headers, bodies, error mapping, context parsing and the generated types compiling against sample payloads.
- `package.json`: `exports` with types, `files: ["dist"]`, `engines`, `sideEffects: false`, `publishConfig: {access: public}`, `prepack` builds. Not published here.

The contract test of both SDKs against a real test daemon (task 2.3) is not part of this change; the fake daemons check what each SDK sends against the documented routes.

## A4. Order and checks

1. Golden file, features additions, freeze tests (tick p25 4.1 and 4.2).
2. API schema in `garage-protocol` and its test.
3. Rust SDK publish-readiness and additions.
4. TS SDK.
5. Docs, then the docs/golden consistency test.
6. Checks: `cargo clippy -D warnings` on the changed crates and `wall` lib/bins, crate tests, `wall` with `--lib --bins`, `npm test` and `npm run build` in `sdk/ts`. No real tmux, no real daemon.
