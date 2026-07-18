# Design: p1-pit-wall

## Context

P0 (`archive/2026-07-19-p0-terminal-bridge`) proved the tmux ⇄ node-pty ⇄ WebSocket ⇄ xterm.js pipeline for one hardcoded session (D1–D7: tmux session name as registry, Fastify + `ws`, one `node-pty` per socket, client-driven resize, binary/JSON WS framing, npm-workspaces monorepo, detached spawn). P1 builds the actual product thesis identified in IDEA.md and confirmed unclaimed by GAP.md: all terminals of a workspace visible at once, and needs-you-first triage that works even when the pit wall isn't on screen. That requires four new pieces of daemon state/behavior (workspace registry, status store, push channel, notifications), a breaking change to the spawn contract, and a UI keybinding model that can coexist with typing directly into a live terminal. This design stays consistent with P0's philosophy: tmux remains the sole session source of truth; anything new is either derived, ephemeral, or a thin directory-mapping file.

## Goals / Non-Goals

**Goals:**
- Define a `StatusStore` abstraction and state model that is agnostic to the detection mechanism, since the mechanism itself is chosen by a separate implementation-time spike.
- Define the workspace registry (`~/.garage/state.json`) format, write discipline, and its boundary (directory mapping only, never a session list).
- Define how the terminal grid extends P0's attach-per-socket model to multiple simultaneous terminals, focus-scoped to one workspace.
- Choose the push mechanism that gets status changes from daemon to UI.
- Define the macOS notification integration with no new dependency, and its Linux behavior.
- Define the breaking `POST /api/sessions` change and the new workspace-registration endpoint.
- Define the keybinding/focus model that lets single-key pit-wall shortcuts coexist with typing into a focused terminal.

**Non-Goals:**
- Re-litigating the status detection mechanism during implementation — it was resolved by pre-implementation research (see D-status: hybrid poller + HTTP hooks); the remaining spike task only verifies the hybrid live.
- Diff panel, review mode, VS Code jump (P2).
- Reboot restore, `npx` packaging (P3).
- tmux-native deck layout (`garage deck`) — deliberately deferred per IDEA.md's surface decision, not part of this change.
- Any auth model beyond P0's existing origin allowlist.
- Windows support.

## Decisions

**D-status — `StatusStore` abstraction, state model, and decay.**
Four states per session id: `needs-input` (Claude is blocked on the user — a prompt, a question, a tool-permission confirmation), `working` (actively generating or running tools), `done` (just finished a turn — worth a glance, not blocking), `idle` (resting, nothing to review). `done` decays to `idle` on whichever comes first: a fixed timeout since entering `done` (proposed ~2 minutes, tunable — see Open Questions), or the session becoming the UI's app-focused session (the user has now seen it). `needs-input` has no automatic decay; it only clears when a new `working`/`done` event arrives for that session (the user actually responded).

The store itself is one daemon-side module: `Map<sessionId, {state, since}>` plus a pub-sub so the push channel (D-push) can subscribe to transitions. Exactly one function, `setStatus(sessionId, state)`, is the only write path; everything downstream (API, SSE, notifications) only ever reads the store. Two ingestion sources can feed that function without either being decided here:
1. `POST /api/hooks/claude` — receives Claude Code hook payloads (`Notification` → `needs-input`, `Stop` → `done`).
2. A poller shelling out to `claude agents --json` on an interval, diffing reported states into the store's vocabulary.

**Mechanism: RESOLVED (pre-implementation research, 2026-07-19) — hybrid, both sources feed the store.** Verified against Claude Code 2.1.214 and official docs:
- `claude agents --json` exists and returns `{pid, cwd, sessionId, status}` per live session, but `status` is only `busy | idle` — it CANNOT distinguish "waiting for permission/input" from plain idle. Polling alone can never light the amber dot correctly, so hooks are not optional.
- Claude Code hooks support a native **HTTP type** (`{"type": "http", "url": ...}` + `allowedHttpHookUrls` in settings) — the daemon receives hook POSTs directly, no curl wrapper. `Notification` (matchers `permission_prompt`, `idle_prompt`) → `needs-input`; `Stop` → `done`. Payloads carry `session_id` and `cwd`.
- Division of labor: the **poller** (interval ~2s) provides zero-setup baseline — session liveness plus `working`/`idle` from `busy`/`idle` — so garage degrades gracefully when hooks aren't installed; **hooks** provide the precision states (`needs-input`, `done`) at sub-second latency. Hook events always win over poller events for the same session (richer semantics); the poller never downgrades `needs-input` to `idle` while the underlying claude process still reports no new activity — only a hook (`Stop`/new `working`) or the poller observing `busy` clears it.

Mapping an event to a session — SOLVED exactly, no cwd guessing needed: garage spawns `claude` as the tmux pane's root process, so `tmux list-panes -a -F "#{session_name} #{pane_pid}"` gives pane_pid == the `pid` reported by `claude agents --json`. That join maps agents-json entries to garage session ids exactly, even when two sessions share a cwd. Hook payloads carry `session_id`, which correlates to `sessionId` in `agents --json` (whose `pid` then joins to the pane) — so hook events also resolve exactly. Fallback when a join fails (e.g. claude exec'd through a wrapper): fail open on `cwd` match — apply to all sessions with that dir, over-notify rather than silently drop.

The former "spike" task is now a **verification spike**: prove the hybrid live (hook fires → store updates → SSE within budget; poller-only degradation works with hooks uninstalled) rather than choose a mechanism. Setup burden: P1 ships a `GET /api/hooks/snippet` helper returning the exact JSON block (hooks + `allowedHttpHookUrls`) for `~/.claude/settings.json`, and the UI shows a one-time banner when status events are poller-only.

**D-push — SSE over a second WebSocket or polling.**
Recommend `GET /api/events` as a Server-Sent Events stream. Status flows one direction only (daemon → UI), which is exactly what SSE is for; `EventSource` auto-reconnects with zero client code; Fastify serves it with a plain `reply.raw.write` loop, adding no new dependency (unlike a second `ws` upgrade handler duplicating what `term.js` already does for a channel that doesn't need bidirectional bytes). Polling `GET /api/sessions` is rejected — any interval tight enough to feel live fights the "attention routing" latency goal, and one loose enough to be cheap feels laggy, which is the exact competitive gap this change fixes. A second WS is rejected only because the traffic is one-directional and SSE generalizes cleanly to more event types later (e.g. P2 diff-ready pushes) without protocol renegotiation.

**D-registry — `~/.garage/state.json`.**
Format: `{"workspaces": {"<name>": {"dir": "<absolute path>"}}}`. Created lazily on the first `PUT /api/workspaces` call, not at daemon startup — consistent with "daemon is almost stateless": a user who registers nothing has no file to migrate or corrupt. Writes are atomic: write to a temp file in the same directory, then `rename()` (atomic on the same filesystem), so a crash mid-write never leaves a partially-written file — readers see the old or new complete content, never a partial one. The registry is explicitly NOT a session store: it maps workspace name → directory for spawn-time resolution (D-api) and later P3 restore metadata only. It is never consulted for which sessions exist or their status — `tmux ls` remains the sole source of truth for that, unchanged from P0's D1.

**D-grid — focus-scoped attach-per-terminal.**
Extends P0's D3 (one `node-pty` per WebSocket): every session belonging to the currently-focused workspace gets its own `/term/:id` socket + pty attach as soon as it's visible in the grid. Switching focus to a different workspace closes the previous workspace's sockets (killing those ptys via `term.js`'s existing close-on-disconnect behavior) and opens new ones for the newly-focused workspace. Non-focused workspaces carry zero pty cost — the workspace rail shows them as name + last-known status glyph from `StatusStore` only, no live terminal. Design center is ~6 sessions per workspace (IDEA.md's `1–9` keybinding and pit-wall description), so worst case is ~6 concurrent ptys/sockets at any time, not one per session across every workspace the user has ever created.

tmux's smallest-client sizing (flagged as a P0 risk) recurs whenever a grid pane and an iTerm `tmux attach` escape-hatch coexist on the same session — tmux still sizes to the smallest attached client. P1 does not solve this; same acceptance as P0. It's softened in practice because grid cells are already smaller than P0's full-screen terminal, so the visual "shrink" from an external attach is less jarring than it would be in a single-terminal layout.

**D-notify — `osascript` subprocess on `needs-input` transition.**
`osascript -e 'display notification "<label>" with title "claude-garage"'`, fired edge-triggered — once per transition INTO `needs-input`, not once per poll tick — to avoid repeat spam while a session sits blocked. No new npm dependency: reuses the same `child_process.execFile` pattern `tmux.js` already uses. Platform-gated on `process.platform === "darwin"`; on Linux this is a no-op for now, per IDEA.md's "macOS first, Linux should work" split — notification is the one piece that's explicitly macOS-only rather than merely macOS-first.

**D-api — breaking spawn change + workspace registration.**
`POST /api/sessions` body changes from `{workspace, label, dir}` to `{workspace, label}`; `dir` is resolved server-side from the registry (`workspaces[workspace].dir`), 400 if the workspace is unregistered. This is a breaking change to the P0 contract; accepted per the proposal because no external consumers exist. New endpoint `PUT /api/workspaces` with body `{name, dir}` — PUT rather than POST because registration is an idempotent "declare this workspace's directory" upsert keyed by `name`, not creation of a server-assigned resource: calling it twice with the same body is a no-op, calling it again with a different `dir` updates the mapping. `name` reuses the existing `NAME_RE` (`[a-z0-9-]+`) validation from `tmux.js`; `dir` must `stat` as an existing directory, mirroring the check `sessions.js` already performs.

**D-keys — chrome bindings vs. terminal typing.**
Single `keydown` listener at the app-chrome level (not per-component). Pit-wall bindings (`1–9`, `[`/`]`, `a`, and later `j/k`, `r`/`Esc`, `v`, `o`) fire only when DOM focus is outside any terminal (`document.activeElement.closest(".xterm")` is null). Clicking into a terminal's content area gives it DOM focus — keystrokes go straight to the pty and Claude Code's own TUI keybindings (including its own `Escape` handling) pass through completely untouched — and marks it the app-focused session. Clicking outside any terminal (workspace rail, header, grid gutter) blurs the active terminal, returning to chrome-navigation mode where single-key bindings are live again. Rejected: a modifier scheme (e.g. `Ctrl+1..9`) — it would rewrite every binding IDEA.md already fixes, and modifiers still collide with terminal-native chords (`Ctrl+A`, `Ctrl+[` are meaningful in tmux/readline/Claude Code), so it relocates the coexistence problem instead of resolving it. Trade-off: in P1 there is no keyboard-only way to blur a focused terminal (see Open Questions).

## Risks / Trade-offs

- [pty-per-terminal cost scales with grid size] → capped by focus-driven attach (only the focused workspace's sessions hold a pty; design center ~6), and by killing sockets/ptys immediately on workspace switch (reuses P0's close-kills-pty behavior).
- [Hook setup UX: `~/.claude/settings.json` must be hand-edited for full precision] → hybrid degrades gracefully to poller-only (working/idle still correct, amber dot reduced to best-effort); `GET /api/hooks/snippet` + a UI banner make installation a paste, not a scavenger hunt.
- [pid join could fail if claude is wrapped (shell alias, script) so pane_pid ≠ claude pid] → fail open on `cwd` match — apply the event to all sessions in that dir; over-notify, never silently drop.
- [xterm.js rendering ~6 live terminals simultaneously may cost more CPU than P0's single terminal] → bounded by the same focus-driven cap as the pty limit; if it becomes a measured problem, revisit with xterm's WebGL renderer addon rather than adding it preemptively.
- [tmux smallest-client sizing shrinks a grid pane when an iTerm escape-hatch attach coexists] → same documented acceptance as P0 D4; grid cells are already smaller than P0's full-screen terminal, softening the effect; revisit with `aggressive-resize` if it's a real complaint.
- [SSE stream drop leaves the UI silently stale] → `EventSource` auto-reconnects; on reconnect the UI does one `GET /api/sessions` (which embeds current status) to resync instead of assuming the stream resumed mid-sequence.
- [Registry file corruption from an interrupted write] → temp-file + `rename()` atomic write; single daemon process means no cross-process write races in practice.

## Open Questions

- Keyboard-only path to blur a focused terminal back to chrome-navigation mode — P1 ships mouse-click-only; worth a lightweight chord later if it proves annoying.
- Exact `done` → `idle` decay timeout (proposed ~2 minutes) — tune from real usage rather than fixing now.
- Poller interval: starting at 2s (satisfies the ≤2s spec budget for poller-sourced transitions); tune if CPU cost is measurable.
