# pit-pet

## Purpose

An anthropomorphized attention badge: a small ASCII creature on the footer key strip whose mood is derived wall state, giving the pit wall an ambient, glanceable, emotional signal.

## Requirements

### Requirement: Opt-in pet with a species roster
The settings popover SHALL offer a pit-pet choice of `off` (default), `shop cat`, `rubber duck`, and `pit pup`, persisted with the other viewer settings. With `off` selected nothing renders — the pet must never surprise a user who did not ask for it.

#### Scenario: Off by default
- **WHEN** a user who has never opened the pet setting loads the pit wall
- **THEN** no pet renders anywhere

### Requirement: Mood is derived wall state
The pet's mood SHALL be computed from existing signals only — never stored: daemon unreachable → hiding (box); no sessions → sleeping; any session `needs-input` → alert (moving toward the rail with a `!` indicator); any session `working` → watching; otherwise sleeping. The last blocked session clearing SHALL trigger a transient celebration. Clicking the pet while alert SHALL perform the same jump as the `a` keybinding; clicking it otherwise is a harmless petting interaction.

#### Scenario: Pet mirrors a needs-input transition
- **WHEN** a session transitions to `needs-input` and is later answered
- **THEN** the pet goes alert (with the `!` indicator) for the duration and celebrates briefly once the count returns to zero

#### Scenario: Clicking the alert pet jumps
- **WHEN** the pet is alert and the user clicks it
- **THEN** focus jumps to a needs-input session exactly as pressing `a` would

### Requirement: Species-true animation
Each species SHALL express every state in its own motion vocabulary rather than sharing generic animations: the cat saunters, uses at most a subtle bounce, may ignore idle strolls, and celebrates by kneading in place; the duck never bounces — it waddles while walking, alerts with a single motionless stare and a static `!`, and celebrates with a single flap; the pup uses the fastest frame cadence and movement, a large bounce, and celebrates with zoomies. Rendering SHALL use theme-token colors and disable motion under `prefers-reduced-motion` (the pose still reflects mood).

#### Scenario: The duck does not bounce
- **WHEN** the rubber duck is selected and a session needs input
- **THEN** the duck shows its static stare with a non-pulsing `!` and no bounce animation, while the same state with the pup selected bounces
