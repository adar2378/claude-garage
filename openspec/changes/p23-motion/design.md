## Context

The render loop redraws on events (input, PTY output, daemon events). The pit pet already animates by hand with its own tick. p22 splits the render loop and slots out of `runtime.rs`; this change builds on that.

## Decisions

**1. One motion clock, active only while an effect runs.** A motion module owns running effects. While any is running, it asks the loop for a redraw at about 30 fps; when none is running, it asks for nothing, and the loop is event-only again.

**2. Effects never sit between input and the session.** Key handling and PTY writes run before rendering, unchanged. An effect only changes how a frame is drawn.

**3. Amber is reserved.** `needs-input` gets at most one short attention pulse when it starts. No other effect uses amber.

**4. Off switch.** `animations = false` in `~/.garage/config.toml` or `GARAGE_NO_MOTION=1` (or the common `NO_MOTION`/`REDUCE_MOTION` variables) disables every effect and the startup reveal. Spinners become a static "…" marker.

## Risks / Trade-offs

- Slow terminals over SSH: the off switch covers it.
- Crate churn: all three crates are small and pinned; effects sit behind the motion module, so swapping one is local.
