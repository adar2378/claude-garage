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

## Authoring tools and discovery (detailed design)

Code: `crates/cli/src/plugin/{scaffold,dev,search}.rs`, templates in `crates/cli/templates/`, example in `examples/openspec-progress`, skill in `docs/plugins/skill/SKILL.md`. All three commands run without a daemon except the reload in `dev`, and every seam (`Fetch`, `Builder`, `Admin`, `Ui`) is the one `install` already uses, so tests use fakes and temp dirs.

### `plugin new <name> [--lang rust|ts|shell]`

- `<name>` is the plugin id and the folder, created in the current directory; the id rule and the reserved words are the manifest checker's. An existing folder is never touched.
- Templates are embedded files with `@@ID@@`-style placeholders (not `{{ }}`, which clash with Rust and JS braces). Every language gets: `garage-plugin.toml`, `README.md`, `CLAUDE.md` (for plugin authors: manifest rules, run context, widget format, limits, dev loop), `.gitignore`, one action (`greet`), one `strip.right` widget (`status`), a `daemon.started` event that re-pushes it (widgets live in memory), and a test.
- Rust: standalone Cargo project (own `[workspace]`), test uses the SDK's fake daemon. Shell: `plugin.sh` + `test.sh` with a fake `curl`. TypeScript: `tsc` to `dist/`, `node --test`, no runtime dependencies (global `fetch`).
- The SDK is not on crates.io, so the Rust template depends on `git = ".../claude-garage", tag = "v<this garage version>"`. A hidden `--sdk-path` points it at a local checkout (used to verify the template builds).
- Rendered files are checked with `parse` + `check_host` BEFORE anything is written, so a template bug fails in our tests (one test per language) and never produces a plugin that `plugin link` refuses.
- Output: the folder and the next steps (`plugin dev .`, or `plugin link .`).

### `plugin dev [path]`

1. `sync` = (a) link when not linked, linked to another folder, or the approved surface changed; (b) run `[[build]]` unless only the manifest changed; (c) `POST /api/plugins/reload` through `Admin::reload` (UI token and identity challenge as for other writes; a no-op when the daemon is down, it loads at start).
2. **Consent covers builds.** `plugin link` runs no build, `plugin dev` does, so the dev preview (`Mode::Dev`) lists every `[[build]]` argv ("every reload"). The hash of the approved surface (permissions, all commands including builds, URLs; not titles, versions or keys) is stored in `<garage>/plugins/dev-approved/<id>`, outside the plugin's writable data dir. On every sync a different hash stops everything and prompts again BEFORE any build or reload; non-interactive without `--yes` refuses; declining changes nothing. Editing code, titles or versions never prompts.
3. Watching is a 400 ms poll of (mtime, size) over the folder, skipping `.git`, dot-directories, `target`, `node_modules`, `dist`, `build`, logs and editor temp files (so build output cannot trigger itself). A change must be seen in two consecutive polls before acting (one reload per save, not per file). No watcher crate: same approach as the daemon's manifest poller.
4. A failed build or invalid manifest prints the error and the last good version keeps running; the loop continues. The plugin log is tailed in the same terminal. Ctrl+C stops it (SIGINT flag on Unix); the plugin stays linked.
5. If the first link is declined, `dev` exits instead of watching nothing.

### `plugin search [words] [--json] [--sort stars|updated] [--limit n]`

- One HTTPS GET to `https://api.github.com/search/repositories?q=topic:claude-garage-plugin <words> in:name,description,readme` through the existing `CurlFetch` (`--proto =https`, no `~/.curlrc`, size cap); no new dependency. Default order is stars (open question resolved: stars by default, `--sort updated` for activity).
- Shows `owner/repo`, description, stars, last push (relative) and the exact `plugin install` line. The first line is always the UNREVIEWED warning; `--json` carries `"unreviewed": true`. The search cannot confirm a valid `garage-plugin.toml` (that needs authenticated code search), so the footer says a repository without one will not install.
- Descriptions are written by strangers: control and invisible characters are stripped and text is cut to one line before printing. A 403/429 is explained as GitHub's 10 searches a minute without a login.
- Tests fake the GitHub response through the `Fetch` seam; no network.

### Example plugin `examples/openspec-progress`

Not bundled; its own Cargo project built only on `garage-plugin-sdk`. A `timer` event (5 s, the minimum) and `daemon.started` run `tick`: for each workspace from `GET /api/workspaces` it counts checkboxes in `<dir>/openspec/changes/*/tasks.md` (skipping `archive`, code fences and files with no checkboxes) and PUTs a `rail.header` widget with a scope per workspace (`spec 12/30`, green when complete, clickable); with nothing to show it clears the widget. Pane `changes` (overlay, action `openspec-progress:changes`) lists each change of the focused workspace with a bar, done/total and percent. Permissions: `ui.widgets`, `ui.panes`, `workspaces.read`. "Watches via a timer" means no file watcher and no resident process.

### Skill

`docs/plugins/skill/SKILL.md` is a Claude Code skill (install by copying to `~/.claude/skills/garage-plugin/`). It maps a sentence to pieces (widget, action, event, pane, cli), picks a language, runs `plugin new`, tells the agent to treat the scaffold's `CLAUDE.md` as the authority, requires tests, minimal permissions, and leaves running `plugin dev` to the user.

### Left for later

TypeScript scaffold is built from the API as it is; when the generated TS SDK exists the template should import it. The example is published as its own repo with the topic (task 4b.4) once the repo exists.
