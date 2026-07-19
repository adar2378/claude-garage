# p7 design — VS Code-style grid + UX review fixes

## Context

The UX review (2026-07-19) and the validated interactive mockup (claude.ai artifact `c437a27b`) identified three trust-breaking moments — first five minutes, first wake-from-sleep, first worktree finish — plus the grid's vertical-stack default. Current state: `lib/layout.js#buildDefault` chains every panel `direction:"below"`; `reconcile` always splits the largest panel `below`; `SessionTerminal.jsx` prints `[detached]` on WS close with no retry; chrome keybindings die silently while a terminal has focus; the finish toast's `discard` is one-click destructive. dockview (already shipped, p4) is the layout engine — it natively supports directional splits and group maximize, so the grid work is thin wiring, not a new engine.

## Goals / Non-Goals

**Goals:**
- Balanced grid default + longest-axis placement, preserving persisted-layout round-trips.
- VS Code-style controls (split right/down, maximize, spawn menu) as wrappers over dockview + existing APIs.
- Kill the three trust-breakers: mode-trap, dead terminals, one-click discard.
- Make the attention signal glanceable (header badge, `document.title`).
- Reduce first-run friction (empty state, status legend, one-click hook install, inline errors).

**Non-Goals:**
- No VS Code-style "one terminal visible, others tabbed" model — all-sessions-visible stays the default; we adopt VS Code's *controls*, not its panel philosophy.
- No terminal scrollback replay on reconnect beyond what tmux redraws (the pane repaints on reattach; that is sufficient).
- No layout-contention fix for two pit-wall windows (stays in LATER.md).
- No Linux hook-install path changes beyond what the endpoint already implies (settings.json is platform-neutral).

## Decisions

**D-grid-default — balanced grid via `addPanel` math, not serialized-tree authoring.**
`buildDefault` computes `cols = ceil(sqrt(n))` and places panel *i* with `referencePanel` = row neighbor (`direction:"right"`) when `i % cols !== 0`, else the panel one row up (`direction:"below"`). Expressed through `api.addPanel` like today, so dockview keeps the grid tree consistent. Alternative considered: hand-authoring `fromJSON` grid snapshots — rejected; brittle against dockview schema changes and duplicates what `addPanel` already guarantees.

**D-grid-join — longest-axis split in `reconcile`.**
Replace the hardcoded `"below"` with `target.api.width > target.api.height ? "right" : "below"`. Largest-panel targeting stays. This is the same heuristic VS Code uses to keep splits balanced. Alternative: full re-layout on every join — rejected; discards user arrangements, which p4 explicitly protects.

**D-grid-controls — toolbar + tab buttons are thin wrappers.**
Split = `createSession(workspace, autoLabel, {worktree})` then, on the sessions refetch, the new id is placed by `reconcile` — except that explicit splits must honor the chosen cell/direction, so TerminalGrid records a one-shot placement hint `{id?, referencePanel, direction}` consumed by the next reconcile pass before falling back to the longest-axis rule. Auto-labels are `claude-N` (first free suffix), editable later via the existing rename-free label conventions (labels are immutable today; acceptable). Maximize uses dockview's group maximize (`api.maximizeGroup(panel.group)` / `api.exitMaximizedGroup()`); state is not persisted (a reload restores the normal grid). `m` and `\` join the existing single keydown listener with the same terminal-focus suppression rule.

**D-reconnect — retry loop inside SessionTerminal, existence-gated.**
On WS `close`, if the component is still mounted, schedule reconnect with capped exponential backoff (0.5s → 8s). Before each retry past the first, consult the latest sessions list (via a lightweight callback prop or module-level cache fed by App's SSE/sessions state) — if the id is gone, stop and render nothing (the panel is about to be reconciled away). The overlay is rendered by `SessionCellPanel` from a `connected` state the terminal reports upward via callback, keeping xterm imperative code where it already lives. Alternative: reconnect at a shared WS-manager layer — rejected as over-engineering for one socket type.

**D-conn-chip — SSE is the single health signal.**
The chip derives from App's existing `EventSource`: `error` → reconnecting, `open` → live (+ the existing resync-on-reopen). Per-terminal WS state deliberately does not feed the chip — one global signal, per-cell overlays carry the local detail.

**D-mode-chip — DOM focus is the mode source of truth.**
App already knows the rule (`document.activeElement.closest(".xterm")`). Add a `focusin`/`focusout` window listener that recomputes `{mode: "chrome" | sessionId}` and drives chip, footer strip note, and the transient hint toast. No xterm API coupling needed. The hint toast shows once per focus transition, auto-dismisses (~4s), and is suppressed by `prefers-reduced-motion`-safe CSS transitions only (content still appears).

**D-badge — derived, not stored.**
`needsCount = sessions.filter(s => s.status === "needs-input").length` in App; badge renders it, click calls the existing `jumpToNeedsInput`, and an effect mirrors it into `document.title`. Zero state renders quiet ("all clear") to keep header geometry stable.

**D-discard-confirm — reuse the armed-confirm pattern.**
Same 3s arm/disarm mechanics as SessionCellTab's ✕ (local state + timer), applied to the finish toast's discard button with destructive styling and a warning note naming the branch. No shared abstraction extracted yet (three call sites — rail delete, cell close, discard — is the threshold; extract if a fourth appears).

**D-hooks-install — daemon merges, UI one-click.**
`POST /api/hooks/install`: read `~/.claude/settings.json` (missing file → `{}`), parse (parse failure → 422, file untouched), deep-merge the snippet's `hooks` arrays with entry-level dedupe (match on command string), write backup `settings.json.garage-backup-<ISO>` first, then atomic write (tmp + rename). Idempotency comes from the dedupe. Banner gains the install button; its trigger condition adds `sessions.length > 0`. Alternative: keep manual-only — rejected by review finding #7; the daemon already owns the machine.

**D-empty-state — static JSX, no new capability plumbing.**
The onboarding card lives in TerminalGrid's existing empty branch; its CTA lifts `setShowAddWorkspace(true)` via a new callback prop from App. Rail keeps its one-liner.

**D-affordances — tokens and vocabulary only, no redesign.**
`index.css`: bump `--color-garage-faint`/`--color-garage-dim`, add `button { cursor: pointer }`. WorkspaceRail: `✕` → `unreg` text control (same armed confirm), padding bump on the control cluster. AddSessionControl: `wt` → `worktree`, error `!` → inline text line. Auto-collapse: a `matchMedia("(max-width: 1080px)")` listener in App forces `changesPaneCollapsed` while narrow and restores the prior value on widen (media-query-only CSS can't drive the existing grid-template state).

## Risks / Trade-offs

- [dockview maximize API surface differs across versions] → pin/verify the shipped dockview version's group-maximize API during implementation; fall back to a CSS-level "solo render" of the focused panel if absent (the mockup's approach).
- [Reconnect loop races session deletion — retrying a killed session's socket] → existence gate before retry (D-reconnect); reconcile removes the panel anyway, bounding the race to one failed attempt.
- [Placement hint vs. reconcile ordering — explicit split's session might arrive via SSE refetch before the hint is registered] → register the hint synchronously before firing `createSession`; hint entries expire after one consume or 10s.
- [Hook-merge could mangle a user's settings.json] → backup-then-atomic-write, parse-failure refusal, and entry-level dedupe; the snippet path remains as escape hatch.
- [`\` and `m` collide with terminal input expectations] → both are suppressed while a terminal has focus by the existing top-of-listener rule; only chrome mode is affected.
- [Balanced default surprises existing users with persisted stacks] → persisted layouts load unchanged; only genuinely layout-less workspaces (and explicit resets) get the new default.

## Migration Plan

Pure client + one additive daemon endpoint; no state migrations. Persisted dockview layouts, `~/.garage/state.json`, and hook snippets are all unchanged. Rollback = revert the commit; nothing on disk needs cleanup (backups written by hook-install are inert files).

## Open Questions

- Auto-label scheme for split-spawned sessions (`claude-N` vs `<focused-label>-N`) — default to `claude-N`; revisit if labels prove confusing in verification.
- Whether the maximize state should also suppress the changes pane for true full-bleed — deferred; verification will show whether the pane feels in the way.
