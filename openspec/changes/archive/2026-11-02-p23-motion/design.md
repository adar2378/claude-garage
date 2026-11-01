## Context

The render loop redraws on events (input, PTY output, daemon events). The pit pet already animates by hand with its own tick. p22 splits the render loop and slots out of `runtime.rs`; this change builds on that.

## Decisions

**1. One motion clock, active only while an effect runs.** A motion module owns running effects. While any is running, it asks the loop for a redraw at about 30 fps; when none is running, it asks for nothing, and the loop is event-only again.

**2. Effects never sit between input and the session.** Key handling and PTY writes run before rendering, unchanged. An effect only changes how a frame is drawn.

**3. Amber is reserved.** `needs-input` gets at most one short attention pulse when it starts. No other effect uses amber.

**4. Off switch.** `animations = false` in `~/.garage/config.toml` or `GARAGE_NO_MOTION=1` (or the common `NO_MOTION`/`REDUCE_MOTION` variables) disables every effect and the startup reveal. Spinners become a static "…" marker.

**5. Startup hand-off.** When the launcher spawns the daemon it does not wait: it starts the wall with `GARAGE_DAEMON_STARTING=1`, `GARAGE_DAEMON_LOG`, `GARAGE_DAEMON_PID` and an explicit `GARAGE_PORT`, and the wall waits for `/api/health` behind the reveal (15 s). During the reveal only Ctrl+C aborts; every other key (including Esc and q) is dropped, so a stray keystroke cannot cancel a start. If `GARAGE_DAEMON_PID` is set and `kill(pid, 0)` reports no such process (unix only), the wall fails fast with a "daemon exited during startup, see <log>" message instead of waiting out the deadline. A terminal IO error during the reveal prints its own message, not "did not come up".

**6. A dead daemon does not keep the wall awake.** The strip spinner runs at most 15 s after the SSE stream drops or a restart is requested; after that the strip shows a static `◌ daemon down` and the motion clock asks for no tick.

**7. Fade-in is for first attach only.** A tile fades in the first time its session attaches in this run, not when a workspace or view switch re-attaches it. The `done` fade interpolates in RGB at every step, endpoints included, and is never amber.

## Risks / Trade-offs

- Slow terminals over SSH: the off switch covers it.
- Crate churn: all three crates are small and pinned; effects sit behind the motion module, so swapping one is local.

## Resolved (2026-10-31, user: all defaults)

- Off switches: `animations = false`, `GARAGE_NO_MOTION`, `NO_MOTION`, `REDUCE_MOTION` (all honoured).
