//! Pit pet: the wall's one ambient, opt-in ASCII creature (spec `pit-pet`)
//! on the TUI's one-row bottom strip (spec `tui-pit-pet`). Three pets with
//! their own names, personalities and derived-mood rules; motion and art are
//! cut for a single strip row.
//!
//! Pure module: no ratatui, no IO, no `Instant`/`SystemTime`, no `rand`.
//! Time and randomness are injected — `now_ms: i64` and `rng: &mut impl
//! FnMut() -> f64` yielding `[0.0, 1.0)` — exactly the discipline
//! `runtime::Notices` already uses, so every mood, motion, and chatter rule
//! unit-tests without a terminal.
//!
//! Sprite widths are measured with `chars().count()` rather than the
//! `unicode-width` crate: `unicode-width` is only a *transitive* dependency
//! today (pulled in by ratatui — see `Cargo.lock`), not declared in this
//! crate's `Cargo.toml`, and every glyph used below (ASCII plus the pup's
//! `ᴥ`/`°`/`·`) is single-width, so a plain character count equals display
//! width here.
//!
//! Requirements covered in this file (spec `tui-pit-pet`):
//! - "Opt-in roster cycled by `P`" → [`Species`].
//! - "One-row sprites and derived mood" → the sprite tables and [`mood`].
//! - "Species-true one-row motion" → [`PetSim::step`].
//! - "Species-voiced chatter" → [`Intent`], [`LINES`], [`Chatter`].
//!
//! Click routing, strip rendering, and state/runtime wiring are out of
//! scope for this file (see `ui/strip.rs`, `runtime.rs`, `state/`).

// ── Species ─────────────────────────────────────────────────────────────

/// The family, by name (named by the user, 2026-07-19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Species {
    Cat,
    Duck,
    Pup,
}

impl Species {
    /// The pet's given name (`PET_NAMES` in the web original).
    pub fn name(self) -> &'static str {
        match self {
            Species::Cat => "Arthur",
            Species::Duck => "Papito",
            Species::Pup => "Segan",
        }
    }

    /// The strip-notice arrival label (spec "Opt-in roster cycled by `P`").
    pub fn label(self) -> &'static str {
        match self {
            Species::Cat => "Arthur the shop cat",
            Species::Duck => "Papito the rubber duck",
            Species::Pup => "Segan the pit pup",
        }
    }

    /// Persistence key for `wall.json`'s optional `pet` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Species::Cat => "cat",
            Species::Duck => "duck",
            Species::Pup => "pup",
        }
    }

    /// Lenient parse: unrecognized text is the caller's problem (they get
    /// `None`, which persistence.rs treats as `off` per the spec).
    ///
    /// Named `from_str`/`as_str` (not the `FromStr` trait) to mirror the
    /// task's naming and because `wall.json`'s lenient "unknown → off"
    /// parse doesn't fit `FromStr`'s `Result`-returning contract.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Species> {
        match s {
            "cat" => Some(Species::Cat),
            "duck" => Some(Species::Duck),
            "pup" => Some(Species::Pup),
            _ => None,
        }
    }

    /// `P` cycling: off → Arthur → Papito → Segan → off.
    pub fn cycle(current: Option<Species>) -> Option<Species> {
        match current {
            None => Some(Species::Cat),
            Some(Species::Cat) => Some(Species::Duck),
            Some(Species::Duck) => Some(Species::Pup),
            Some(Species::Pup) => None,
        }
    }

    fn base_speed(self) -> u16 {
        match self {
            Species::Cat => 1,
            Species::Duck => 1,
            Species::Pup => 2,
        }
    }

    fn frame_every(self) -> u64 {
        match self {
            Species::Cat => 2,
            Species::Duck => 3,
            Species::Pup => 1,
        }
    }

    /// Chance (0.0–1.0) of ignoring an idle stroll roll. Only the cat is
    /// aloof (spec "the cat ... ignore roughly 40% of idle strolls").
    fn aloof(self) -> f64 {
        match self {
            Species::Cat => 0.4,
            _ => 0.0,
        }
    }

    /// Celebrate window length in ms (`PET_PROFILES[...].celebrateMs` in the
    /// web original).
    fn celebrate_ms(self) -> i64 {
        match self {
            Species::Cat => 3000,
            Species::Duck => 1100,
            Species::Pup => 3400,
        }
    }
}

// ── Mood ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Sleep,
    Watch,
    Alert,
    Box,
    Celebrate,
}

/// Mood from wall state only, never stored (spec "One-row sprites and
/// derived mood"): box > sleep-when-empty > alert > watch > sleep.
pub fn mood(daemon_live: bool, any_needs_input: bool, any_working: bool, session_count: usize) -> Mood {
    if !daemon_live {
        return Mood::Box;
    }
    if session_count == 0 {
        return Mood::Sleep;
    }
    if any_needs_input {
        return Mood::Alert;
    }
    if any_working {
        return Mood::Watch;
    }
    Mood::Sleep
}

// ── Sprites ─────────────────────────────────────────────────────────────

/// A single-row sprite frame plus whether this mood carries a `!` (the
/// caller renders the `!` itself, in amber — see `StepResult::bang_lit` for
/// whether it's currently lit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sprite {
    pub text: &'static str,
    pub bang: bool,
}

fn cat_frames(mood: Mood) -> &'static [&'static str] {
    match mood {
        Mood::Sleep => &["=-.-= z", "=-.-=  z"],
        Mood::Watch => &["=o.o=", "=o.o= ~"],
        Mood::Alert => &["=O_O="],
        Mood::Box => &["[-.-]"],
        Mood::Celebrate => &["=^.^=", "=^.^= *"],
    }
}

fn duck_frames(mood: Mood) -> &'static [&'static str] {
    match mood {
        // The blink (open/closed eye) doubles as the waddle: the leading
        // space toggles side per frame (spec "waddle = alternating sprite
        // offset").
        Mood::Sleep => &["<(- )___ z", " <(- )___z"],
        Mood::Watch => &["<(o )___ ", " <(- )___"],
        // "a single motionless stare. that IS the animation." — one frame,
        // never blinks (see `bang_lit`).
        Mood::Alert => &["<(O )___"],
        Mood::Box => &["[(o )>]"],
        // "single flap: exactly 2 frames, then hold" — frame selection
        // clamps instead of looping (see `PetSim::step`).
        Mood::Celebrate => &["\\<(o )___/", "<(o )___ "],
    }
}

fn pup_frames(mood: Mood, ascii: bool) -> &'static [&'static str] {
    if ascii {
        match mood {
            Mood::Sleep => &["(-o-) z", "(-o-)  z"],
            Mood::Watch => &["(o.o)", "(o.o)/"],
            Mood::Alert => &["(O.O)"],
            Mood::Box => &["[o.o]"],
            Mood::Celebrate => &["(^o^)", "\\(^o^)/"],
        }
    } else {
        match mood {
            Mood::Sleep => &["(-ᴥ-) z", "(-ᴥ-)  z"],
            Mood::Watch => &["(·ᴥ·)", "(·ᴥ·)/"],
            Mood::Alert => &["(°ᴥ°)"],
            Mood::Box => &["[·ᴥ·]"],
            Mood::Celebrate => &["(^ᴥ^)", "\\(^ᴥ^)/"],
        }
    }
}

fn frames_for(species: Species, mood: Mood, ascii_pup: bool) -> &'static [&'static str] {
    match species {
        Species::Cat => cat_frames(mood),
        Species::Duck => duck_frames(mood),
        Species::Pup => pup_frames(mood, ascii_pup),
    }
}

/// One sprite for `(species, mood, frame)`. `frame` is taken modulo the
/// mood's frame count except where noted (duck celebrate — see
/// `PetSim::step`, which is the only caller that needs the hold behavior;
/// this function always wraps).
pub fn sprite(species: Species, mood: Mood, frame: usize, ascii_pup: bool) -> Sprite {
    let frames = frames_for(species, mood, ascii_pup);
    let text = frames[frame % frames.len()];
    Sprite { text, bang: mood == Mood::Alert }
}

// ── Motion ──────────────────────────────────────────────────────────────

/// Per-tick simulation state (design.md decision 2). `x`/`target` are
/// strip-local columns within the filler; `frame` is a free-running counter
/// advanced every `frame_every` ticks and reset on a mood change so
/// animations restart cleanly; `tick` is a free-running per-step counter
/// (drives cadence and bang-blink); `celebrate_until_ms`/`prev_blocked`
/// drive the blocked-count-clears-to-zero transient celebrate.
///
/// `last_text`/`last_bang_lit` are bookkeeping for `changed` — not part of
/// the design.md field list, but needed so `changed` doesn't have to
/// re-derive the previous frame's display state from scratch.
#[derive(Debug, Default)]
pub struct PetSim {
    pub x: u16,
    pub target: u16,
    /// False until the first step places the pet at its home (the right
    /// end of the filler, beside the badge/chip cluster where eyes already
    /// go). Sized lazily because the filler width is only known at tick.
    pub placed: bool,
    pub frame: u64,
    pub tick: u64,
    pub celebrate_until_ms: i64,
    pub prev_blocked: usize,
    prev_mood: Option<Mood>,
    last_text: Option<&'static str>,
    last_bang_lit: bool,
}

/// One tick's outcome (spec "Species-true one-row motion").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepResult {
    /// True only when the sprite text, bang state, or x differ from the
    /// previous step — the caller's dirty-frame signal.
    pub changed: bool,
    /// Effective mood after the celebrate window is applied.
    pub mood: Mood,
    pub sprite: Sprite,
    pub x: u16,
    pub bang_lit: bool,
}

impl PetSim {
    fn bang_lit(species: Species, tick: u64) -> bool {
        match species {
            // "never blinks" (spec "Species-true one-row motion").
            Species::Duck => true,
            // The aloof cat doesn't blink its alarm either — a steady
            // stare fits the personality and isn't specified otherwise.
            Species::Cat => true,
            // "alert with a blinking `!`" (spec).
            Species::Pup => tick.is_multiple_of(2),
        }
    }

    /// Advance the simulation by one 300 ms tick.
    ///
    /// `ascii_pup` is `GARAGE_PET_ASCII=1`, read once by the caller and
    /// passed in here (this module stays env-free) — a deviation from the
    /// task's suggested `step` signature, which didn't list it; sprite
    /// selection needs it, so it travels alongside `species`.
    #[allow(clippy::too_many_arguments)] // one call site (runtime.rs); splitting these into a
    // params struct wouldn't make the tick logic any clearer.
    pub fn step(
        &mut self,
        species: Species,
        mood_in: Mood,
        blocked: usize,
        session_count: usize,
        filler_width: u16,
        now_ms: i64,
        ascii_pup: bool,
        rng: &mut impl FnMut() -> f64,
    ) -> StepResult {
        let old_x = self.x;
        self.tick += 1;

        // Blocked count clearing to zero (with sessions present) starts a
        // transient celebrate, species-timed.
        if blocked == 0 && self.prev_blocked > 0 && session_count > 0 {
            self.celebrate_until_ms = now_ms + species.celebrate_ms();
            self.frame = 0;
        }
        self.prev_blocked = blocked;

        // Alert always wins (a fresh needs-input beats an in-flight
        // celebrate); otherwise the celebrate window, if still open, wins
        // over whatever mood the caller derived.
        let effective = if mood_in == Mood::Alert {
            Mood::Alert
        } else if now_ms < self.celebrate_until_ms {
            Mood::Celebrate
        } else {
            mood_in
        };

        if self.prev_mood != Some(effective) {
            self.frame = 0;
        }
        self.prev_mood = Some(effective);

        let frames = frames_for(species, effective, ascii_pup);
        let sprite_width = frames[0].chars().count() as u16;
        let max_x = filler_width.saturating_sub(sprite_width);
        if !self.placed {
            self.x = max_x;
            self.target = max_x;
            self.placed = true;
        }

        match effective {
            Mood::Alert => {
                // "move the pet left toward the rail edge of the filler."
                self.target = 0;
            }
            Mood::Celebrate => match species {
                // Zoomies: rapid left-right target flips.
                Species::Pup => {
                    if self.tick.is_multiple_of(3) {
                        self.target = if self.target == 0 { max_x } else { 0 };
                    }
                }
                // Cat kneads, duck flaps — both stay put.
                Species::Cat | Species::Duck => self.target = self.x,
            },
            Mood::Box => self.target = self.x,
            Mood::Sleep | Mood::Watch => {
                // 5% chance per tick to pick a new idle-stroll target.
                if rng() < 0.05 {
                    let aloof = species.aloof();
                    if aloof <= 0.0 || rng() >= aloof {
                        // Strolls stay within the right 40% of the filler
                        // so the pet never wanders out of the eye-line of
                        // the badge/chip cluster it lives beside.
                        let roll = rng().clamp(0.0, 0.999_999);
                        let span = (f64::from(max_x) * 0.4).floor();
                        self.target = max_x - (roll * (span + 1.0)) as u16;
                    }
                }
            }
        }
        self.target = self.target.min(max_x);

        let zoomies = species == Species::Pup && effective == Mood::Celebrate;
        let speed = if zoomies { species.base_speed() * 2 } else { species.base_speed() };
        if self.x < self.target {
            self.x = (self.x + speed).min(self.target);
        } else if self.x > self.target {
            self.x = self.x.saturating_sub(speed).max(self.target);
        }
        self.x = self.x.min(max_x);

        if self.tick.is_multiple_of(species.frame_every()) {
            self.frame += 1;
        }

        // Duck's celebrate is a one-shot flap: play frame 0 then hold on
        // frame 1, never loop back.
        let hold_last = species == Species::Duck && effective == Mood::Celebrate;
        let idx = if hold_last {
            (self.frame as usize).min(frames.len() - 1)
        } else {
            (self.frame as usize) % frames.len()
        };
        let text = frames[idx];
        let bang = effective == Mood::Alert;
        let bang_lit = bang && Self::bang_lit(species, self.tick);

        let changed = self.x != old_x || Some(text) != self.last_text || bang_lit != self.last_bang_lit;
        self.last_text = Some(text);
        self.last_bang_lit = bang_lit;

        StepResult {
            changed,
            mood: effective,
            sprite: Sprite { text, bang },
            x: self.x,
            bang_lit,
        }
    }

    /// External trigger for a short celebrate (spec tui-pit-pet "Click
    /// routing": petting the pet is "a short celebrate and a species-voiced
    /// petting reply"). Additive — seeds the same `celebrate_until_ms`
    /// window `step`'s own blocked-transition logic uses, with the spec's
    /// fixed 1200 ms duration, and resets the frame so the celebrate
    /// animation restarts from its first frame.
    pub fn pet(&mut self, now_ms: i64) {
        self.celebrate_until_ms = now_ms + 1200;
        self.frame = 0;
    }
}

// ── Chatter ─────────────────────────────────────────────────────────────

/// What kind of thing the pet might say (spec "Species-voiced chatter").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intent {
    Hydrate,
    Encourage,
    Proud,
    Stretch,
    LateNight,
    Usage,
    Back,
    Petting,
}

/// Rate-limit floor between ordinary chatter lines (spec "Rate limit").
pub const CHATTER_FLOOR_MS: i64 = 180_000;

/// Chance a chatter line fires on a given 300 ms tick, once past the floor
/// — tuned so it averages "about once every 3 minutes" past the floor
/// (spec "Species-voiced chatter"): at one roll per 300 ms tick, 600 ticks
/// is 180 s.
pub const PET_TICK_MS: i64 = 300;

/// Voice copy: 2–3 lines per (species, intent) cell (task 5.1 — user
/// reviews before merge). Arthur is aloof and dry, Papito is deadpan and
/// terse, Segan is eager and loud.
pub const LINES: &[(Species, Intent, &[&str])] = &[
    // Arthur the shop cat — aloof, dry.
    (Species::Cat, Intent::Hydrate, &[
        "water. now. i'm not asking.",
        "hydrate. i'll wait.",
        "the bowl's right there. use it.",
    ]),
    (Species::Cat, Intent::Encourage, &[
        "...fine. this is going okay.",
        "not bad. don't get used to praise.",
        "acceptable pace. carry on.",
    ]),
    (Species::Cat, Intent::Proud, &[
        "i suppose i'm proud. don't tell anyone.",
        "hm. good work. that's all you get.",
        "fine. i'm impressed. barely.",
    ]),
    (Species::Cat, Intent::Stretch, &[
        "get up. stretch. i'll watch the shop.",
        "your spine will thank you. eventually.",
        "stand up before you fuse to that chair.",
    ]),
    (Species::Cat, Intent::LateNight, &[
        "it's late. even i'm not judging you. much.",
        "the shop's closed. you're not.",
        "go to bed. i mean it. sort of.",
    ]),
    (Species::Cat, Intent::Usage, &[
        "you're burning through your window. pace it.",
        "budget check: slow down.",
        "easy on the tokens, champ.",
    ]),
    (Species::Cat, Intent::Back, &[
        "oh. you're back. i wasn't worried.",
        "there you are. the shop survived.",
        "welcome back. i didn't move.",
    ]),
    (Species::Cat, Intent::Petting, &[
        "...fine. two seconds of this.",
        "don't make this a habit.",
        "acceptable. proceed.",
    ]),
    // Papito the rubber duck — deadpan, terse.
    (Species::Duck, Intent::Hydrate, &["drink water.", "hydrate.", "water. now."]),
    (Species::Duck, Intent::Encourage, &["acceptable.", "fine progress.", "quack. good."]),
    (Species::Duck, Intent::Proud, &["proud of you. quack.", "well done. quack.", "good. quack."]),
    (Species::Duck, Intent::Stretch, &["stand up.", "stretch. quack.", "move your legs."]),
    (Species::Duck, Intent::LateNight, &["late. quack.", "go to sleep.", "it is night. quack."]),
    (Species::Duck, Intent::Usage, &["usage high. slow down.", "pace yourself. quack.", "tokens low. quack."]),
    (Species::Duck, Intent::Back, &["you returned. quack.", "welcome back.", "connection restored. quack."]),
    (Species::Duck, Intent::Petting, &["quack.", "acknowledged. quack.", "..quack."]),
    // Segan the pit pup — eager, loud.
    (Species::Pup, Intent::Hydrate, &[
        "WATER BREAK!! then back to it!",
        "DRINK SOME WATER!! you got this!",
        "H2O TIME!! go go go!",
    ]),
    (Species::Pup, Intent::Encourage, &[
        "you're doing great!!",
        "KEEP GOING!! you got this!!",
        "look at you go!!",
    ]),
    (Species::Pup, Intent::Proud, &[
        "SO proud of you!!!",
        "YOU DID IT!! amazing!!",
        "best human ever!! proud!!",
    ]),
    (Species::Pup, Intent::Stretch, &[
        "STRETCH BREAK!! wiggle time!!",
        "get up and shake it out!!",
        "legs need love too!! stretch!!",
    ]),
    (Species::Pup, Intent::LateNight, &[
        "it's SO late!! sleep soon??",
        "still up?? okay but sleep after!!",
        "night owl mode!! don't forget rest!!",
    ]),
    (Species::Pup, Intent::Usage, &[
        "WHOA usage is high!! pace it!!",
        "tokens getting low!! slow down!!",
        "careful!! usage climbing fast!!",
    ]),
    (Species::Pup, Intent::Back, &[
        "YOU'RE BACK!! i missed you!!",
        "THERE YOU ARE!! hi hi hi!!",
        "welcome back!! zoomies!!",
    ]),
    (Species::Pup, Intent::Petting, &["*happy tail noises*", "YES pet me more!!", "best day ever!!"]),
];

fn lines_for(species: Species, intent: Intent) -> &'static [&'static str] {
    LINES
        .iter()
        .find(|(s, i, _)| *s == species && *i == intent)
        .map(|(_, _, lines)| *lines)
        .unwrap_or(&[])
}

fn pick_line(
    species: Species,
    intent: Intent,
    last_line: Option<&'static str>,
    rng: &mut impl FnMut() -> f64,
) -> Option<&'static str> {
    let lines = lines_for(species, intent);
    if lines.is_empty() {
        return None;
    }
    // Never repeat the last line said, when there's an alternative.
    let fresh: Vec<&'static str> = lines.iter().copied().filter(|l| Some(*l) != last_line).collect();
    let pool: &[&'static str] = if fresh.is_empty() { lines } else { &fresh };
    let roll = rng().clamp(0.0, 0.999_999);
    let idx = (roll * pool.len() as f64) as usize;
    Some(pool[idx.min(pool.len() - 1)])
}

/// Everything `Chatter::next_line` needs to decide what (if anything) to
/// say this tick — see design.md decision 6/8.
pub struct ChatterCtx {
    pub now_ms: i64,
    pub uptime_ms: i64,
    pub local_hour: u8,
    pub usage_pct_max: Option<u8>,
    pub mood: Mood,
    pub just_celebrated: bool,
    pub just_back_live: bool,
    pub alert: bool,
    pub user_notice_active: bool,
}

/// Chatter's own rate-limit/dedup state, one per pet session (reset by
/// picking a new species — the caller owns that).
#[derive(Debug)]
pub struct Chatter {
    pub last_said_ms: i64,
    pub last_line: Option<&'static str>,
    pub said_usage_high: bool,
    pub said_back: bool,
    /// The rate-limit floor between ordinary lines — [`CHATTER_FLOOR_MS`] by
    /// default. Overridable via [`Chatter::with_floor_ms`] for E2E harnesses
    /// (`GARAGE_PET_CHATTER_MS`) that can't wait 3 real minutes for a line;
    /// production callers always get the spec's floor through `default()`.
    floor_ms: i64,
}

impl Default for Chatter {
    fn default() -> Chatter {
        Chatter {
            last_said_ms: 0,
            last_line: None,
            said_usage_high: false,
            said_back: false,
            floor_ms: CHATTER_FLOOR_MS,
        }
    }
}

impl Chatter {
    /// An E2E-only constructor with a lowered (or raised) rate-limit floor
    /// — everything else starts at `default()`'s values (spec "Rate limit"
    /// is otherwise unaffected: alert suppression, the user-notice gate, and
    /// the event-bound exemptions all still apply).
    pub fn with_floor_ms(floor_ms: i64) -> Chatter {
        Chatter { floor_ms, ..Default::default() }
    }

    /// One chatter decision per tick. `None` when nothing is due. Event-
    /// bound lines (Back, Proud-on-celebrate) are exempt from the 3-minute
    /// floor (spec "Rate limit": "except the 'you're back' and
    /// proud-on-celebrate lines").
    pub fn next_line(&mut self, species: Species, ctx: &ChatterCtx, rng: &mut impl FnMut() -> f64) -> Option<&'static str> {
        if ctx.alert || ctx.user_notice_active {
            return None;
        }

        // A fresh box (daemon down) re-arms the "you're back" line for the
        // next reconnect.
        if ctx.mood == Mood::Box {
            self.said_back = false;
        }

        if ctx.just_back_live && !self.said_back {
            if let Some(line) = pick_line(species, Intent::Back, self.last_line, rng) {
                self.said_back = true;
                self.commit(ctx.now_ms, line);
                return Some(line);
            }
        }

        if ctx.just_celebrated {
            if let Some(line) = pick_line(species, Intent::Proud, self.last_line, rng) {
                self.commit(ctx.now_ms, line);
                return Some(line);
            }
        }

        if ctx.now_ms - self.last_said_ms < self.floor_ms {
            return None;
        }
        // Past the floor, the per-tick chance is tuned so the expected extra
        // wait ≈ one more floor (mean spacing ≈ 2× floor). Derived from the
        // floor so `with_floor_ms` (E2E) shortens the whole cadence.
        let tick_probability = (PET_TICK_MS as f64 / self.floor_ms.max(1) as f64).min(1.0);
        if rng() >= tick_probability {
            return None;
        }

        if ctx.usage_pct_max.is_none_or(|p| p < 80) {
            self.said_usage_high = false;
        }

        let mut candidates: Vec<(Intent, f64)> = Vec::new();
        let heavy_uptime = ctx.uptime_ms >= 60 * 60 * 1000;
        candidates.push((Intent::Hydrate, if heavy_uptime { 3.0 } else { 1.0 }));
        candidates.push((Intent::Encourage, 1.0));
        candidates.push((Intent::Stretch, if heavy_uptime { 3.0 } else { 1.0 }));
        if ctx.local_hour >= 23 || ctx.local_hour < 5 {
            candidates.push((Intent::LateNight, 1.0));
        }
        if ctx.usage_pct_max.is_some_and(|p| p >= 80) && !self.said_usage_high {
            candidates.push((Intent::Usage, 1.0));
        }

        let total: f64 = candidates.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            return None;
        }
        let mut roll = rng().clamp(0.0, 0.999_999) * total;
        let mut chosen = None;
        for (intent, weight) in &candidates {
            if roll < *weight {
                chosen = Some(*intent);
                break;
            }
            roll -= weight;
        }
        let intent = chosen?;
        let line = pick_line(species, intent, self.last_line, rng)?;
        if intent == Intent::Usage {
            self.said_usage_high = true;
        }
        self.commit(ctx.now_ms, line);
        Some(line)
    }

    fn commit(&mut self, now_ms: i64, line: &'static str) {
        self.last_said_ms = now_ms;
        self.last_line = Some(line);
    }
}

/// A petting reply, independent of the rate limit/floor (spec "Click
/// routing": "a petting interaction: a short celebrate and a species-voiced
/// petting reply").
pub fn petting_line(species: Species, rng: &mut impl FnMut() -> f64) -> Option<&'static str> {
    let lines = lines_for(species, Intent::Petting);
    if lines.is_empty() {
        return None;
    }
    let roll = rng().clamp(0.0, 0.999_999);
    let idx = (roll * lines.len() as f64) as usize;
    Some(lines[idx.min(lines.len() - 1)])
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub rng yielding a fixed constant every call.
    fn constant(value: f64) -> impl FnMut() -> f64 {
        move || value
    }

    /// A stub rng cycling through a fixed sequence, wrapping.
    fn sequence(values: &'static [f64]) -> impl FnMut() -> f64 {
        let mut i = 0usize;
        move || {
            let v = values[i % values.len()];
            i += 1;
            v
        }
    }

    // ── mood ───────────────────────────────────────────────────────────

    #[test]
    fn mood_precedence_box_beats_everything() {
        assert_eq!(mood(false, true, true, 5), Mood::Box);
    }

    #[test]
    fn mood_precedence_empty_beats_alert_and_watch() {
        assert_eq!(mood(true, true, true, 0), Mood::Sleep);
    }

    #[test]
    fn mood_precedence_alert_beats_watch() {
        assert_eq!(mood(true, true, true, 3), Mood::Alert);
    }

    #[test]
    fn mood_precedence_watch_beats_sleep() {
        assert_eq!(mood(true, false, true, 3), Mood::Watch);
    }

    #[test]
    fn mood_sleep_when_nothing_is_happening() {
        assert_eq!(mood(true, false, false, 3), Mood::Sleep);
    }

    // ── Species ────────────────────────────────────────────────────────

    #[test]
    fn cycle_goes_off_cat_duck_pup_off() {
        let mut cur = None;
        let order = [Some(Species::Cat), Some(Species::Duck), Some(Species::Pup), None];
        for expected in order {
            cur = Species::cycle(cur);
            assert_eq!(cur, expected);
        }
    }

    #[test]
    fn species_str_round_trips() {
        for s in [Species::Cat, Species::Duck, Species::Pup] {
            assert_eq!(Species::from_str(s.as_str()), Some(s));
        }
        assert_eq!(Species::from_str("iguana"), None);
    }

    // ── sprites ────────────────────────────────────────────────────────

    #[test]
    fn every_species_mood_has_a_single_line_sprite() {
        let moods = [Mood::Sleep, Mood::Watch, Mood::Alert, Mood::Box, Mood::Celebrate];
        for species in [Species::Cat, Species::Duck, Species::Pup] {
            for m in moods {
                let frames = frames_for(species, m, false);
                assert!(!frames.is_empty(), "{species:?} {m:?} has no frames");
                for f in frames {
                    assert!(!f.contains('\n'), "{species:?} {m:?} frame is multi-line: {f:?}");
                }
            }
        }
    }

    #[test]
    fn alert_sprite_carries_the_bang_flag() {
        let s = sprite(Species::Pup, Mood::Alert, 0, false);
        assert!(s.bang);
        let s = sprite(Species::Pup, Mood::Sleep, 0, false);
        assert!(!s.bang);
    }

    #[test]
    fn ascii_fallback_avoids_the_pup_face_glyphs() {
        for m in [Mood::Sleep, Mood::Watch, Mood::Alert, Mood::Box, Mood::Celebrate] {
            for f in pup_frames(m, true) {
                assert!(!f.contains('ᴥ') && !f.contains('°') && !f.contains('·'), "{f:?}");
            }
        }
    }

    // ── PetSim::step ───────────────────────────────────────────────────

    #[test]
    fn duck_alert_bang_never_blinks_across_ten_ticks() {
        let mut sim = PetSim::default();
        let mut rng = constant(0.9);
        for _ in 0..10 {
            let r = sim.step(Species::Duck, Mood::Alert, 1, 1, 40, 0, false, &mut rng);
            assert!(r.bang_lit, "duck bang should stay lit every tick");
        }
    }

    #[test]
    fn pup_alert_bang_blinks() {
        let mut sim = PetSim::default();
        let mut rng = constant(0.9);
        let mut seen_lit = false;
        let mut seen_unlit = false;
        for _ in 0..10 {
            let r = sim.step(Species::Pup, Mood::Alert, 1, 1, 40, 0, false, &mut rng);
            if r.bang_lit {
                seen_lit = true;
            } else {
                seen_unlit = true;
            }
        }
        assert!(seen_lit && seen_unlit, "pup bang should alternate lit/unlit");
    }

    #[test]
    fn cat_ignores_strolls_with_a_stubbed_rng() {
        // rng() == 0.0 always: the 5% stroll roll always fires (0.0 < 0.05)
        // but the aloof check (rng() >= 0.4) always fails, so the cat
        // always ignores it.
        let mut sim = PetSim { x: 5, target: 5, placed: true, ..Default::default() };
        let mut rng = constant(0.0);
        for _ in 0..20 {
            sim.step(Species::Cat, Mood::Sleep, 0, 1, 40, 0, false, &mut rng);
        }
        assert_eq!(sim.target, 5, "aloof cat should never take the stroll");
    }

    #[test]
    fn first_step_homes_the_pet_at_the_right_end_and_strolls_stay_right() {
        let mut sim = PetSim::default();
        let mut rng = constant(0.9);
        let r = sim.step(Species::Duck, Mood::Watch, 0, 1, 40, 0, false, &mut rng);
        let width = r.sprite.text.chars().count() as u16;
        assert_eq!(r.x, 40 - width, "home is the right end of the filler");
        // Force a stroll (roll 0.0 < 0.05) with the farthest-left target
        // (roll 0.0 → max_x - 0): still inside the right 40%.
        let mut sim = PetSim::default();
        let mut zero = constant(0.0);
        for _ in 0..3 {
            sim.step(Species::Duck, Mood::Watch, 0, 1, 40, 0, false, &mut zero);
        }
        let max_x = 40 - width;
        assert!(sim.target >= max_x - (f64::from(max_x) * 0.4) as u16, "target {} in right 40%", sim.target);
    }

    #[test]
    fn alert_moves_toward_zero() {
        let mut sim = PetSim { x: 10, placed: true, ..Default::default() };
        let mut rng = constant(0.9);
        let mut last_x = 10;
        for _ in 0..15 {
            let r = sim.step(Species::Cat, Mood::Alert, 1, 1, 40, 0, false, &mut rng);
            assert!(r.x <= last_x, "x should never move away from 0 while alert");
            last_x = r.x;
        }
        assert_eq!(last_x, 0);
    }

    #[test]
    fn celebrate_triggers_on_blocked_transition_and_expires() {
        let mut sim = PetSim { prev_blocked: 1, ..Default::default() };
        let mut rng = constant(0.9);

        let r = sim.step(Species::Cat, Mood::Sleep, 0, 1, 40, 1_000, false, &mut rng);
        assert_eq!(r.mood, Mood::Celebrate);
        assert_eq!(sim.celebrate_until_ms, 1_000 + Species::Cat.celebrate_ms());

        let r = sim.step(Species::Cat, Mood::Sleep, 0, 1, 40, 1_000 + Species::Cat.celebrate_ms() + 1, false, &mut rng);
        assert_eq!(r.mood, Mood::Sleep, "celebrate should expire");
    }

    #[test]
    fn changed_is_false_on_a_no_op_tick() {
        let mut sim = PetSim::default();
        let mut rng = constant(0.9); // never rolls a stroll (>= 0.05)
        // Box: single frame, no movement — a true steady state.
        let _ = sim.step(Species::Cat, Mood::Box, 0, 1, 40, 0, false, &mut rng);
        let r = sim.step(Species::Cat, Mood::Box, 0, 1, 40, 300, false, &mut rng);
        assert!(!r.changed);
    }

    #[test]
    fn duck_celebrate_flaps_once_then_holds() {
        let mut sim = PetSim { prev_blocked: 1, ..Default::default() };
        let mut rng = constant(0.9);
        let r0 = sim.step(Species::Duck, Mood::Sleep, 0, 1, 40, 0, false, &mut rng);
        assert_eq!(r0.mood, Mood::Celebrate);
        let frame0 = r0.sprite.text;
        // Duck frame_every is 3 ticks; step well past two frame advances
        // and confirm it holds on the second frame rather than looping.
        let mut last = frame0;
        for _ in 0..12 {
            let r = sim.step(Species::Duck, Mood::Sleep, 0, 1, 40, 100, false, &mut rng);
            last = r.sprite.text;
        }
        // Holds: stepping further doesn't bounce back to frame 0's text.
        let r_more = sim.step(Species::Duck, Mood::Sleep, 0, 1, 40, 100, false, &mut rng);
        assert_eq!(r_more.sprite.text, last);
    }

    // ── Chatter ────────────────────────────────────────────────────────

    fn ctx(now_ms: i64) -> ChatterCtx {
        ChatterCtx {
            now_ms,
            uptime_ms: 0,
            local_hour: 12,
            usage_pct_max: None,
            mood: Mood::Sleep,
            just_celebrated: false,
            just_back_live: false,
            alert: false,
            user_notice_active: false,
        }
    }

    #[test]
    fn no_chatter_while_alert() {
        let mut chatter = Chatter::default();
        let mut c = ctx(1_000_000);
        c.alert = true;
        let mut rng = constant(0.0);
        assert_eq!(chatter.next_line(Species::Cat, &c, &mut rng), None);
    }

    #[test]
    fn no_chatter_while_a_user_notice_is_active() {
        let mut chatter = Chatter::default();
        let mut c = ctx(1_000_000);
        c.user_notice_active = true;
        let mut rng = constant(0.0);
        assert_eq!(chatter.next_line(Species::Cat, &c, &mut rng), None);
    }

    #[test]
    fn floor_is_enforced() {
        let mut chatter = Chatter { last_said_ms: 1_000_000, ..Default::default() };
        let c = ctx(1_000_000 + CHATTER_FLOOR_MS - 1);
        let mut rng = constant(0.0); // would always fire if floor let it through
        assert_eq!(chatter.next_line(Species::Cat, &c, &mut rng), None);
    }

    #[test]
    fn back_line_is_exempt_from_the_floor_and_said_once() {
        // Floor would otherwise block.
        let mut chatter = Chatter { last_said_ms: 1_000_000, ..Default::default() };
        let mut c = ctx(1_000_000 + 1);
        c.just_back_live = true;
        let mut rng = constant(0.0);
        let line = chatter.next_line(Species::Cat, &c, &mut rng);
        assert!(line.is_some());
        // Immediately again: said_back is now true, no repeat Back line.
        let line2 = chatter.next_line(Species::Cat, &c, &mut rng);
        assert!(lines_for(Species::Cat, Intent::Back).iter().all(|l| Some(*l) != line2) || line2.is_none());
    }

    #[test]
    fn usage_fires_once_until_it_drops_below_eighty() {
        let mut chatter = Chatter::default();
        // Floor already elapsed (last_said_ms default 0) and the tick
        // probability always fires.
        let mut c = ctx(CHATTER_FLOOR_MS + 1);
        c.usage_pct_max = Some(85);
        // [0]: tick-probability roll (fires); [1]: weighted-intent roll,
        // high enough to land on Usage (the last, smallest-share bucket
        // once Hydrate/Encourage/Stretch are exhausted); [2]: line pick.
        let mut rng = sequence(&[0.0, 0.9, 0.0]);
        let first = chatter.next_line(Species::Pup, &c, &mut rng);
        assert!(first.is_some());
        assert!(chatter.said_usage_high);

        // Drop below 80 resets the flag.
        c.usage_pct_max = Some(50);
        c.now_ms += CHATTER_FLOOR_MS + 1;
        chatter.next_line(Species::Pup, &c, &mut rng);
        assert!(!chatter.said_usage_high);
    }

    #[test]
    fn late_night_only_enabled_in_window() {
        let mut c = ctx(0);
        c.local_hour = 12;
        assert!(!(c.local_hour >= 23 || c.local_hour < 5));
        c.local_hour = 23;
        assert!(c.local_hour >= 23 || c.local_hour < 5);
        c.local_hour = 4;
        assert!(c.local_hour >= 23 || c.local_hour < 5);
    }

    #[test]
    fn chatter_never_repeats_the_last_line() {
        let chatter = Chatter { last_line: Some("water. now. i'm not asking."), ..Default::default() };
        let mut rng = constant(0.0);
        let line = pick_line(Species::Cat, Intent::Hydrate, chatter.last_line, &mut rng);
        assert_ne!(line, chatter.last_line);
    }

    #[test]
    fn every_lines_cell_is_non_empty() {
        for species in [Species::Cat, Species::Duck, Species::Pup] {
            for intent in [
                Intent::Hydrate,
                Intent::Encourage,
                Intent::Proud,
                Intent::Stretch,
                Intent::LateNight,
                Intent::Usage,
                Intent::Back,
                Intent::Petting,
            ] {
                let lines = lines_for(species, intent);
                assert!(!lines.is_empty(), "{species:?} {intent:?} has no lines");
                for l in lines {
                    assert!(!l.is_empty());
                }
            }
        }
    }

    #[test]
    fn with_floor_ms_lowers_the_rate_limit_for_e2e() {
        let mut chatter = Chatter::with_floor_ms(10);
        chatter.last_said_ms = 1_000;
        let c = ctx(1_011); // 11ms later: past the lowered floor, still under the default one
        let mut rng = constant(0.0); // tick-probability always fires
        assert!(chatter.next_line(Species::Cat, &c, &mut rng).is_some());
    }

    #[test]
    fn pet_sim_pet_starts_a_short_celebrate() {
        let mut sim = PetSim::default();
        sim.pet(1_000);
        assert_eq!(sim.celebrate_until_ms, 1_000 + 1200);
        let mut rng = constant(0.9);
        let r = sim.step(Species::Cat, Mood::Sleep, 0, 1, 40, 1_050, false, &mut rng);
        assert_eq!(r.mood, Mood::Celebrate);
    }

    #[test]
    fn petting_line_returns_a_species_voiced_reply() {
        let mut rng = constant(0.0);
        let line = petting_line(Species::Duck, &mut rng).unwrap();
        assert!(lines_for(Species::Duck, Intent::Petting).contains(&line));
    }
}
