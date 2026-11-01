## ADDED Requirements

### Requirement: Motion only while something happens
The TUI SHALL animate only to show work in progress or a state change: the startup reveal while the daemon starts, spinners while tiles attach or restore and while the daemon restarts, the `done` fade, tile fade-in and close dissolve. When no effect is running, the TUI SHALL NOT redraw on a timer.

#### Scenario: Idle wall costs nothing
- **WHEN** no session changes state and no effect is running
- **THEN** the TUI makes no timed redraws

#### Scenario: Daemon already running
- **WHEN** the user launches the wall and the daemon is already up
- **THEN** the wall opens without the startup reveal

### Requirement: Amber stays reserved
Motion SHALL NOT use amber except for one short pulse when a session enters `needs-input`.

#### Scenario: Done fade colour
- **WHEN** a session is `done` and fading
- **THEN** it fades in green and neutral tones, never amber

### Requirement: Motion can be turned off
Setting `animations = false` in the garage config, or `GARAGE_NO_MOTION`, `NO_MOTION` or `REDUCE_MOTION` in the environment, SHALL disable every effect and the startup reveal, and spinners SHALL render as a static marker.

#### Scenario: Motion off
- **WHEN** `GARAGE_NO_MOTION=1` is set and a tile attaches
- **THEN** the tile shows a static marker until attached, with no animation

### Requirement: Motion never delays input
Keystrokes SHALL reach the session before any effect is drawn; a running effect SHALL NOT add latency to input.

#### Scenario: Typing during an effect
- **WHEN** the user types into an engaged tile while another tile fades in
- **THEN** each keystroke reaches the session as fast as with motion off
