//! The pure part: stored state, one simulated window of frames, and the wire
//! body that draws it. No I/O, no clock reads; every time is passed in.

use garage_plugin_sdk::{Segment, Style, WidgetItem, WidgetPut};
use serde::{Deserialize, Serialize};

use crate::engine::{Chatter, ChatterCtx, LINES, Mood, PetSim, Species, mood, petting_line};

/// Milliseconds per simulated tick and per frame: what the wall's own pet
/// ticked at.
pub const TICK_MS: i64 = 300;
/// Frames per set (the widget limit). 24 frames last 7.2 s, longer than the
/// ~6 s timer that renews the set, so the pet never replays the start of one.
pub const FRAMES: usize = 24;
/// A pushed set lives this long unless renewed: a pet whose plugin died (or
/// whose daemon forgot it) disappears instead of freezing on its last frame.
/// More than two timer periods, so one late run does not make it blink.
pub const PUSH_TTL_SECS: u32 = 15;
/// No push within this long of the last (a burst of session events makes one).
pub const DEBOUNCE_MS: i64 = 300;
/// How long a chatter line stays beside the pet.
pub const CHATTER_CAPTION_MS: i64 = 4000;
/// How long a petting reply or an arrival line stays.
pub const PET_CAPTION_MS: i64 = 2500;
pub const ARRIVAL_CAPTION_MS: i64 = 3000;
/// The click action of the pet (this plugin's own).
pub const CLICK: &str = "pet";
/// What a click on an alerting pet does: go to the session that needs you.
pub const ALERT_CLICK: &str = "jump.blocked";

/// What one frame looks like.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameSpec {
    pub text: String,
    pub bang: bool,
    pub bang_lit: bool,
    /// Column within the strip's filler where the sprite starts.
    pub x: u16,
    /// Sleep and box are drawn dim.
    pub dim: bool,
    /// The pet is alerting: a click goes to the blocked session.
    #[serde(default)]
    pub alert: bool,
}

/// One simulated tick, kept so the next run can continue from where the pet
/// really is at that moment instead of from the end of the old window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snap {
    pub t: i64,
    pub sim: PetSim,
    pub rng: u64,
    pub mood: Mood,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Caption {
    pub text: String,
    pub until_ms: i64,
}

/// `Chatter` with its borrowed line made storable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatterSave {
    pub floor_ms: Option<i64>,
    pub last_said_ms: i64,
    pub last_line: Option<String>,
    pub said_usage_high: bool,
    pub said_back: bool,
}

impl ChatterSave {
    fn load(&self) -> Chatter {
        let mut c = self
            .floor_ms
            .map_or_else(Chatter::default, Chatter::with_floor_ms);
        c.last_said_ms = self.last_said_ms;
        c.said_usage_high = self.said_usage_high;
        c.said_back = self.said_back;
        c.last_line = self
            .last_line
            .as_deref()
            .and_then(|s| {
                LINES
                    .iter()
                    .flat_map(|(_, _, ls)| ls.iter())
                    .find(|l| **l == s)
            })
            .copied();
        c
    }

    fn store(c: &Chatter) -> ChatterSave {
        ChatterSave {
            floor_ms: Some(c.floor_ms()),
            last_said_ms: c.last_said_ms,
            last_line: c.last_line.map(str::to_owned),
            said_usage_high: c.said_usage_high,
            said_back: c.said_back,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    /// `None`: no pet (the default).
    pub species: Option<Species>,
    pub sim: PetSim,
    pub rng: u64,
    pub chatter: ChatterSave,
    /// The effective mood of the last simulated tick.
    pub mood: Option<Mood>,
    /// The last window, oldest tick first.
    pub trace: Vec<Snap>,
    /// The last window as pushed (rotated onto the clock), to redraw it
    /// without a caption.
    pub frames: Vec<FrameSpec>,
    pub caption: Option<Caption>,
    /// Bumped on every push; a delayed caption clear only applies to the push
    /// it was scheduled for.
    pub rev: u64,
    /// Whether a widget is currently pushed (so turning the pet off clears it
    /// exactly once).
    pub pushed: bool,
    /// Epoch ms of the daemon start this state last saw: the chatter's uptime.
    pub started_ms: i64,
    /// Epoch ms of the last push, for the debounce.
    #[serde(default)]
    pub last_push_ms: i64,
    /// `date +%H` result and the minute it was read.
    pub hour_cache: Option<(i64, u8)>,
    /// The old `pet` value of `wall.json` was looked at.
    pub migrated: bool,
}

impl Default for State {
    fn default() -> State {
        State {
            species: None,
            sim: PetSim::default(),
            rng: 0x9E37_79B9_7F4A_7C15,
            chatter: ChatterSave::default(),
            mood: None,
            trace: Vec::new(),
            frames: Vec::new(),
            caption: None,
            rev: 0,
            pushed: false,
            started_ms: 0,
            last_push_ms: 0,
            hour_cache: None,
            migrated: false,
        }
    }
}

/// A tiny xorshift64 PRNG in `[0, 1)`: the one source of randomness the sim
/// and the chatter share (not cryptographic; the pet only has to look
/// unpredictable).
pub fn xorshift(state: &mut u64) -> f64 {
    let mut x = if *state == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        *state
    };
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// What the sim needs to know about the outside world.
#[derive(Debug, Clone, PartialEq)]
pub struct Inputs {
    pub blocked: usize,
    pub any_working: bool,
    pub session_count: usize,
    /// The larger of the two usage windows, when known.
    pub usage_pct_max: Option<u8>,
    /// Filler columns the sim may use (strip filler minus its two gutters).
    pub filler_width: u16,
    pub local_hour: u8,
    pub ascii_pup: bool,
}

impl State {
    /// Set a caption, replacing any other.
    pub fn say(&mut self, text: impl Into<String>, now: i64, ttl_ms: i64) {
        self.caption = Some(Caption {
            text: text.into(),
            until_ms: now + ttl_ms,
        });
    }

    /// The caption still showing at `now`.
    pub fn caption_at(&self, now: i64) -> Option<&Caption> {
        self.caption.as_ref().filter(|c| c.until_ms > now)
    }

    /// `P`: off, then each species, then off. The sim starts over (a fresh
    /// pet begins at home, not wherever the last one wandered); the chatter
    /// limiter is kept so swapping species cannot dodge it.
    pub fn cycle(&mut self, now: i64) {
        self.species = Species::cycle(self.species);
        self.sim = PetSim::default();
        self.trace.clear();
        self.mood = None;
        self.caption = None;
        if let Some(s) = self.species {
            self.say(
                format!("{} is on the wall", s.label()),
                now,
                ARRIVAL_CAPTION_MS,
            );
        }
    }

    /// Petting: a short celebrate and a species-voiced reply. Returns `false`
    /// when there is no pet to pet.
    pub fn pet(&mut self, now: i64) -> bool {
        let Some(species) = self.species else {
            return false;
        };
        // Continue from the tick the pet is really at.
        self.rebase(now);
        self.sim.pet(now);
        // The sim now holds the petting; planning must continue from it, not
        // rebase over it from the old window.
        self.trace.clear();
        let mut rng_state = self.rng;
        let line = petting_line(species, &mut || xorshift(&mut rng_state));
        self.rng = rng_state;
        if let Some(line) = line {
            self.say(line, now, PET_CAPTION_MS);
        }
        true
    }

    /// Load the sim and rng of the last tick strictly before `now`'s tick, if
    /// the trace has one; this is what makes a run triggered at any moment
    /// continue from the pet's actual place.
    fn rebase(&mut self, now: i64) {
        let t0 = now.div_euclid(TICK_MS) * TICK_MS;
        if let Some(s) = self.trace.iter().rev().find(|s| s.t < t0) {
            self.sim = s.sim.clone();
            self.rng = s.rng;
            self.mood = Some(s.mood);
        }
    }

    /// Simulate the next window starting at the tick containing `now`, store
    /// it, and return the frames rotated onto the wall clock (frame `i` is
    /// the one the TUI shows when `floor(clock / TICK_MS) mod 16 == i`).
    /// `None` when there is no pet.
    pub fn plan(&mut self, inputs: &Inputs, now: i64) -> Option<Vec<FrameSpec>> {
        let species = self.species?;
        self.rebase(now);
        let t0 = now.div_euclid(TICK_MS) * TICK_MS;
        let mut chatter = self.chatter.load();
        let mut rng_state = self.rng;
        let mut prev_mood = self.mood;
        let mood_in = mood(
            true,
            inputs.blocked > 0,
            inputs.any_working,
            inputs.session_count,
        );
        let uptime_base = if self.started_ms > 0 {
            self.started_ms
        } else {
            now
        };
        let mut trace = Vec::with_capacity(FRAMES);
        let mut frames: Vec<Option<FrameSpec>> = vec![None; FRAMES];
        let mut new_line: Option<&'static str> = None;
        for k in 0..FRAMES {
            let t = t0 + k as i64 * TICK_MS;
            let r = self.sim.step(
                species,
                mood_in,
                inputs.blocked,
                inputs.session_count,
                inputs.filler_width,
                t,
                inputs.ascii_pup,
                &mut || xorshift(&mut rng_state),
            );
            let just_celebrated = prev_mood != Some(Mood::Celebrate) && r.mood == Mood::Celebrate;
            prev_mood = Some(r.mood);
            let ctx = ChatterCtx {
                now_ms: t,
                uptime_ms: t - uptime_base,
                local_hour: inputs.local_hour,
                usage_pct_max: inputs.usage_pct_max,
                mood: r.mood,
                just_celebrated,
                just_back_live: false,
                alert: r.mood == Mood::Alert,
                user_notice_active: self.caption_at(now).is_some() || new_line.is_some(),
            };
            if let Some(line) = chatter.next_line(species, &ctx, &mut || xorshift(&mut rng_state)) {
                new_line.get_or_insert(line);
            }
            trace.push(Snap {
                t,
                sim: self.sim.clone(),
                rng: rng_state,
                mood: r.mood,
            });
            let index = (t.div_euclid(TICK_MS)).rem_euclid(FRAMES as i64) as usize;
            frames[index] = Some(FrameSpec {
                text: r.sprite.text.to_owned(),
                bang: r.sprite.bang,
                bang_lit: r.bang_lit,
                x: r.x,
                dim: matches!(r.mood, Mood::Sleep | Mood::Box),
                alert: r.mood == Mood::Alert,
            });
        }
        self.rng = rng_state;
        self.mood = prev_mood;
        self.chatter = ChatterSave::store(&chatter);
        self.trace = trace;
        if let Some(line) = new_line {
            self.say(line, now, CHATTER_CAPTION_MS);
        }
        let frames: Vec<FrameSpec> = frames.into_iter().flatten().collect();
        self.frames = frames.clone();
        Some(frames)
    }
}

fn seg(text: &str, fg: &str, bold: bool, click: Option<&str>) -> Segment {
    Segment {
        text: text.to_owned(),
        style: Some(Style {
            fg: Some(fg.to_owned()),
            bold,
            ..Style::default()
        }),
        click: click.map(|action| garage_plugin_sdk::Click {
            action: action.to_owned(),
            args: None,
        }),
    }
}

fn segments(f: &FrameSpec) -> Vec<Segment> {
    // The alert frames carry the jump themselves; the TUI runs it on click
    // without asking this plugin, so it works even with the plugin not running.
    let click = if f.alert { ALERT_CLICK } else { CLICK };
    let mut out = vec![seg(
        &f.text,
        if f.dim { "dim" } else { "text" },
        false,
        Some(click),
    )];
    if f.bang {
        out.push(Segment {
            text: " ".into(),
            style: None,
            click: None,
        });
        out.push(if f.bang_lit {
            // Amber while a session needs input (the wall decides; the
            // token is neutral the rest of the time).
            seg("!", "needs-input", true, None)
        } else {
            Segment {
                text: " ".into(),
                style: None,
                click: None,
            }
        });
    }
    out
}

/// The widget body for a window: one animated filler item whose column
/// follows the frame, the caption beside it.
pub fn build_put(frames: &[FrameSpec], caption: Option<&str>) -> WidgetPut {
    let item = WidgetItem {
        priority: 100,
        placement: garage_plugin_sdk::Placement::Filler {
            x: frames.first().map_or(0, |f| f.x),
            caption: caption.map(str::to_owned),
        },
        frames: Some(frames.iter().map(segments).collect()),
        frame_ms: Some(TICK_MS as u32),
        frame_x: Some(frames.iter().map(|f| f.x).collect()),
        ..WidgetItem::default()
    };
    WidgetPut {
        scopes: [("*".to_owned(), vec![item])].into(),
        ttl_secs: Some(PUSH_TTL_SECS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(blocked: usize, working: bool, sessions: usize) -> Inputs {
        Inputs {
            blocked,
            any_working: working,
            session_count: sessions,
            usage_pct_max: None,
            filler_width: 60,
            local_hour: 12,
            ascii_pup: false,
        }
    }

    fn cat() -> State {
        State {
            species: Some(Species::Cat),
            // Never chatter unless a test asks for it.
            chatter: ChatterSave {
                floor_ms: Some(i64::MAX / 4),
                ..ChatterSave::default()
            },
            ..State::default()
        }
    }

    fn index(t: i64) -> usize {
        t.div_euclid(TICK_MS).rem_euclid(FRAMES as i64) as usize
    }

    const T: i64 = 1_800_000_000_000 / TICK_MS * TICK_MS;

    #[test]
    fn a_window_lies_on_the_wall_clock_one_tick_per_frame() {
        let mut st = cat();
        let frames = st.plan(&inputs(1, false, 1), T + 100).unwrap();
        assert_eq!(frames.len(), FRAMES, "every index of the set is filled");
        // The tick simulated for time t is the frame the TUI shows at time t.
        for snap in &st.trace {
            let f = &frames[index(snap.t)];
            assert_eq!(f.x, snap.sim.x, "tick at {}", snap.t - T);
        }
        // The first tick is the one containing `now`, the rest follow in order.
        let times: Vec<i64> = st.trace.iter().map(|s| s.t - T).collect();
        assert_eq!(
            times,
            (0..FRAMES as i64).map(|k| k * TICK_MS).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_alerting_cat_walks_to_the_rail_edge_with_a_lit_bang() {
        let mut st = cat();
        let frames = st.plan(&inputs(2, true, 3), T).unwrap();
        let first = &frames[index(T)];
        let last = &frames[index(T + 15 * TICK_MS)];
        assert_eq!(
            (first.text.as_str(), first.bang, first.bang_lit),
            ("=O_O=", true, true)
        );
        assert!(last.x < first.x, "walks left: {} -> {}", first.x, last.x);
        assert!(!first.dim);
    }

    #[test]
    fn a_run_in_mid_window_continues_from_where_the_pet_is() {
        let mut st = cat();
        let a = st.plan(&inputs(1, false, 1), T).unwrap();
        // 5 ticks later a session event triggers a run with the same world.
        let later = T + 5 * TICK_MS + 80;
        let b = st.plan(&inputs(1, false, 1), later).unwrap();
        for k in 5..16 {
            let t = T + k * TICK_MS;
            assert_eq!(
                a[index(t)],
                b[index(t)],
                "tick {k} is the same pet in both windows"
            );
        }
    }

    #[test]
    fn the_blocked_count_falling_to_zero_celebrates_and_the_mood_follows_the_sessions() {
        let mut st = cat();
        st.plan(&inputs(1, false, 1), T).unwrap();
        let after = T + 4 * TICK_MS;
        let frames = st.plan(&inputs(0, true, 1), after).unwrap();
        assert_eq!(frames[index(after)].text, "=^.^=", "celebrate right away");
        // Long after the celebrate window, a working session is Watch.
        let much_later = after + 20_000;
        let frames = st.plan(&inputs(0, true, 1), much_later).unwrap();
        assert!(frames[index(much_later)].text.starts_with("=o.o="));
        // No sessions at all: asleep and dim.
        let frames = st.plan(&inputs(0, false, 0), much_later + 20_000).unwrap();
        let f = &frames[index(much_later + 20_000)];
        assert!(f.text.starts_with("=-.-="), "{}", f.text);
        assert!(f.dim);
    }

    #[test]
    fn no_pet_plans_nothing() {
        let mut st = State::default();
        assert!(st.plan(&inputs(0, false, 0), T).is_none());
    }

    #[test]
    fn p_walks_the_roster_resets_the_sim_and_the_arrival_is_a_caption() {
        let mut st = State::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            st.cycle(T);
            seen.push(st.species);
        }
        assert_eq!(
            seen,
            [
                Some(Species::Cat),
                Some(Species::Duck),
                Some(Species::Pup),
                None
            ]
        );
        st.cycle(T);
        assert_eq!(
            st.caption_at(T).unwrap().text,
            "Arthur the shop cat is on the wall"
        );
        assert!(
            st.caption_at(T + ARRIVAL_CAPTION_MS).is_none(),
            "gone after its time"
        );
        st.cycle(T);
        assert_eq!(
            st.caption_at(T).unwrap().text,
            "Papito the rubber duck is on the wall",
            "a new pick replaces the old line"
        );
        assert!(!st.sim.placed, "a fresh pet starts over at home");
    }

    #[test]
    fn petting_celebrates_for_a_moment_and_the_pet_answers() {
        let mut st = cat();
        st.plan(&inputs(0, false, 1), T).unwrap();
        let click = T + 3 * TICK_MS + 50;
        assert!(st.pet(click));
        assert!(st.caption_at(click).is_some(), "a species-voiced reply");
        let frames = st.plan(&inputs(0, false, 1), click).unwrap();
        assert!(
            frames[index(click)].text.starts_with("=^.^="),
            "celebrating"
        );
        // 1.2 s later the celebrate is over and the pet sleeps again.
        let over = click + 1300 + TICK_MS;
        assert!(
            frames[index(over)].text.starts_with("=-.-="),
            "{}",
            frames[index(over)].text
        );
        assert!(
            !State::default().pet(T),
            "nothing to pet when the pet is off"
        );
    }

    #[test]
    fn a_chatter_line_becomes_the_caption_once_and_is_one_of_the_pets_lines() {
        let mut st = cat();
        st.chatter.floor_ms = Some(1); // fires on the first roll
        st.plan(&inputs(0, true, 1), T).unwrap();
        let c = st.caption_at(T).expect("a line fired").clone();
        assert!(
            LINES
                .iter()
                .any(|(sp, _, ls)| *sp == Species::Cat && ls.contains(&c.text.as_str())),
            "{c:?}"
        );
        assert_eq!(c.until_ms, T + CHATTER_CAPTION_MS);
        // While it shows, nothing replaces it.
        st.plan(&inputs(0, true, 1), T + 1000).unwrap();
        assert_eq!(st.caption.as_ref(), Some(&c));
    }

    #[test]
    fn the_state_survives_the_file_and_the_pet_does_not_jump() {
        let mut st = cat();
        st.chatter.floor_ms = Some(1);
        st.plan(&inputs(0, true, 1), T).unwrap();
        let back: State = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(back.sim.x, st.sim.x);
        assert_eq!(back.trace.len(), FRAMES);
        assert_eq!(back.chatter.last_line, st.chatter.last_line);
        assert!(
            back.chatter.load().last_line.is_some(),
            "the borrowed line is found again"
        );
        let mut a = st;
        let mut b = back;
        let t = T + 6 * TICK_MS;
        assert_eq!(
            a.plan(&inputs(0, true, 1), t).unwrap(),
            b.plan(&inputs(0, true, 1), t).unwrap(),
            "the same plan before and after the round trip"
        );
    }

    fn golden_check(name: &str, put: &WidgetPut) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("golden")
            .join(name);
        let got = serde_json::to_string(put).unwrap() + "\n";
        if std::env::var_os("GARAGE_UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &got).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), got, "{name}");
    }

    /// The two strip frames the wall's snapshot renders (an alerting pup with
    /// speech and a sleeping cat), as this plugin writes them.
    #[test]
    fn the_wire_bodies_are_the_golden_files_the_wall_renders() {
        let alert = FrameSpec {
            text: "(O.O)".into(),
            bang: true,
            bang_lit: true,
            x: 30,
            dim: false,
            alert: true,
        };
        let asleep = FrameSpec {
            text: "=-.-=".into(),
            bang: false,
            bang_lit: false,
            x: 4,
            dim: true,
            alert: false,
        };
        golden_check(
            "alert-speech.json",
            &build_put(&[alert], Some("someone needs you")),
        );
        golden_check("asleep.json", &build_put(&[asleep], None));
    }

    /// A whole 24-frame window of an alerting cat, as the wall renders it: the
    /// frames cycle (the sprite changes and the cat walks), and the body stays
    /// far below the 16 KiB limit with room for the caption.
    #[test]
    fn a_full_window_is_a_golden_that_cycles_and_stays_small() {
        let mut st = cat();
        let frames = st.plan(&inputs(1, true, 2), T).unwrap();
        assert_eq!(frames.len(), FRAMES);
        let put = build_put(&frames, Some("someone needs you"));
        golden_check("cycle-24.json", &put);
        let body = serde_json::to_vec(&put).unwrap();
        assert!(
            body.len() < 12 * 1024,
            "a window is {} bytes; the limit is 16 KiB",
            body.len()
        );
        // It cycles: more than one column, and every frame is clickable as
        // the alert it is.
        let xs: std::collections::BTreeSet<u16> = frames.iter().map(|f| f.x).collect();
        assert!(xs.len() > 3, "the cat walks: {xs:?}");
        assert!(frames.iter().all(|f| f.alert && f.bang));
        let item = &put.scopes["*"][0];
        assert_eq!(item.frames.as_ref().unwrap().len(), FRAMES);
        assert_eq!(put.ttl_secs, Some(PUSH_TTL_SECS));
    }

    #[test]
    fn alert_frames_jump_to_the_blocked_session_and_calm_ones_pet() {
        let mut st = cat();
        let alert = st.plan(&inputs(1, false, 1), T).unwrap();
        let put = build_put(&alert, None);
        let clicks = |put: &WidgetPut| -> Vec<String> {
            put.scopes["*"][0]
                .frames
                .as_ref()
                .unwrap()
                .iter()
                .map(|f| f[0].click.as_ref().unwrap().action.clone())
                .collect()
        };
        assert!(clicks(&put).iter().all(|a| a == "jump.blocked"));
        // The lit bang uses the needs-input token, never a fixed colour.
        let bang_colours: Vec<_> = put.scopes["*"][0]
            .frames
            .as_ref()
            .unwrap()
            .iter()
            .flatten()
            .filter(|s| s.text == "!")
            .map(|s| s.style.as_ref().unwrap().fg.clone().unwrap())
            .collect();
        assert!(!bang_colours.is_empty());
        assert!(bang_colours.iter().all(|c| c == "needs-input"));
        let mut calm = cat();
        let frames = calm.plan(&inputs(0, true, 1), T + 60_000).unwrap();
        assert!(clicks(&build_put(&frames, None)).iter().all(|a| a == "pet"));
    }

    #[test]
    fn a_set_validates_against_the_daemons_widget_limits() {
        let mut st = cat();
        st.plan(&inputs(1, false, 1), T).unwrap();
        let put = build_put(&st.frames, Some("hi"));
        let ok = garage_plugin_sdk::validate_put(garage_plugin_sdk::WidgetSlot::StripRight, put)
            .unwrap();
        let item = &ok.scopes["*"][0];
        assert_eq!(item.frames.as_ref().unwrap().len(), FRAMES);
        assert_eq!(item.frame_x.as_ref().unwrap().len(), FRAMES);
        assert_eq!(item.frame_ms, Some(TICK_MS as u32));
        assert_eq!(ok.ttl_secs, Some(PUSH_TTL_SECS));
    }
}
