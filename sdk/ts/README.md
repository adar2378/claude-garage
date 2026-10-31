# @claude-garage/plugin-sdk

A small typed client for the [claude-garage](https://github.com/adar2378/claude-garage) plugin API.
Zero runtime dependencies. Node 20 or later.

```ts
import { Api, readContext, textWidget } from "@claude-garage/plugin-sdk";

const ctx = await readContext(); // from GARAGE_CONTEXT, GARAGE_CONTEXT_FILE or stdin
const api = Api.fromEnv(); // GARAGE_API, GARAGE_PLUGIN_ID, GARAGE_PLUGIN_TOKEN

await api.putWidget("bar", textWidget(`in ${ctx.workspace ?? "no workspace"}`));
await api.notice("pushed");
```

The manifest, permissions, events and widget format are described in
[docs/plugins](https://github.com/adar2378/claude-garage/tree/main/docs/plugins).
Your manifest must declare the widget and the permission (`ui.widgets`, `ui.notice`, ...) each call needs.

## What it covers

| Call | Route | Needs |
|---|---|---|
| `features()` | `GET /api/plugin/features` | nothing |
| `sessions()`, `workspaces()`, `usage()`, `health()` | core read routes | nothing |
| `closedChats(ws)` | `GET /api/workspaces/{ws}/closed` | `sessions.read` |
| `plugins()`, `widgets()`, `panes()` | `GET /api/plugins...` | nothing |
| `putWidget(id, put)`, `clearWidget(id)` | `PUT/DELETE /api/plugins/{me}/widgets/{id}` | `ui.widgets` |
| `notice(text, ttlMs?)` | `POST /api/plugins/{me}/notice` | `ui.notice` |
| `uiAction(action, target, args?)` | `POST /api/plugins/ui/action` | `ui.invoke` (+ `ui.navigate` or `core.actions` for core ids) |
| `sessionInput(id, text, {ifStatusNot})` | `POST /api/sessions/input` | `sessions.input` (dangerous) |
| `openPane(id)`, `closePane(id)` | `POST/DELETE /api/plugins/{me}/panes/...` | `ui.panes` |
| `request(method, path, body?)` | any route | what the route needs |

A non-2xx reply rejects with `GarageError` (`status`, `body`). A daemon that cannot be reached is `status` 0.

## Types

`src/types.gen.ts` is generated from `schemas/garage-plugin-api.schema.json`, which the Rust protocol crate
writes from the same types the daemon uses. After the schema changes: `npm run gen`. `npm test` fails when the file is stale.

## Develop

```sh
npm install
npm test        # checks generated types, builds, runs tests against a fake daemon
npm run build   # dist/
```
