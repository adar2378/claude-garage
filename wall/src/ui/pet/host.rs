//! `PetHost`: the runtime's single handle on the pit pet — motion sim,
//! chatter scheduler, mood, PRNG and the frame it last rendered (moved out
//! of `runtime/mod.rs`, p22 task 5.1).

use std::cell::RefCell;
use std::rc::Rc;

use crate::runtime::{NoticeKind, Notices};
use crate::slots::{SlotCtx, SlotItem, SlotProvider};
use crate::state::wall_state::WallState;
use crate::ui::pet::{self, PetClick};
use crate::ui::strip::{PetRender, pet_slot_item};

/// A tiny xorshift64 PRNG (the pet module takes time and randomness
/// injected as `rng: &mut impl FnMut() -> f64`; this is the one concrete
/// source — no new crate for one PRNG). Not cryptographic, not seeded for
/// reproducibility across runs: the pet's idle strolls and chatter picks
/// only need to look unpredictable, not be.
fn xorshift_next(state: &mut u64) -> f64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    // Top 53 bits → [0.0, 1.0), matching the precision of an f64 mantissa.
    (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// The frame handed from the host to its slot provider.
type Frame = Rc<RefCell<Option<PetRender>>>;

/// The pet's `strip.right` provider: serves whatever frame the host last
/// published (its own state, which is why providers are trait objects).
struct PetSlot(Frame);

impl SlotProvider for PetSlot {
    fn items(&self, _ctx: &SlotCtx) -> Vec<SlotItem> {
        self.0.borrow().iter().map(pet_slot_item).collect()
    }
}

pub struct PetHost {
    /// Motion state; reset whenever `P` changes the species (a fresh pet
    /// starts centered, not wherever the last one wandered).
    sim: pet::PetSim,
    /// Chatter's own rate-limit/dedup state — never reset by `P` (the spec
    /// doesn't ask for it, and it'd only let a species-swap dodge the floor).
    chatter: pet::Chatter,
    /// Next epoch ms the 300 ms tick is due; `0` so a freshly-enabled pet
    /// renders on the very next loop pass.
    next_tick_ms: i64,
    /// This frame's sprite — `None` whenever the pet is off.
    render: Option<PetRender>,
    /// What the `strip.right` provider serves: the sprite with its chatter
    /// line, republished by [`PetHost::publish_frame`] before each draw.
    frame: Frame,
    /// `GARAGE_PET_ASCII=1`, read once at construction (the pup's face
    /// glyphs have an ASCII fallback for fonts that mangle them).
    ascii: bool,
    /// Epoch ms the wall started — `ChatterCtx::uptime_ms`'s zero point.
    started_ms: i64,
    /// The effective mood from the last tick: detects a fresh transition
    /// into `Celebrate` and is the authority for click routing.
    mood: pet::Mood,
    /// `daemon_live` as of the last tick (the box → live edge).
    prev_daemon_live: bool,
    /// xorshift64 state feeding every `rng` the pet module needs.
    rng: u64,
    /// `(minute-bucket, hour)` — `ChatterCtx::local_hour` is shelled out to
    /// `date +%H` at most once a minute, not once per 300 ms tick.
    local_hour_cache: Option<(i64, u8)>,
}

impl PetHost {
    /// `GARAGE_PET_CHATTER_MS` (E2E-only) lowers the chatter floor so a
    /// harness need not wait 3 real minutes for a line; absent in every
    /// normal run.
    pub fn new(started_ms: i64) -> Option<Self> {
        let chatter = std::env::var("GARAGE_PET_CHATTER_MS")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .map(pet::Chatter::with_floor_ms)
            .unwrap_or_default();
        Some(PetHost {
            sim: pet::PetSim::default(),
            chatter,
            next_tick_ms: 0,
            render: None,
            frame: Frame::default(),
            ascii: std::env::var("GARAGE_PET_ASCII").ok().as_deref() == Some("1"),
            started_ms,
            mood: pet::Mood::Sleep,
            prev_daemon_live: true, // matches WallState::initial()'s daemon_live default
            rng: (started_ms as u64) | 1, // xorshift64 needs a non-zero seed
            local_hour_cache: None,
        })
    }

    /// `P` changed the species: a fresh pet starts its motion sim from
    /// scratch and renders on the very next tick.
    pub fn reset_sim(&mut self) {
        self.sim = pet::PetSim::default();
        self.next_tick_ms = 0;
    }

    /// The next roster choice for `P` (off → Arthur → Papito → Segan → off)
    /// and its arrival notice. `None` where there is no pet to cycle.
    pub fn cycle_choice(current: Option<&str>) -> Option<(Option<String>, String)> {
        let next = pet::Species::cycle(current.and_then(pet::Species::from_str));
        let text = match next {
            Some(species) => format!("{} is on the wall", species.label()),
            None => "the strip is quiet again".to_owned(),
        };
        Some((next.map(|s| s.as_str().to_owned()), text))
    }

    /// The provider the app registers on `strip.right`.
    pub fn slot_provider(&self) -> Option<Box<dyn SlotProvider>> {
        Some(Box::new(PetSlot(self.frame.clone())))
    }

    /// Hand the provider this frame's sprite (with the live chatter line).
    pub fn publish_frame(&self, notices: &Notices, now: i64) {
        *self.frame.borrow_mut() = self.strip_render(notices, now);
    }

    /// This frame's sprite with the live pet-notice line as its speech.
    pub fn strip_render(&self, notices: &Notices, now: i64) -> Option<PetRender> {
        self.render.clone().map(|mut p| {
            p.say = notices.current_of(NoticeKind::Pet, now).map(str::to_owned);
            p
        })
    }

    /// A click on the pet's strip columns (spec tui-pit-pet "Click
    /// routing"): while alert, the caller performs the jump `a` does;
    /// otherwise a petting interaction (a short celebrate + a
    /// species-voiced reply).
    pub fn click(&mut self, now: i64, state: &WallState, notices: &mut Notices) -> PetClick {
        if self.mood == pet::Mood::Alert {
            return PetClick::JumpToBlocked;
        }
        let Some(species) = state.pet.as_deref().and_then(pet::Species::from_str) else {
            return PetClick::Handled; // race: `P` turned it off between paint and click
        };
        self.sim.pet(now);
        // Petting is a celebrate the sim will report next tick; pre-set the
        // mood so it is not mistaken for a blocked→clear transition (which
        // would trigger a Proud line over the petting reply).
        self.mood = pet::Mood::Celebrate;
        let mut rng_state = self.rng;
        let mut rng = || xorshift_next(&mut rng_state);
        let line = pet::petting_line(species, &mut rng);
        self.rng = rng_state;
        if let Some(line) = line {
            notices.show_pet(line, 2500, now);
        }
        PetClick::Handled
    }

    /// `ChatterCtx::local_hour`, shelled out to `date +%H` at most once a
    /// minute (no time crate in this workspace's `Cargo.toml`). Falls back
    /// to noon (never late-night, never usage-adjacent) if the shell-out
    /// fails — chatter staying silent on a broken clock beats it misfiring
    /// the late-night lines all day.
    fn local_hour(&mut self, now: i64) -> u8 {
        let minute_bucket = now.div_euclid(60_000);
        if let Some((bucket, hour)) = self.local_hour_cache
            && bucket == minute_bucket
        {
            return hour;
        }
        let hour = std::process::Command::new("date")
            .arg("+%H")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u8>().ok())
            .unwrap_or(12);
        self.local_hour_cache = Some((minute_bucket, hour));
        hour
    }

    /// The 300 ms pet tick (spec tui-pit-pet "Species-true one-row
    /// motion"). Off → one early return and nothing else runs. On → steps
    /// the sim, updates the render, and runs the chatter scheduler in the
    /// same beat. `strip_filler` is the last-drawn strip's filler width.
    /// Returns whether the caller should mark the frame dirty.
    pub fn tick(
        &mut self,
        now: i64,
        state: &WallState,
        notices: &mut Notices,
        strip_filler: u16,
    ) -> bool {
        let Some(species) = state.pet.as_deref().and_then(pet::Species::from_str) else {
            // Off: nothing to render, nothing to say — cheap by construction.
            return self.render.take().is_some();
        };
        if now < self.next_tick_ms {
            return false;
        }
        self.next_tick_ms = now + 300;

        let usage_pct_max = [
            state.usage.five_hour.as_ref(),
            state.usage.seven_day.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(|w| w.used_percentage.min(u32::from(u8::MAX)) as u8)
        .max();
        let (daemon_live, blocked, any_working, session_count) = (
            state.daemon_live,
            state.blocked_count(),
            state.sessions.iter().any(|s| s.status == "working"),
            state.sessions.len(),
        );
        let mood_in = pet::mood(daemon_live, blocked > 0, any_working, session_count);
        // `strip_filler` is `0` before the first frame is ever drawn — a
        // conservative fallback keeps the very first tick from computing
        // against a zero-width filler and hiding the pet before it's ever
        // been seen. Minus the strip's two gutter columns (see
        // `strip_line`), so the sim's clamp and the paint's clamp agree.
        let filler_width = if strip_filler > 0 {
            strip_filler.saturating_sub(2)
        } else {
            20
        };

        let mut rng_state = self.rng;
        let mut rng = || xorshift_next(&mut rng_state);
        let result = self.sim.step(
            species,
            mood_in,
            blocked,
            session_count,
            filler_width,
            now,
            self.ascii,
            &mut rng,
        );
        self.rng = rng_state;

        let just_celebrated =
            self.mood != pet::Mood::Celebrate && result.mood == pet::Mood::Celebrate;
        let just_back_live = !self.prev_daemon_live && daemon_live;
        self.mood = result.mood;
        self.prev_daemon_live = daemon_live;

        let was_none = self.render.is_none();
        self.render = Some(PetRender {
            text: result.sprite.text,
            bang: result.sprite.bang,
            bang_lit: result.bang_lit,
            x: result.x,
            dim: matches!(result.mood, pet::Mood::Sleep | pet::Mood::Box),
            say: None, // filled at draw time from the live pet notice
        });
        let mut dirty = result.changed || was_none;

        if result.mood == pet::Mood::Alert && notices.clear_pet() {
            dirty = true;
        }
        let ctx = pet::ChatterCtx {
            now_ms: now,
            uptime_ms: now - self.started_ms,
            local_hour: self.local_hour(now),
            usage_pct_max,
            mood: result.mood,
            just_celebrated,
            just_back_live,
            alert: result.mood == pet::Mood::Alert,
            user_notice_active: notices.user_notice_active(now),
        };
        let mut rng_state = self.rng;
        let mut rng = || xorshift_next(&mut rng_state);
        let line = self.chatter.next_line(species, &ctx, &mut rng);
        self.rng = rng_state;
        if let Some(line) = line
            && notices.show_pet(line, 4000, now)
        {
            dirty = true;
        }

        dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::{SessionInfo, WorkspaceInfo};
    use crate::state::store::WallStore;

    fn store_with_blocked(pet: Option<&str>) -> WallStore {
        let mut store = WallStore::new();
        store.workspaces_fetched(vec![WorkspaceInfo {
            name: "a".into(),
            dir: Some("/r/a".into()),
            branch: None,
        }]);
        store.sessions_fetched(vec![SessionInfo {
            id: "garage/a/one".into(),
            workspace: "a".into(),
            label: "one".into(),
            dir: None,
            attached: false,
            status: "needs-input".into(),
            since: Some(0),
            message: None,
            branch: None,
            restorable: false,
            title: None,
            context: None,
        }]);
        store.set_pet(pet.map(str::to_owned));
        store
    }

    #[test]
    fn off_renders_nothing_and_a_selected_pet_renders_on_the_first_tick() {
        let mut host = PetHost::new(0).unwrap();
        let mut notices = Notices::default();
        let off = store_with_blocked(None);
        assert!(!host.tick(1_000, off.state(), &mut notices, 40));
        assert!(host.strip_render(&notices, 1_000).is_none());

        let on = store_with_blocked(Some("cat"));
        assert!(
            host.tick(1_000, on.state(), &mut notices, 40),
            "first tick is dirty"
        );
        assert!(host.strip_render(&notices, 1_000).is_some());
        assert!(
            !host.tick(1_100, on.state(), &mut notices, 40),
            "300 ms gate holds"
        );

        // Turning it off clears the render once, then stays quiet.
        assert!(host.tick(2_000, off.state(), &mut notices, 40));
        assert!(host.strip_render(&notices, 2_000).is_none());
    }

    #[test]
    fn click_while_alert_asks_for_the_jump_else_pets() {
        let mut host = PetHost::new(0).unwrap();
        let mut notices = Notices::default();
        let on = store_with_blocked(Some("cat"));
        host.tick(1_000, on.state(), &mut notices, 40);
        // A blocked session makes the effective mood Alert.
        assert_eq!(
            host.click(1_100, on.state(), &mut notices),
            PetClick::JumpToBlocked
        );

        let mut calm = PetHost::new(0).unwrap();
        assert_eq!(
            calm.click(1_100, on.state(), &mut notices),
            PetClick::Handled
        );
        let off = store_with_blocked(None);
        assert_eq!(
            calm.click(1_200, off.state(), &mut notices),
            PetClick::Handled
        );
    }

    #[test]
    fn the_published_frame_reaches_the_strip_through_the_slot_provider_with_a_click_range() {
        use crate::actions::ids;
        use crate::slots::{SlotId, SlotRegistry};
        let mut host = PetHost::new(0).unwrap();
        let mut slots = SlotRegistry::core();
        slots.register(SlotId::StripRight, host.slot_provider().unwrap());
        let mut notices = Notices::default();
        let on = store_with_blocked(Some("cat"));
        let strip =
            |slots: &SlotRegistry| crate::ui::strip::strip_line(on.state(), slots, None, 160);

        // Nothing published yet: no pet, no click range.
        assert!(
            strip(&slots)
                .slots
                .iter()
                .all(|(_, t)| t.action != ids::PET_PET)
        );

        host.tick(1_000, on.state(), &mut notices, 100);
        host.publish_frame(&notices, 1_000);
        let line = strip(&slots);
        let (range, _) = line
            .slots
            .iter()
            .find(|(_, t)| t.action == ids::PET_PET)
            .expect("pet is clickable");
        assert_eq!(
            usize::from(range.end - range.start),
            host.strip_render(&notices, 1_000)
                .map(|p| p.text.chars().count() + if p.bang { 2 } else { 0 })
                .unwrap(),
            "the item is exactly as wide as the sprite plus its bang column"
        );

        // Turned off: the next published frame is empty again.
        let off = store_with_blocked(None);
        host.tick(2_000, off.state(), &mut notices, 100);
        host.publish_frame(&notices, 2_000);
        assert!(
            strip(&slots)
                .slots
                .iter()
                .all(|(_, t)| t.action != ids::PET_PET)
        );
    }

    #[test]
    fn cycle_choice_walks_the_roster_and_back_to_off() {
        let (first, text) = PetHost::cycle_choice(None).unwrap();
        assert!(first.is_some());
        assert!(text.ends_with("is on the wall"), "{text}");
        let mut current = first;
        for _ in 0..3 {
            current = PetHost::cycle_choice(current.as_deref()).unwrap().0;
        }
        assert_eq!(current, None, "three pets, then off");
    }
}
