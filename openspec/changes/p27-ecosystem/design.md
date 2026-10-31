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
