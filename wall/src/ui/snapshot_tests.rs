//! insta snapshots of the rendered rail, strip and tile titles (p22 task 0.1,
//! moved from p20). Each snapshot records the cell text AND the style runs
//! (fg / bg / modifiers), so a color or emphasis regression shows up, not
//! just a wording one. They guard the p22 refactors: any snapshot change is
//! a behaviour change.
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::api::models::{SessionInfo, UsageInfo, UsageWindow, WorkspaceInfo};
use crate::state::store::WallStore;
use crate::state::wall_state::WallSession;
use crate::ui::rail::render_rail;
use crate::ui::strip::{render_strip, STATUSLINE_HINT};
use crate::ui::tile::{render_tile, TileView};

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
    render_rail(&mut buf, rect, store.state(), 125_000);
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
    render_strip(&mut buf, rect, store.state(), notice, None);
    dump(&buf)
}

fn strip_store() -> WallStore {
    let mut store = WallStore::new();
    store.workspaces_fetched(vec![ws("alpha", "/r/alpha"), ws("beta", "/r/beta"), ws("gamma", "/r/gamma")]);
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

#[test]
fn strip_with_usage_chip_and_notice() {
    let mut store = strip_store();
    store.usage_fetched(UsageInfo {
        five_hour: Some(UsageWindow { used_percentage: 24, resets_at: None }),
        seven_day: Some(UsageWindow { used_percentage: 61, resets_at: None }),
    });
    insta::assert_snapshot!(strip_dump(&store, Some("restarted 2 sessions"), 130));
}

#[test]
fn strip_install_hint_is_dim() {
    insta::assert_snapshot!(strip_dump(&strip_store(), Some(STATUSLINE_HINT), 140));
}

#[test]
fn strip_narrow_drops_right_side_content() {
    let mut store = strip_store();
    store.usage_fetched(UsageInfo {
        five_hour: Some(UsageWindow { used_percentage: 98, resets_at: None }),
        seven_day: None,
    });
    insta::assert_snapshot!(strip_dump(&store, Some("a notice"), 60));
}

/// The strip with the pet laid into its filler: a lit bang beside speech,
/// then a dim sleeping pet without one. Guards the p22 move of the pet onto
/// the `strip.right` slot (the columns and styles must not shift).
#[cfg(feature = "pet")]
#[test]
fn strip_with_the_pet_in_the_filler() {
    use crate::ui::strip::PetRender;
    let store = strip_store();
    let mut out = String::new();
    let alert = PetRender {
        text: "(O.O)",
        bang: true,
        bang_lit: true,
        x: 30,
        dim: false,
        say: Some("someone needs you".into()),
    };
    let asleep = PetRender {
        text: "=-.-=",
        bang: false,
        bang_lit: false,
        x: 4,
        dim: true,
        say: None,
    };
    for (name, pet) in [("alert with speech", alert), ("asleep", asleep)] {
        let rect = Rect::new(0, 0, 110, 1);
        let mut buf = Buffer::empty(rect);
        render_strip(&mut buf, rect, store.state(), None, Some(pet));
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
    let v = TileView {
        session: s,
        focused,
        engaged,
        restoring: false,
        dead_reason: None,
        frozen_new_lines: None,
        now_ms: 125_000,
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
        out.push_str(&format!("## {status}\n{}\n", tile_dump(&s, false, false, 40)));
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
    out.push_str(&format!("## wide focused\n{}\n", tile_dump(&s, true, false, 70)));
    out.push_str(&format!("## narrow engaged\n{}\n", tile_dump(&s, true, true, 36)));
    s.context = Some(crate::api::models::ContextInfo {
        used_percentage: 10,
        source: "transcript".into(),
    });
    out.push_str(&format!("## cool meter\n{}\n", tile_dump(&s, false, false, 70)));
    insta::assert_snapshot!(out);
}
