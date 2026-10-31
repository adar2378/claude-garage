//! One run of the plugin: read stored state under a lock, look at the world,
//! plan the next window, push it, store. All the logic is in [`crate::plan`];
//! this is the glue, written so a test can hand it a fake daemon and a clock.

use std::path::Path;
use std::time::Duration;

use garage_plugin_sdk::{Api, Error, StateLock, Value, read_state_in, write_state_in};

use crate::engine::Species;
use crate::plan::{DEBOUNCE_MS, Inputs, State, build_put};

pub const WIDGET: &str = "pet";
const STORE: &str = "pet.json";
const LOCK: &str = "pet";
/// A run holds the lock for milliseconds; a lock this old died with its run.
const STALE: Duration = Duration::from_secs(20);
/// How long a run waits for another run's lock before it skips.
pub const LOCK_WAIT: Duration = Duration::from_secs(8);

/// Everything a run needs from outside.
pub struct Env<'a> {
    pub api: &'a Api,
    pub dir: &'a Path,
    pub now: i64,
    /// The run context JSON (`ui.slots` gives the strip's room).
    pub context: &'a Value,
    /// Called with `(rev, delay_ms)` when a caption must be taken down later.
    pub schedule_clear: &'a dyn Fn(u64, i64),
    /// `date +%H`, injected.
    pub local_hour: &'a dyn Fn() -> u8,
    /// How long to wait for the state lock ([`LOCK_WAIT`]).
    pub lock_wait: Duration,
}

fn load(env: &Env) -> State {
    let mut st: State = read_state_in(env.dir, STORE).unwrap_or_default();
    if !st.migrated {
        st.migrated = true;
        if st.species.is_none()
            && let Some(old) = wall_json_pet(env.dir)
        {
            st.species = Some(old);
        }
    }
    st
}

/// A run that has nothing to do: no pet is chosen, nothing is on the strip,
/// and the old `wall.json` has no pet to adopt. It must not take the lock or
/// write a file, so a plugin nobody uses costs nothing but the process start.
fn idle(env: &Env) -> bool {
    match read_state_in::<State>(env.dir, STORE) {
        Some(st) => {
            st.species.is_none() && !st.pushed && (st.migrated || wall_json_pet(env.dir).is_none())
        }
        None => wall_json_pet(env.dir).is_none(),
    }
}

/// The pet the wall used to keep in `wall.json` (`<garage dir>/wall.json`; the
/// plugin's state dir is `<garage dir>/plugins/data/<id>`). Read once.
fn wall_json_pet(state_dir: &Path) -> Option<Species> {
    let garage = state_dir.parent()?.parent()?.parent()?;
    let raw = std::fs::read_to_string(garage.join("wall.json")).ok()?;
    let doc: Value = serde_json::from_str(&raw).ok()?;
    Species::from_str(doc.get("pet")?.as_str()?)
}

fn save(env: &Env, st: &State) -> Result<(), Error> {
    write_state_in(env.dir, STORE, st)
}

fn lock(env: &Env) -> Result<StateLock, Error> {
    StateLock::acquire(env.dir, LOCK, STALE, env.lock_wait)
}

/// The strip filler's width as the sim wants it: the room minus the two gutter
/// columns, or a modest default before the TUI has reported its geometry.
pub fn filler_width(context: &Value) -> u16 {
    let at = |k: &str| {
        context
            .pointer(&format!("/ui/slots/strip.right/filler/{k}"))
            .and_then(Value::as_u64)
    };
    match (at("x0"), at("x1")) {
        (Some(a), Some(b)) if b > a => (b - a).saturating_sub(2).min(u64::from(u16::MAX)) as u16,
        _ => 20,
    }
}

struct World {
    blocked: usize,
    any_working: bool,
    session_count: usize,
}

fn sessions(api: &Api) -> Result<World, Error> {
    let list = api.get("/api/sessions")?;
    let list = list.as_array().cloned().unwrap_or_default();
    let status = |s: &Value| {
        s.get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    Ok(World {
        blocked: list.iter().filter(|s| status(s) == "needs-input").count(),
        any_working: list.iter().any(|s| status(s) == "working"),
        session_count: list.len(),
    })
}

fn usage_pct_max(api: &Api) -> Option<u8> {
    let u = api.get("/api/usage").ok()?;
    ["fiveHour", "sevenDay"]
        .iter()
        .filter_map(|k| u.get(k)?.get("usedPercentage")?.as_f64())
        .map(|p| p.clamp(0.0, 100.0).round() as u8)
        .max()
}

fn inputs(env: &Env, st: &mut State, world: &World) -> Inputs {
    let minute = env.now.div_euclid(60_000);
    let hour = match st.hour_cache {
        Some((m, h)) if m == minute => h,
        _ => {
            let h = (env.local_hour)();
            st.hour_cache = Some((minute, h));
            h
        }
    };
    Inputs {
        blocked: world.blocked,
        any_working: world.any_working,
        session_count: world.session_count,
        usage_pct_max: usage_pct_max(env.api),
        filler_width: filler_width(env.context),
        local_hour: hour,
        ascii_pup: env.dir.join("ascii").exists(),
    }
}

/// Plan and push the next window (or clear the widget when there is no pet).
/// `force` is for what the user did (a key, a click) and for a daemon that
/// forgot the widget; the rest wait at least [`DEBOUNCE_MS`] after the last
/// push, so a burst of session events makes one push.
fn show(env: &Env, st: &mut State, world: &World, force: bool) -> Result<(), Error> {
    if !force && st.species.is_some() && (0..DEBOUNCE_MS).contains(&(env.now - st.last_push_ms)) {
        return Ok(());
    }
    let inputs = inputs(env, st, world);
    let Some(frames) = st.plan(&inputs, env.now) else {
        return clear(env, st);
    };
    st.rev += 1;
    let caption = st.caption_at(env.now).cloned();
    env.api.put_widget(
        WIDGET,
        &build_put(&frames, caption.as_ref().map(|c| c.text.as_str())),
    )?;
    st.pushed = true;
    st.last_push_ms = env.now;
    save(env, st)?;
    if let Some(c) = caption {
        (env.schedule_clear)(st.rev, c.until_ms - env.now);
    }
    Ok(())
}

/// Take the pet off the strip. Writes the state only when something was
/// there to take off.
fn clear(env: &Env, st: &mut State) -> Result<(), Error> {
    if !st.pushed && st.frames.is_empty() {
        return Ok(());
    }
    if st.pushed {
        env.api.clear_widget(WIDGET)?;
        st.pushed = false;
    }
    st.frames.clear();
    save(env, st)
}

/// `update`: timer and session events. Keeps a window arriving.
pub fn update(env: &Env) -> Result<(), Error> {
    if idle(env) {
        return Ok(());
    }
    let _lock = lock(env)?;
    let mut st = load(env);
    if st.species.is_none() {
        return clear(env, &mut st);
    }
    let world = sessions(env.api)?;
    show(env, &mut st, &world, false)
}

/// `started`: the daemon (re)started and forgot every widget.
pub fn started(env: &Env) -> Result<(), Error> {
    if idle(env) {
        return Ok(());
    }
    let _lock = lock(env)?;
    let mut st = load(env);
    st.started_ms = env.now;
    st.pushed = false;
    if st.species.is_none() {
        return save(env, &st);
    }
    let world = sessions(env.api)?;
    show(env, &mut st, &world, true)
}

/// `cycle`: the `P` key.
pub fn cycle(env: &Env) -> Result<(), Error> {
    let _lock = lock(env)?;
    let mut st = load(env);
    st.cycle(env.now);
    if st.species.is_none() {
        // The choice itself changed, so the state is written even when the
        // strip had nothing to take down.
        if st.pushed {
            env.api.clear_widget(WIDGET)?;
        }
        st.pushed = false;
        st.frames.clear();
        save(env, &st)?;
        // A toast, not a caption: there is no pet left to hold one. A refused
        // or limited notice does not fail the key.
        let _ = env.api.notice("the strip is quiet again", Some(3000));
        return Ok(());
    }
    let world = sessions(env.api)?;
    show(env, &mut st, &world, true)
}

/// `pet`: a click on a calm pet. (An alerting pet's frames carry
/// `jump.blocked` themselves, so a click on one never reaches this run; if a
/// session started waiting since the click was painted, the pet stays put.)
pub fn pet(env: &Env) -> Result<(), Error> {
    let _lock = lock(env)?;
    let mut st = load(env);
    let world = sessions(env.api)?;
    if world.blocked > 0 {
        return Ok(());
    }
    if !st.pet(env.now) {
        return Ok(()); // the pet was turned off between paint and click
    }
    show(env, &mut st, &world, true)
}

/// `clear <rev>`: take the caption of push `rev` down, unless a newer push
/// has replaced it.
pub fn clear_caption(env: &Env, rev: u64) -> Result<(), Error> {
    let _lock = lock(env)?;
    let mut st = load(env);
    if st.rev != rev || st.caption.is_none() || st.frames.is_empty() || st.species.is_none() {
        return Ok(());
    }
    st.caption = None;
    st.rev += 1;
    env.api.put_widget(WIDGET, &build_put(&st.frames, None))?;
    st.last_push_ms = env.now;
    save(env, &st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{FRAMES, PET_CAPTION_MS, PUSH_TTL_SECS, TICK_MS};
    use garage_plugin_sdk::fake::FakeDaemon;
    use garage_plugin_sdk::json;
    use std::cell::RefCell;

    const NOW: i64 = 1_800_000_000_000;
    const PUT: &str = "/api/plugins/pit-pet/widgets/pet";

    struct Rig {
        d: FakeDaemon,
        api: Api,
        dir: std::path::PathBuf,
        ctx: Value,
        cleared: RefCell<Vec<(u64, i64)>>,
    }

    fn rig(tag: &str, sessions: &str) -> Rig {
        let d = FakeDaemon::start();
        d.route("GET", "/api/sessions", 200, sessions);
        d.route(
            "GET",
            "/api/usage",
            200,
            r#"{"fiveHour":null,"sevenDay":null}"#,
        );
        let dir = std::env::temp_dir().join(format!("pit-pet-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Rig {
            api: Api::new(&d.base, "pit-pet", "tok"),
            d,
            dir,
            ctx: json!({"ui": {"slots": {"strip.right": {"width": 120, "filler": {"x0": 20, "x1": 80}}}}}),
            cleared: RefCell::default(),
        }
    }

    impl Rig {
        fn env(&self, now: i64) -> Env<'_> {
            Env {
                api: &self.api,
                dir: &self.dir,
                now,
                context: &self.ctx,
                schedule_clear: Box::leak(Box::new({
                    let cleared = &self.cleared;
                    move |rev, ms| cleared.borrow_mut().push((rev, ms))
                })),
                local_hour: &|| 12,
                lock_wait: Duration::from_millis(60),
            }
        }
        fn state(&self) -> State {
            read_state_in(&self.dir, STORE).expect("state was stored")
        }
        fn puts(&self) -> Vec<Value> {
            self.d.bodies("PUT", PUT)
        }
        fn set(&self, f: impl FnOnce(&mut State)) {
            let mut st: State = read_state_in(&self.dir, STORE).unwrap_or_default();
            st.migrated = true;
            f(&mut st);
            write_state_in(&self.dir, STORE, &st).unwrap();
        }
    }

    const ONE_BLOCKED: &str =
        r#"[{"id":"garage/a/x","status":"needs-input"},{"id":"garage/a/y","status":"idle"}]"#;

    #[test]
    fn the_strip_room_comes_from_the_context_minus_the_two_gutter_columns() {
        assert_eq!(
            filler_width(
                &json!({"ui": {"slots": {"strip.right": {"filler": {"x0": 20, "x1": 80}}}}})
            ),
            58
        );
        assert_eq!(
            filler_width(&json!({"ui": {"slots": {}}})),
            20,
            "before the TUI reports"
        );
        assert_eq!(filler_width(&Value::Null), 20);
    }

    #[test]
    fn an_update_pushes_a_full_moving_window_for_the_sessions_it_sees() {
        let r = rig("update", ONE_BLOCKED);
        r.set(|st| st.species = Some(Species::Cat));
        update(&r.env(NOW)).unwrap();
        let puts = r.puts();
        assert_eq!(puts.len(), 1);
        let item = &puts[0]["scopes"]["*"][0];
        assert_eq!(item["frames"].as_array().unwrap().len(), FRAMES);
        assert_eq!(item["frame_x"].as_array().unwrap().len(), FRAMES);
        assert_eq!(item["frame_ms"], TICK_MS);
        assert_eq!(item["placement"]["kind"], "filler");
        // One session needs input: the cat alerts, the bang is lit in the
        // needs-input colour, and a click goes straight to the blocked session.
        let frame = &item["frames"][((NOW / TICK_MS) % FRAMES as i64) as usize];
        assert_eq!(frame[0]["text"], "=O_O=");
        assert_eq!(frame[0]["click"]["action"], "jump.blocked");
        assert_eq!(frame[2]["text"], "!");
        assert_eq!(frame[2]["style"]["fg"], "needs-input");
        assert_eq!(puts[0]["ttl_secs"], PUSH_TTL_SECS, "a dead pet disappears");
        assert_eq!(r.state().rev, 1);
        assert!(
            r.cleared.borrow().is_empty(),
            "no caption, nothing to take down"
        );
    }

    #[test]
    fn with_no_pet_a_run_does_nothing_and_turning_it_off_clears_once() {
        let r = rig("off", ONE_BLOCKED);
        update(&r.env(NOW)).unwrap();
        assert!(
            r.d.seen()
                .iter()
                .all(|s| s.method == "GET" || s.path.starts_with("/api/sessions")),
            "{:?}",
            r.d.seen()
        );
        assert!(r.puts().is_empty());
        // Pup is the third pick; the fourth turns it off.
        r.set(|st| st.species = Some(Species::Pup));
        update(&r.env(NOW)).unwrap();
        assert_eq!(r.puts().len(), 1);
        cycle(&r.env(NOW + 1000)).unwrap();
        let deletes = || r.d.seen().iter().filter(|s| s.method == "DELETE").count();
        assert_eq!(deletes(), 1, "the pet is gone from the strip");
        assert_eq!(r.state().species, None);
        update(&r.env(NOW + 6000)).unwrap();
        assert_eq!(deletes(), 1, "a pet that is off is not cleared again");
    }

    #[test]
    fn p_brings_the_first_pet_with_its_arrival_line_and_asks_for_it_to_be_taken_down() {
        let r = rig("cycle", "[]");
        cycle(&r.env(NOW)).unwrap();
        let st = r.state();
        assert_eq!(st.species, Some(Species::Cat));
        let item = &r.puts()[0]["scopes"]["*"][0];
        assert_eq!(
            item["placement"]["caption"],
            "Arthur the shop cat is on the wall"
        );
        assert_eq!(*r.cleared.borrow(), vec![(st.rev, 3000)]);
    }

    #[test]
    fn a_click_that_reaches_the_plugin_while_a_session_waits_changes_nothing() {
        let r = rig("jump", ONE_BLOCKED);
        r.set(|st| st.species = Some(Species::Duck));
        pet(&r.env(NOW)).unwrap();
        assert!(
            r.d.bodies("POST", "/api/plugins/ui/action").is_empty(),
            "the jump is on the frames; the plugin never asks for it"
        );
        assert!(r.puts().is_empty(), "no petting while it is asking for you");
    }

    #[test]
    fn a_click_on_a_calm_pet_pets_it() {
        let r = rig("pet", r#"[{"id":"garage/a/x","status":"working"}]"#);
        r.set(|st| st.species = Some(Species::Cat));
        pet(&r.env(NOW)).unwrap();
        assert!(r.d.bodies("POST", "/api/plugins/ui/action").is_empty());
        let item = &r.puts()[0]["scopes"]["*"][0];
        assert!(
            item["placement"]["caption"]
                .as_str()
                .is_some_and(|c| !c.is_empty()),
            "a reply"
        );
        let frame = &item["frames"][((NOW / TICK_MS) % FRAMES as i64) as usize];
        assert!(
            frame[0]["text"].as_str().unwrap().starts_with("=^.^="),
            "celebrating: {frame}"
        );
        let st = r.state();
        assert_eq!(*r.cleared.borrow(), vec![(st.rev, PET_CAPTION_MS)]);
        // A click with no pet at all is harmless.
        let none = rig("pet-none", "[]");
        pet(&none.env(NOW)).unwrap();
        assert!(none.puts().is_empty());
    }

    #[test]
    fn taking_a_caption_down_redraws_the_same_frames_and_only_for_its_own_push() {
        let r = rig("clear", "[]");
        cycle(&r.env(NOW)).unwrap();
        let rev = r.state().rev;
        // A newer push has happened since: the old timer must not undo it.
        clear_caption(&r.env(NOW + 3000), rev + 1).unwrap();
        assert_eq!(r.puts().len(), 1);
        clear_caption(&r.env(NOW + 3000), rev).unwrap();
        let puts = r.puts();
        assert_eq!(puts.len(), 2);
        assert!(
            puts[1]["scopes"]["*"][0]["placement"]
                .get("caption")
                .is_none_or(Value::is_null)
        );
        assert_eq!(
            puts[1]["scopes"]["*"][0]["frames"],
            puts[0]["scopes"]["*"][0]["frames"]
        );
        assert!(r.state().caption.is_none());
        clear_caption(&r.env(NOW + 3100), r.state().rev).unwrap();
        assert_eq!(r.puts().len(), 2, "already down");
    }

    #[test]
    fn a_daemon_restart_draws_the_pet_again_and_restarts_the_uptime() {
        let r = rig("started", "[]");
        r.set(|st| {
            st.species = Some(Species::Pup);
            st.pushed = true;
        });
        started(&r.env(NOW)).unwrap();
        assert_eq!(r.puts().len(), 1);
        assert_eq!(r.state().started_ms, NOW);
    }

    #[test]
    fn the_pet_the_wall_used_to_keep_follows_the_user_once() {
        let garage = std::env::temp_dir().join(format!("pit-pet-garage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&garage);
        std::fs::create_dir_all(&garage).unwrap();
        std::fs::write(
            garage.join("wall.json"),
            r#"{"version":1,"views":{},"pet":"duck"}"#,
        )
        .unwrap();
        let mut r = rig("migrate", "[]");
        r.dir = garage.join("plugins/data/pit-pet");
        update(&r.env(NOW)).unwrap();
        assert_eq!(r.state().species, Some(Species::Duck));
        assert_eq!(r.puts().len(), 1, "and it is drawn");
        // Later the user turns it off; the old file is not consulted again.
        cycle(&r.env(NOW + 1000)).unwrap();
        cycle(&r.env(NOW + 2000)).unwrap();
        assert_eq!(r.state().species, None);
        update(&r.env(NOW + 3000)).unwrap();
        assert_eq!(r.state().species, None);
    }

    #[test]
    fn a_daemon_that_cannot_list_sessions_fails_the_run_instead_of_drawing_a_guess() {
        let r = rig("nosessions", "[]");
        r.d.route("GET", "/api/sessions", 500, r#"{"error":"x"}"#);
        r.set(|st| st.species = Some(Species::Cat));
        assert!(update(&r.env(NOW)).is_err());
        assert!(r.puts().is_empty());
    }

    #[test]
    fn with_no_pet_a_run_touches_nothing_at_all() {
        // Not the daemon, not the state file, not the lock.
        for run in [update, started] {
            let r = rig("idle", ONE_BLOCKED);
            run(&r.env(NOW)).unwrap();
            assert!(r.d.seen().is_empty(), "{:?}", r.d.seen());
            assert!(
                !r.dir.exists(),
                "no pet.json, no lock, not even the directory"
            );
        }
        // A pet that was turned off keeps its (one) state file and is left alone.
        let r = rig("idle-off", "[]");
        r.set(|st| st.species = None);
        let before = std::fs::read(r.dir.join(STORE)).unwrap();
        update(&r.env(NOW)).unwrap();
        assert!(r.d.seen().is_empty());
        assert_eq!(std::fs::read(r.dir.join(STORE)).unwrap(), before);
    }

    #[test]
    fn a_burst_of_events_makes_one_push_but_a_key_or_a_restart_always_pushes() {
        let r = rig("debounce", "[]");
        r.set(|st| st.species = Some(Species::Cat));
        update(&r.env(NOW)).unwrap();
        update(&r.env(NOW + 100)).unwrap();
        update(&r.env(NOW + 299)).unwrap();
        assert_eq!(r.puts().len(), 1, "inside 300 ms of the last push");
        update(&r.env(NOW + 300)).unwrap();
        assert_eq!(r.puts().len(), 2);
        // The user's own key and a daemon that forgot the widget are not debounced.
        pet(&r.env(NOW + 310)).unwrap();
        assert_eq!(r.puts().len(), 3);
        started(&r.env(NOW + 320)).unwrap();
        assert_eq!(r.puts().len(), 4);
        cycle(&r.env(NOW + 330)).unwrap();
        assert_eq!(r.puts().len(), 5);
    }

    #[test]
    fn a_run_that_cannot_get_the_lock_skips_with_a_distinct_error() {
        let r = rig("locked", "[]");
        r.set(|st| st.species = Some(Species::Cat));
        let held = StateLock::acquire(&r.dir, LOCK, STALE, Duration::ZERO).unwrap();
        for run in [update, started, cycle, pet] {
            assert!(
                matches!(run(&r.env(NOW)), Err(Error::LockHeld(_))),
                "a busy lock is a skip the binary exits 0 on, not a failure"
            );
        }
        assert!(
            matches!(clear_caption(&r.env(NOW), 1), Err(Error::LockHeld(_))),
            "the caption clear too"
        );
        assert!(r.puts().is_empty());
        drop(held);
        update(&r.env(NOW)).unwrap();
        assert_eq!(r.puts().len(), 1);
    }

    #[test]
    fn turning_the_pet_off_says_so_in_a_toast_and_a_refused_toast_does_not_fail_the_key() {
        let r = rig("quiet", "[]");
        r.set(|st| st.species = Some(Species::Pup));
        cycle(&r.env(NOW)).unwrap();
        assert_eq!(
            r.d.bodies("POST", "/api/plugins/pit-pet/notice"),
            vec![json!({"text": "the strip is quiet again", "ttl_ms": 3000})]
        );
        assert_eq!(r.state().species, None);
        // Rate limited (or no permission): the key still works.
        let r = rig("quiet-429", "[]");
        r.d.route(
            "POST",
            "/api/plugins/pit-pet/notice",
            429,
            r#"{"error":"slow"}"#,
        );
        r.set(|st| st.species = Some(Species::Pup));
        cycle(&r.env(NOW)).unwrap();
        assert_eq!(r.state().species, None, "state saved before the toast");
        // Cycling to a pet is not a toast: the arrival line is its caption.
        let r = rig("arrive", "[]");
        cycle(&r.env(NOW)).unwrap();
        assert!(r.d.bodies("POST", "/api/plugins/pit-pet/notice").is_empty());
    }
}
