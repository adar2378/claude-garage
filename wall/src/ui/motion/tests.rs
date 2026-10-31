use super::*;
use crate::state::wall_state::WallState;

fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
}

// ── Off switch ───────────────────────────────────────────────────────────

#[test]
fn motion_defaults_on() {
    assert!(motion_enabled(None, &env_of(&[])));
    assert!(motion_enabled(Some("# nothing here\n"), &env_of(&[])));
}

#[test]
fn each_env_switch_turns_motion_off() {
    for name in ["GARAGE_NO_MOTION", "NO_MOTION", "REDUCE_MOTION"] {
        let env = move |n: &str| (n == name).then(|| "1".to_owned());
        assert!(!motion_enabled(None, &env), "{name}");
    }
}

#[test]
fn env_values_that_mean_no_do_not_switch_motion_off() {
    for value in ["", "0", "false", "off", "No"] {
        let env = move |n: &str| (n == "GARAGE_NO_MOTION").then(|| value.to_owned());
        assert!(motion_enabled(None, &env), "{value:?}");
    }
    // NO_MOTION follows the no-color convention: any other value counts.
    let env = |n: &str| (n == "NO_MOTION").then(|| "yes-please".to_owned());
    assert!(!motion_enabled(None, &env));
}

#[test]
fn config_animations_false_turns_motion_off_and_env_beats_config_true() {
    assert!(!motion_enabled(Some("animations = false\n"), &env_of(&[])));
    assert!(!motion_enabled(Some("animations = true\n"), &env_of(&[("REDUCE_MOTION", "1")])));
    assert!(motion_enabled(Some("animations = true\n"), &env_of(&[])));
}

#[test]
fn config_parsing_ignores_comments_quotes_and_other_keys() {
    assert_eq!(config_animations("animations = false # off for ssh"), Some(false));
    assert_eq!(config_animations("[ui]\n  animations=\"true\"\n"), Some(true));
    assert_eq!(config_animations("# animations = false\n"), None);
    assert_eq!(config_animations("pet_animations = false\n"), None);
    assert_eq!(config_animations("animations = maybe\n"), None);
}

// ── Idle cost (task 3.1) ─────────────────────────────────────────────────

fn tile() -> Rect {
    Rect::new(0, 0, 20, 6)
}

/// A tile as the renderer would paint it: border + a line of text.
fn painted() -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    let r = tile();
    for x in r.left()..r.right() {
        buf[(x, r.top())].set_char('─').set_fg(colors::AMBER);
        buf[(x, r.bottom() - 1)].set_char('─').set_fg(colors::AMBER);
    }
    for y in r.top()..r.bottom() {
        buf[(r.left(), y)].set_char('│').set_fg(colors::AMBER);
        buf[(r.right() - 1, y)].set_char('│').set_fg(colors::AMBER);
    }
    for (i, ch) in "hello world".chars().enumerate() {
        buf[(2 + i as u16, 2)].set_char(ch).set_fg(Color::White);
    }
    buf
}

/// One frame at `now_ms` with the tile in the given state; returns the
/// finished buffer.
fn frame(m: &mut Motion, now_ms: i64, attached: bool, blocked: bool) -> Buffer {
    let mut buf = painted();
    m.begin_frame(now_ms);
    m.tile_drawn("a", tile(), attached, blocked);
    m.apply(&mut buf);
    buf
}

#[test]
fn nothing_animating_schedules_no_tick() {
    let mut m = Motion::new(true);
    assert_eq!(m.next_tick(), None, "fresh");
    // A tile that has been attached and calm since before any frame we saw
    // is only seeded by its first frame's fade-in...
    frame(&mut m, 0, true, false);
    // ...and once that is over, the clock goes quiet again.
    let mut now = 0;
    while m.next_tick().is_some() {
        now += 33;
        frame(&mut m, now, true, false);
        assert!(now < 2_000, "effect never finished");
    }
    assert_eq!(m.next_tick(), None);
    frame(&mut m, now + 5_000, true, false);
    assert_eq!(m.next_tick(), None, "a calm tile asks for no redraw");
}

#[test]
fn an_effect_asks_for_the_fx_tick_and_a_lone_spinner_for_the_slower_one() {
    let mut m = Motion::new(true);
    frame(&mut m, 0, true, false);
    assert_eq!(m.next_tick(), Some(FX_TICK));

    let mut m = Motion::new(true);
    m.begin_frame(0);
    m.spinner_shown();
    assert_eq!(m.next_tick(), Some(SPINNER_TICK));
    // The flag is per frame: the next frame without a spinner clears it.
    m.begin_frame(100);
    assert_eq!(m.next_tick(), None);
}

// ── Motion off (task 3.2) ────────────────────────────────────────────────

#[test]
fn with_motion_off_no_effect_starts_and_frames_are_untouched() {
    let mut m = Motion::new(false);
    for now in [0, 33, 66] {
        let buf = frame(&mut m, now, true, true);
        assert_eq!(buf, painted(), "frame at {now}");
    }
    m.spinner_shown();
    m.daemon_restart_requested(0);
    assert!(!m.effects_running());
    assert_eq!(m.next_tick(), None);
    assert_eq!(m.tile_spinner(0), None, "tiles keep their static marker");
    assert!(m.strip_spinner().is_none());
    assert!(!m.strip_spinner_active(false, 0));
}

// ── Effects ──────────────────────────────────────────────────────────────

#[test]
fn fade_in_fires_once_on_attach_and_only_dims_text() {
    let mut m = Motion::new(true);
    frame(&mut m, 0, false, false);
    assert!(!m.effects_running(), "a placeholder tile does not fade");
    let first = frame(&mut m, 33, true, false);
    assert!(m.effects_running());
    // The fade starts dark (its first step is zero-length, so the idle gap
    // before the attach never eats the animation).
    assert_eq!(first[(2, 2)].fg, Color::Rgb(40, 40, 40));
    let mid = frame(&mut m, 133, true, false);
    assert_ne!(mid[(2, 2)].fg, Color::White, "text is mid-fade");
    assert_ne!(mid[(2, 2)].fg, Color::Rgb(40, 40, 40), "and has moved");
    assert_eq!(mid[(2, 2)].symbol(), "h", "text never moves");
}

#[test]
fn finished_effects_leave_the_plain_frame_behind() {
    let mut m = Motion::new(true);
    frame(&mut m, 0, true, false);
    let mut now = 0;
    let mut last_animated = None;
    while m.next_tick().is_some() {
        now += 33;
        let buf = frame(&mut m, now, true, false);
        if m.next_tick().is_some() {
            last_animated = Some(buf);
        }
    }
    assert!(last_animated.is_some());
    assert_eq!(frame(&mut m, now + 33, true, false), painted(), "settled frame == plain");
}

#[test]
fn the_attention_pulse_fires_on_the_edge_only_and_touches_only_the_frame() {
    let mut m = Motion::new(true);
    // First sight of an already blocked tile: no pulse (no news).
    frame(&mut m, 0, true, true);
    let mut now = 0;
    while m.next_tick().is_some() {
        now += 33;
        frame(&mut m, now, true, true);
    }
    assert!(!m.effects_running());

    // working -> needs-input: one pulse.
    frame(&mut m, now + 33, true, false);
    frame(&mut m, now + 66, true, true);
    assert!(m.effects_running());
    let mid = frame(&mut m, now + 200, true, true);
    assert_eq!(mid[(2, 2)], painted()[(2, 2)], "terminal body untouched");
    assert_ne!(mid[(5, 0)].fg, colors::AMBER, "border is mid-pulse");

    // It ends and does not repeat while the tile stays blocked.
    let mut t = now + 200;
    while m.next_tick().is_some() {
        t += 33;
        frame(&mut m, t, true, true);
        assert!(t < now + 3_000);
    }
    frame(&mut m, t + 1_000, true, true);
    assert_eq!(m.next_tick(), None);
}

#[test]
fn a_tile_that_stops_being_drawn_drops_its_effects() {
    let mut m = Motion::new(true);
    frame(&mut m, 0, true, false);
    assert!(m.effects_running());
    m.begin_frame(33);
    let mut buf = painted();
    m.apply(&mut buf); // tile not drawn this frame (workspace switch)
    assert_eq!(m.next_tick(), None);
}

#[test]
fn a_closed_session_dissolves_over_the_new_layout_then_is_gone() {
    let mut m = Motion::new(true);
    frame(&mut m, 0, true, false); // snapshot of the clean frame
    let mut now = 0;
    while m.next_tick().is_some() {
        now += 33;
        frame(&mut m, now, true, false);
    }
    // The session left the list.
    m.begin_frame(now + 33);
    m.closed(|_| false);
    assert!(m.effects_running(), "ghost counts as running");
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    m.apply(&mut buf);
    assert_eq!(buf[(2, 2)].symbol(), "h", "first ghost frame shows the old tile");
    let mut t = now + 33;
    let mut frames = 0;
    while m.next_tick().is_some() {
        t += 33;
        frames += 1;
        m.begin_frame(t);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        m.apply(&mut buf);
        assert!(frames < 100, "dissolve never ended");
    }
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    m.begin_frame(t + 33);
    m.apply(&mut buf);
    assert_eq!(buf, Buffer::empty(Rect::new(0, 0, 40, 10)), "nothing of the tile remains");
}

#[test]
fn a_session_that_still_exists_never_dissolves() {
    let mut m = Motion::new(true);
    frame(&mut m, 0, true, false);
    m.begin_frame(33);
    m.closed(|_| true); // merely off-screen
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    m.apply(&mut buf);
    assert_eq!(m.next_tick(), None);
}

// ── Done fade ────────────────────────────────────────────────────────────

#[test]
fn done_fade_runs_green_to_neutral_with_exact_endpoints() {
    assert_eq!(done_fade_color(Some(1000), 1000), colors::GREEN);
    assert_eq!(done_fade_color(Some(1000), 1000 + DONE_FADE_MS), colors::FAINT);
    assert_eq!(done_fade_color(Some(1000), 1000 + 10 * DONE_FADE_MS), colors::FAINT);
    assert_eq!(done_fade_color(None, i64::MAX), colors::GREEN, "unknown since never fades");
    let mid = done_fade_color(Some(0), DONE_FADE_MS / 2);
    assert!(matches!(mid, Color::Rgb(..)));
}

#[test]
fn done_fade_never_leaves_the_green_to_neutral_range_and_never_amber() {
    let mut last_r = 0;
    for step in 0..=DONE_FADE_STEPS * 2 {
        let c = done_fade_color(Some(0), step * (DONE_FADE_MS / DONE_FADE_STEPS));
        assert_ne!(c, colors::AMBER);
        if let Color::Rgb(r, g, b) = c {
            assert!(g >= r && g >= b, "step {step}: {c:?} is not green/neutral");
            assert!(r >= last_r, "monotone toward neutral");
            last_r = r;
        }
    }
}

#[test]
fn done_fade_changes_color_once_per_elapsed_tick() {
    // The loop's 10 s elapsed tick is what advances the fade: one new
    // colour per tick, no finer.
    let step_ms = DONE_FADE_MS / DONE_FADE_STEPS;
    assert_eq!(step_ms, 10_000);
    assert_eq!(done_fade_color(Some(0), step_ms - 1), colors::GREEN);
    assert_ne!(done_fade_color(Some(0), step_ms), colors::GREEN);
}

// ── Spinner ──────────────────────────────────────────────────────────────

#[test]
fn spinner_cycles_six_single_cell_frames() {
    let frames: Vec<String> = (0..6).map(|i| spinner_symbol(i * SPINNER_TICK.as_millis() as i64)).collect();
    let mut unique = frames.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 6);
    assert!(frames.iter().all(|f| f.chars().count() == 1));
    assert_eq!(spinner_symbol(6 * SPINNER_TICK.as_millis() as i64), frames[0]);
}

#[test]
fn strip_spinner_shows_while_down_or_restarting_and_clears_after_the_reconnect() {
    let mut m = Motion::new(true);
    let provider = m.strip_spinner().expect("motion on");
    let mut state = WallState::initial();
    let items = |state: &WallState, now: i64| {
        provider.clock.set(now);
        provider.items(&SlotCtx { state: Some(state), ..SlotCtx::default() })
    };
    assert!(items(&state, 0).is_empty(), "daemon live, nothing restarting");

    m.daemon_restart_requested(0);
    let text: String = items(&state, 10).remove(0).text.iter().map(|s| s.content.to_string()).collect();
    assert!(text.contains("daemon restarting"));

    state.daemon_live = false;
    m.connection_changed(false, 400);
    assert!(!items(&state, 500).is_empty());

    state.daemon_live = true;
    m.connection_changed(true, 800);
    assert!(items(&state, 900).is_empty(), "back up: spinner gone");
    assert!(!m.strip_spinner_active(true, 900));
}

#[test]
fn a_restart_request_that_never_drops_the_stream_times_out() {
    let mut m = Motion::new(true);
    m.daemon_restart_requested(0);
    m.connection_changed(true, 0); // no drop seen: does not clear
    assert!(m.strip_spinner_active(true, 1_000));
    assert!(!m.strip_spinner_active(true, RESTART_GRACE_MS + 1));
}

#[test]
fn a_dead_daemon_stops_the_spinner_after_the_cap_and_shows_a_static_caption() {
    let mut m = Motion::new(true);
    let provider = m.strip_spinner().expect("motion on");
    let mut state = WallState::initial();
    state.daemon_live = false;
    m.connection_changed(false, 1_000);
    let text = |now: i64| -> String {
        provider.clock.set(now);
        provider
            .items(&SlotCtx { state: Some(&state), ..SlotCtx::default() })
            .remove(0)
            .text
            .iter()
            .map(|s| s.content.to_string())
            .collect()
    };
    assert!(text(1_000 + DOWN_SPIN_MS - 1).contains("reconnecting"));
    assert!(m.strip_spinner_active(false, 1_000 + DOWN_SPIN_MS - 1));
    assert_eq!(text(1_000 + DOWN_SPIN_MS), "◌ daemon down");
    assert!(!m.strip_spinner_active(false, 1_000 + DOWN_SPIN_MS), "no spinner, so no tick");
    // A later drop-less call must not restart the clock.
    m.connection_changed(false, 99_000);
    assert!(!m.strip_spinner_active(false, 100_000));
}

#[test]
fn a_switch_and_return_does_not_fade_the_tile_in_again() {
    let mut m = Motion::new(true);
    frame(&mut m, 0, true, false);
    assert!(m.effects_running(), "first attach fades in");
    let mut now = 0;
    while m.effects_running() {
        now += 33;
        frame(&mut m, now, true, false);
        assert!(now < 2_000, "fade never finished");
    }
    // Switch away: the tile is drawn detached (or not at all), then back.
    frame(&mut m, now + 100, false, false);
    frame(&mut m, now + 200, true, false);
    assert!(!m.effects_running(), "re-attach after a switch is not a first attach");
    assert_eq!(m.next_tick(), None);
}

#[test]
fn a_closed_session_id_that_comes_back_fades_in_as_a_new_session() {
    let mut m = Motion::new(true);
    frame(&mut m, 0, true, false);
    m.closed(|_| false);
    // Let the dissolve finish, then the id reappears.
    let mut now = 0;
    while m.effects_running() {
        now += 33;
        m.begin_frame(now);
        let mut buf = painted();
        m.apply(&mut buf);
        assert!(now < 2_000);
    }
    frame(&mut m, now + 100, true, false);
    assert!(m.effects_running());
}
