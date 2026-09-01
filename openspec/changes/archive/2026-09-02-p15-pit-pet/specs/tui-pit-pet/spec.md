# tui-pit-pet

## ADDED Requirements

### Requirement: Opt-in roster cycled by `P`
The TUI wall SHALL offer a pit-pet choice of `off` (default), Arthur the
shop cat, Papito the rubber duck, and Segan the pit pup. Pressing `P` in
the garage layer SHALL cycle off → Arthur → Papito → Segan → off and show
a strip notice naming the arrival (or "the strip is quiet again" for
off). With `off` selected nothing pet-related SHALL render or speak. The
choice SHALL persist in `wall.json` as an optional `pet` field; a missing
or unrecognized value SHALL load as `off` with no error. The `?` help
overlay SHALL list `P`.

#### Scenario: Off by default
- **WHEN** a user who has never pressed `P` starts the wall
- **THEN** the strip filler is empty and no chatter ever appears

#### Scenario: Choice survives restart
- **WHEN** the user presses `P` twice (Papito) and restarts the wall
- **THEN** Papito renders on the first frame

### Requirement: One-row sprites and derived mood
The pet SHALL render as a single-row sprite inside the bottom strip's
filler columns, never displacing workspace tabs, the notice, the usage
chip, the blocked badge, or the keys-target chip; when the filler is
narrower than the widest sprite the pet SHALL be hidden for that frame.
Mood SHALL be computed from wall state only, never stored: daemon SSE
disconnected → box; no sessions → sleep; any session `needs-input` →
alert; any session `working` → watch; otherwise sleep. The last blocked
session clearing (blocked count > 0 → 0 with sessions present) SHALL
trigger a transient celebrate. Each species SHALL have its own sprite for
every mood; the alert sprite SHALL carry a `!` in amber.

#### Scenario: Mood mirrors a needs-input transition
- **WHEN** a session transitions to `needs-input` and is later answered
- **THEN** the pet shows its alert sprite with `!` for the duration and its celebrate sprite briefly once blocked count returns to zero

#### Scenario: Narrow strip hides the pet
- **WHEN** tabs plus right-side chips leave fewer filler columns than the current sprite needs
- **THEN** no sprite renders and the strip is otherwise unchanged

### Requirement: Species-true one-row motion
Motion SHALL run on a 300 ms tick while the pet is on. Alert SHALL move
the pet left toward the rail edge of the filler. The cat SHALL saunter
(slow cadence) and ignore roughly 40% of idle strolls, celebrating by
kneading in place. The duck SHALL waddle (alternating sprite offset)
while walking, alert with one static sprite and a non-blinking `!`, and
celebrate with a single flap. The pup SHALL use the fastest cadence and
step, alert with a blinking `!`, and celebrate with zoomies (rapid
left–right target flips). Off-tick frames SHALL NOT mark the frame dirty.

#### Scenario: The duck does not blink
- **WHEN** Papito is selected and a session needs input
- **THEN** the `!` stays lit on every tick, while the same state with Segan selected blinks it every tick

### Requirement: Click routing
A left click on the pet's columns in the strip SHALL, while alert,
perform the same jump as the `a` key (longest-waiting blocked session,
landing engaged). Otherwise it SHALL be a petting interaction: a short
celebrate and a species-voiced petting reply in the notice slot. Clicks
elsewhere in the strip SHALL route exactly as before.

#### Scenario: Clicking the alert pet jumps
- **WHEN** the pet is alert and the user clicks it
- **THEN** focus jumps to a needs-input session exactly as pressing `a` would

### Requirement: Species-voiced chatter
While the pet is on and not alert, the wall SHALL occasionally show one
chatter line in the strip notice slot for about 4 s, in the selected
species' voice (Arthur aloof, Papito deadpan, Segan eager). Lines SHALL
be at least 3 minutes apart, SHALL NOT replace a non-pet notice that is
still showing, SHALL NOT appear while any session needs input, and SHALL
NEVER be written into a tile. Chatter SHALL be off whenever the pet is
off. Earned lines: a celebrate SHALL prefer the proud intent; wall uptime
≥ 60 min SHALL weight stretch/hydrate up; local time 23:00–05:00 SHALL
enable late-night lines; account usage ≥ 80% on either window SHALL
enable a pace-yourself line (once per window crossing); box → live SHALL
say a "you're back" line once.

#### Scenario: Chatter yields to attention
- **WHEN** a chatter line is due but a session is `needs-input`
- **THEN** nothing is said until no session needs input

#### Scenario: Chatter never clobbers a real notice
- **WHEN** a chatter line is due while an armed-close notice ("press x again…") is showing
- **THEN** the chatter is deferred to a later tick and the notice stays

#### Scenario: Rate limit
- **WHEN** a chatter line was shown less than 3 minutes ago
- **THEN** no new line is shown, even if an earned trigger fires (except the "you're back" and proud-on-celebrate lines, which are event-bound and exempt)
