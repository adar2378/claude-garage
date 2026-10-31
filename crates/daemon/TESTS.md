# Ported tests (JS -> Rust)

## Lane C: telemetry

Fake `TelemetryDeps` (token, set_status, session resolution) and temp dirs for `claude_home`; no tmux, no `claude`, no real `~/.claude`/`~/.garage`.

| JS test | Rust test |
|---|---|
| hooks.test.js: merge preserves unrelated settings and hooks | `api::hooks::tests::merge_preserves_unrelated_settings_and_hooks` |
| hooks.test.js: merge is idempotent | `api::hooks::tests::merge_is_idempotent` |
| hooks.test.js: idle reminder is noise | `api::hooks::tests::idle_reminder_is_noise_blockers_and_unknowns_are_not` |
| statusline.test.js: clampPercentage | `state::context::tests::clamp_percentage_clamps_and_rejects_non_numbers` |
| statusline.test.js: get/set/drop context, rate limits (5 tests) | `state::context::tests::*` |
| statusline.test.js: 401, ignored, resolvable (cwd fallback via fake resolver), model window, rate-only, snippet | `api::statusline::tests::*` |
| statusline.install.test.js: 6 sequential tests | `api::statusline::tests::install_flow_create_idempotent_corrupt_chain_rechain_and_fresh` (one sequence; runs the generated wrapper with `sh`) |
| transcript.test.js: all 21 | `transcript::tests::*` (TTL test uses an injected clock instead of mocked `Date`) |
| usage.test.js: 4 tests | `api::statusline::tests::usage_is_null_then_reflects_and_merges_posts` |
| token-auth.js (no JS test) | `security::token_tests::matches_only_the_exact_token` |
| settings-install.js / claude-home.js (no JS test) | `install::tests::*` |

New (no JS equivalent): hook route auth (401 incl. repeated `token`), ignored events, idle-reminder filter, Notification/Stop status calls with three-valued message, malformed body 400, hook snippet, hook install (create, idempotent, backup, 422).

Not ported: the "real tmux" variants (session creation in statusline.test.js); session resolution is lane B and is faked here.
