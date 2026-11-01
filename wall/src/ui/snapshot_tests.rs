//! insta snapshots of the rendered rail, strip and tile titles (p22 task 0.1,
//! moved from p20). Each snapshot records the cell text AND the style runs
//! (fg / bg / modifiers), so a color or emphasis regression shows up, not
//! just a wording one. They guard the p22 refactors: any snapshot change is
//! a behaviour change.
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::api::models::{SessionInfo, WorkspaceInfo};
use crate::slots::SlotRegistry;
use crate::state::store::WallStore;
use crate::state::wall_state::WallSession;
use crate::ui::plugins_overlay::{PluginsOverlayState, render_plugins_overlay};
use crate::ui::rail::render_rail;
use crate::ui::strip::{STATUSLINE_HINT, render_strip};
use crate::ui::tile::{TileView, render_tile};

/// Text rows, then one line per run of identically styled non-default cells.
fn dump(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let row: String = (area.left()..area.right())
            .map(|x| buf[(x, y)].symbol())
            .collect();
        out.push_str(&format!("|{}|\n", row.trim_end()));
    }
    out.push_str("--- styles ---\n");
    for y in area.top()..area.bottom() {
        let mut x = area.left();
        while x < area.right() {
            let key = |x: u16| {
                let c = &buf[(x, y)];
                (c.fg, c.bg, c.modifier)
            };
            let k = key(x);
            let mut end = x + 1;
            while end < area.right() && key(end) == k {
                end += 1;
            }
            if k != (Color::Reset, Color::Reset, Modifier::empty()) {
                out.push_str(&format!(
                    "r{y} c{x}..{end} fg={:?} bg={:?} mod={:?}\n",
                    k.0, k.1, k.2
                ));
            }
            x = end;
        }
    }
    out
}

fn ws(name: &str, dir: &str) -> WorkspaceInfo {
    WorkspaceInfo {
        name: name.to_owned(),
        dir: Some(dir.to_owned()),
        branch: None,
    }
}

fn si(workspace: &str, label: &str, status: &str, since: Option<i64>) -> SessionInfo {
    SessionInfo {
        id: format!("garage/{workspace}/{label}"),
        workspace: workspace.to_owned(),
        label: label.to_owned(),
        dir: None,
        attached: false,
        status: status.to_owned(),
        since,
        message: None,
        branch: None,
        restorable: status == "restorable",
        title: None,
        context: None,
    }
}

fn render_rail_dump(store: &WallStore, w: u16, h: u16) -> String {
    let rect = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(rect);
    render_rail(
        &mut buf,
        rect,
        store.state(),
        &SlotRegistry::core(),
        125_000,
    );
    dump(&buf)
}

#[test]
fn rail_family_label_blocked_family_and_restorable_rows() {
    let mut store = WallStore::new();
    store.workspaces_fetched(vec![
        ws("et-a", "/work/elite-traders/et-a"),
        ws("et-b", "/work/elite-traders/et-b"),
        ws("solo", "/repos/solo/solo"),
    ]);
    store.sessions_fetched(vec![
        si("et-a", "idle-one", "idle", None),
        si("et-b", "stuck", "needs-input", Some(5_000)),
        si("et-b", "busy", "working", Some(65_000)),
        si("solo", "finished", "done", Some(100_000)),
        si("solo", "old-run", "restorable", None),
    ]);
    insta::assert_snapshot!(render_rail_dump(&store, 30, 12));
}

#[test]
fn rail_empty_state() {
    insta::assert_snapshot!(render_rail_dump(&WallStore::new(), 30, 3));
}

fn strip_dump(store: &WallStore, notice: Option<&str>, w: u16) -> String {
    let rect = Rect::new(0, 0, w, 1);
    let mut buf = Buffer::empty(rect);
    render_strip(&mut buf, rect, store.state(), &SlotRegistry::core(), notice);
    dump(&buf)
}

fn strip_store() -> WallStore {
    let mut store = WallStore::new();
    store.workspaces_fetched(vec![
        ws("alpha", "/r/alpha"),
        ws("beta", "/r/beta"),
        ws("gamma", "/r/gamma"),
    ]);
    store.sessions_fetched(vec![
        si("alpha", "one", "working", Some(0)),
        si("beta", "blocked", "needs-input", Some(0)),
        si("gamma", "also", "needs-input", Some(0)),
    ]);
    store
}

#[test]
fn strip_tabs_blocked_dots_and_no_chip() {
    insta::assert_snapshot!(strip_dump(&strip_store(), None, 100));
}

/// The strip as the usage-bar plugin fills it: its golden widget body goes
/// through the real wire-to-slot path. The snapshot is the one the core chip
/// produced (text, columns and styles are byte-identical).
fn plugin_strip_dump(golden: &str, store: &WallStore, notice: Option<&str>, w: u16) -> String {
    let rect = Rect::new(0, 0, w, 1);
    let mut buf = Buffer::empty(rect);
    let slots = crate::plugins::test_support::registry_with(
        "usage-bar",
        &[],
        garage_protocol::widgets::WidgetSlot::StripRight,
        golden,
    );
    render_strip(&mut buf, rect, store.state(), &slots, notice);
    dump(&buf)
}

#[test]
fn strip_with_usage_chip_and_notice() {
    insta::assert_snapshot!(plugin_strip_dump(
        include_str!("../../../plugins/usage-bar/golden/both.json"),
        &strip_store(),
        Some("restarted 2 sessions"),
        130
    ));
}

#[test]
fn strip_install_hint_is_dim() {
    insta::assert_snapshot!(strip_dump(&strip_store(), Some(STATUSLINE_HINT), 140));
}

#[test]
fn strip_narrow_drops_right_side_content() {
    insta::assert_snapshot!(plugin_strip_dump(
        include_str!("../../../plugins/usage-bar/golden/five-only.json"),
        &strip_store(),
        Some("a notice"),
        60
    ));
}

/// The strip with the pit-pet plugin's frames laid into its filler: an
/// alerting pup with speech, then a dim sleeping cat. The bodies are the
/// plugin's golden files, drawn through the real wire-to-slot path (an
/// animated item with a column per frame). Text, columns and colours are what
/// the in-wall pet drew, the lit `!` in amber included (the `needs-input`
/// token, lit because the store has sessions waiting).
#[test]
fn strip_with_the_pet_in_the_filler() {
    let store = strip_store();
    let mut out = String::new();
    for (name, golden) in [
        (
            "alert with speech",
            include_str!("../../../plugins/pit-pet/golden/alert-speech.json"),
        ),
        (
            "asleep",
            include_str!("../../../plugins/pit-pet/golden/asleep.json"),
        ),
    ] {
        let rect = Rect::new(0, 0, 110, 1);
        let mut buf = Buffer::empty(rect);
        let slots = crate::plugins::test_support::registry_with(
            "pit-pet",
            &["pit-pet:pet"],
            garage_protocol::widgets::WidgetSlot::StripRight,
            golden,
        );
        render_strip(&mut buf, rect, store.state(), &slots, None);
        out.push_str(&format!("## {name}\n{}\n", dump(&buf)));
    }
    insta::assert_snapshot!(out);
}

fn session(status: &str) -> WallSession {
    WallSession {
        id: "garage/ws/lbl".into(),
        workspace: "ws".into(),
        label: "lbl".into(),
        dir: None,
        status: status.into(),
        since: Some(5_000),
        message: None,
        branch: None,
        worktree: false,
        title: None,
        context: None,
    }
}

fn tile_dump(s: &WallSession, focused: bool, engaged: bool, w: u16) -> String {
    tile_dump_at(s, focused, engaged, w, 125_000, false, None)
}

fn tile_dump_at(
    s: &WallSession,
    focused: bool,
    engaged: bool,
    w: u16,
    now_ms: i64,
    fade_done: bool,
    spinner: Option<&str>,
) -> String {
    let slots = SlotRegistry::core();
    let v = TileView {
        session: s,
        focused,
        engaged,
        restoring: false,
        dead_reason: None,
        frozen_new_lines: None,
        now_ms,
        fade_done,
        spinner: spinner.map(str::to_owned),
        slots: &slots,
        gutter: None,
    };
    let rect = Rect::new(0, 0, w, 4);
    let mut buf = Buffer::empty(rect);
    let parser = vt100::Parser::new(2, w.saturating_sub(2).max(1), 0);
    let screen = s.live().then(|| parser.screen());
    render_tile(&mut buf, rect, &v, screen);
    dump(&buf)
}

#[test]
fn tile_titles_for_each_status() {
    let mut out = String::new();
    for status in ["needs-input", "working", "done", "idle", "restorable"] {
        let s = session(status);
        out.push_str(&format!(
            "## {status}\n{}\n",
            tile_dump(&s, false, false, 40)
        ));
    }
    insta::assert_snapshot!(out);
}

#[test]
fn tile_title_branch_title_and_context_meter() {
    let mut s = session("working");
    s.branch = Some("feat/x".into());
    s.title = Some("Fixing the flaky login test".into());
    s.context = Some(crate::api::models::ContextInfo {
        used_percentage: 83,
        source: "statusline".into(),
    });
    let mut out = String::new();
    out.push_str(&format!(
        "## wide focused\n{}\n",
        tile_dump(&s, true, false, 70)
    ));
    out.push_str(&format!(
        "## narrow engaged\n{}\n",
        tile_dump(&s, true, true, 36)
    ));
    s.context = Some(crate::api::models::ContextInfo {
        used_percentage: 10,
        source: "transcript".into(),
    });
    out.push_str(&format!(
        "## cool meter\n{}\n",
        tile_dump(&s, false, false, 70)
    ));
    insta::assert_snapshot!(out);
}

/// p23 task 3.3: the `done` glyph colour at 0, 60 and 120 seconds with
/// motion on — green, a mid step, then the same faint grey the hard switch
/// ends on. The unfaded (motion off) tile is covered by
/// `tile_titles_for_each_status`.
#[test]
fn done_fade_at_0_60_and_120_seconds() {
    let s = session("done"); // since = 5_000
    let mut out = String::new();
    for secs in [0, 60, 120] {
        let now = 5_000 + secs * 1000;
        out.push_str(&format!(
            "## done +{secs}s\n{}\n",
            tile_dump_at(&s, false, false, 40, now, true, None)
        ));
    }
    insta::assert_snapshot!(out);
}

// ── pit-pet plugin: needs-input colour, per-frame clicks, a full window ──

const PET_CYCLE: &str = include_str!("../../../plugins/pit-pet/golden/cycle-24.json");

fn pet_registry(golden: &str) -> SlotRegistry {
    crate::plugins::test_support::registry_with(
        "pit-pet",
        &["pit-pet:pet"],
        garage_protocol::widgets::WidgetSlot::StripRight,
        golden,
    )
}

fn calm_store() -> WallStore {
    let mut store = WallStore::new();
    store.workspaces_fetched(vec![ws("alpha", "/r/alpha")]);
    store.sessions_fetched(vec![si("alpha", "one", "working", Some(0))]);
    store
}

/// The cell of the lit `!` the alert golden draws, and its foreground.
fn bang_fg(store: &WallStore) -> Option<Color> {
    let rect = Rect::new(0, 0, 110, 1);
    let mut buf = Buffer::empty(rect);
    let slots = pet_registry(include_str!(
        "../../../plugins/pit-pet/golden/alert-speech.json"
    ));
    render_strip(&mut buf, rect, store.state(), &slots, None);
    (0..110)
        .map(|x| &buf[(x, 0)])
        .find(|c| c.symbol() == "!")
        .map(|c| c.fg)
}

/// A plugin asks for `needs-input` and the wall decides: amber while some
/// session waits for you, plain text colour the rest of the time. Without
/// this the colour would be lit with nothing to be alerted about.
#[test]
fn the_needs_input_token_is_amber_only_while_a_session_needs_input() {
    assert_eq!(
        bang_fg(&strip_store()),
        Some(crate::ui::theme::colors::AMBER)
    );
    assert_eq!(
        bang_fg(&calm_store()),
        Some(crate::ui::theme::colors::FG),
        "neutral when nothing is waiting"
    );
}

/// A full 24-frame window: the wall cycles through it on its own clock, and a
/// click on an alerting frame is the core jump, with nothing to ask the plugin.
#[test]
fn a_24_frame_window_cycles_on_the_wall_clock_and_its_alert_frames_jump() {
    let slots = pet_registry(PET_CYCLE);
    let store = strip_store();
    let mut seen = std::collections::BTreeSet::new();
    for tick in 0..24 {
        let rect = Rect::new(0, 0, 110, 1);
        let mut buf = Buffer::empty(rect);
        slots.begin_frame(tick * 300);
        let cols = render_strip(&mut buf, rect, store.state(), &slots, None);
        let line: String = (0..110).map(|x| buf[(x, 0)].symbol()).collect();
        seen.insert(line.trim_end().to_owned());
        let click = cols
            .slots
            .iter()
            .find(|(_, t)| t.action == crate::actions::ids::JUMP_BLOCKED);
        assert!(click.is_some(), "frame {tick} jumps to the blocked session");
        assert_eq!(click.unwrap().1.plugin.as_deref(), Some("pit-pet"));
    }
    assert!(seen.len() > 3, "the pet moves through the window: {seen:?}");
    // The 25th tick is the first again.
    let at = |t: i64| {
        let rect = Rect::new(0, 0, 110, 1);
        let mut buf = Buffer::empty(rect);
        slots.begin_frame(t);
        render_strip(&mut buf, rect, store.state(), &slots, None);
        dump(&buf)
    };
    assert_eq!(at(0), at(24 * 300));
}

/// A calm frame keeps the plugin's own click (petting), so the two kinds of
/// frame in one item do not share a target.
#[test]
fn a_calm_frame_pets_and_an_alert_frame_jumps_in_the_same_item() {
    let calm = include_str!("../../../plugins/pit-pet/golden/asleep.json");
    let slots = pet_registry(calm);
    let rect = Rect::new(0, 0, 110, 1);
    let mut buf = Buffer::empty(rect);
    let cols = render_strip(&mut buf, rect, strip_store().state(), &slots, None);
    let target = &cols
        .slots
        .iter()
        .find(|(_, t)| t.plugin.is_some())
        .unwrap()
        .1;
    assert_eq!(target.action.as_str(), "pit-pet:pet");
}

fn plugins_dump(list: serde_json::Value, state: &PluginsOverlayState) -> String {
    let list: garage_protocol::plugins::PluginList =
        serde_json::from_value(serde_json::json!({ "plugins": list })).unwrap();
    let area = Rect::new(0, 0, 80, 12);
    let mut buf = Buffer::empty(area);
    render_plugins_overlay(&mut buf, area, &list, state);
    dump(&buf)
}

#[test]
fn plugins_overlay_populated_with_a_toggle_error() {
    let list = serde_json::json!([
        {"id": "usage-bar", "name": "Usage bar", "enabled": true, "active": true,
         "source": "bundled", "permissions": ["widgets", "notices"]},
        {"id": "pet-extras", "name": "Pet extras", "enabled": false, "active": false,
         "source": "local", "permissions": ["ui.invoke"]},
    ]);
    let state = PluginsOverlayState {
        selected: 1,
        busy: None,
        error: Some("pet-extras: daemon answered 500".into()),
    };
    insta::assert_snapshot!(plugins_dump(list, &state));
}

#[test]
fn plugins_overlay_empty() {
    insta::assert_snapshot!(plugins_dump(
        serde_json::json!([]),
        &PluginsOverlayState::default()
    ));
}
