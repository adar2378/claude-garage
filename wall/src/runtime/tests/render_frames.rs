//! Dev-only: real renders of the wall for the website and brand material.
//!
//! Not a regression test. It does nothing unless `RENDER_FRAMES=1`, so a
//! normal `cargo test` stays untouched:
//!
//! ```text
//! RENDER_FRAMES=1 cargo test -p garage-wall --lib render_brand_frames -- --nocapture
//! ```
//!
//! Each frame goes through `App::draw_frame`, the function the runtime draws
//! with, on a ratatui `TestBackend`. Nothing is hand-drawn: workspaces and
//! sessions come in as the daemon's own records, tile bodies are ANSI bytes
//! fed to a real `vt100` parser (drawn by the same `render_tile`), and the
//! pit pet, usage bar, closed chats and tile buttons arrive as plugin widget
//! content through the real wire-to-slot path, from the plugins' golden JSON
//! where they have one. No tmux, no daemon, no PTY.
//!
//! `RENDER_OUT` sets the output directory (default `docs/brand/renders`);
//! `RENDER_CHROME=1` adds a window frame.
use super::svg_render::{Options, to_svg};
use super::*;
use crate::ui::layout::tile_inner;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Value, json};

const NOW: i64 = 1_800_000_000_000;
const SIZES: [(u16, u16); 2] = [(160, 44), (120, 36)];

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    NeedsInput,
    Engaged,
    Calm,
    Plugins,
}

impl Scene {
    fn file(self) -> &'static str {
        match self {
            Scene::NeedsInput => "wall-needs-input",
            Scene::Engaged => "wall-engaged",
            Scene::Calm => "wall-calm",
            Scene::Plugins => "wall-plugins",
        }
    }
    fn waiting(self) -> bool {
        matches!(self, Scene::NeedsInput | Scene::Engaged)
    }
}

// ── fixtures ──────────────────────────────────────────────────────────────

fn workspace(name: &str, branch: &str) -> WorkspaceInfo {
    WorkspaceInfo {
        name: name.to_owned(),
        dir: Some(format!("/work/acme/{name}")),
        branch: Some(branch.to_owned()),
    }
}

#[allow(clippy::too_many_arguments)]
fn session(
    ws: &str,
    label: &str,
    status: &str,
    age_s: i64,
    ctx: u32,
    title: &str,
    message: Option<&str>,
) -> SessionInfo {
    SessionInfo {
        id: id(ws, label),
        workspace: ws.to_owned(),
        label: label.to_owned(),
        dir: Some(format!("/work/acme/{ws}")),
        attached: false,
        status: status.to_owned(),
        since: Some(NOW - age_s * 1000),
        message: message.map(str::to_owned),
        branch: None,
        restorable: false,
        title: Some(title.to_owned()),
        context: Some(crate::api::models::ContextInfo {
            used_percentage: ctx,
            source: "statusline".to_owned(),
        }),
    }
}

fn sessions(scene: Scene) -> Vec<SessionInfo> {
    let web1 = if scene.waiting() {
        session(
            "web",
            "claude-1",
            "needs-input",
            47,
            68,
            "fix checkout total rounding",
            Some("Claude needs your permission to use Bash"),
        )
    } else {
        session(
            "web",
            "claude-1",
            "working",
            47,
            68,
            "fix checkout total rounding",
            None,
        )
    };
    vec![
        session(
            "api",
            "claude-1",
            "working",
            252,
            41,
            "session middleware to signed tokens",
            None,
        ),
        // 95 s into `done`: green has started to fade (the fade runs 2 min).
        session("api", "claude-2", "done", 95, 23, "bump axios to 1.7", None),
        web1,
        session(
            "web",
            "claude-2",
            "working",
            130,
            35,
            "pricing page copy",
            None,
        ),
        session(
            "infra",
            "claude-1",
            "idle",
            1080,
            12,
            "terraform plan",
            None,
        ),
    ]
}

fn plugin_listing(scene: Scene) -> Value {
    let mut plugins = vec![
        json!({"id": "pit-pet", "name": "Pit pet", "enabled": true, "active": true,
            "widgets": [{"id": "pet", "slot": "strip.right"}]}),
        json!({"id": "usage-bar", "name": "Usage bar", "enabled": true, "active": true,
            "widgets": [{"id": "usage", "slot": "strip.right"}]}),
    ];
    if scene == Scene::Plugins {
        plugins.push(
            json!({"id": "closed-chats", "name": "Closed chats", "enabled": true,
            "active": true, "widgets": [{"id": "rows", "slot": "rail.extra_rows"}]}),
        );
        plugins.push(
            json!({"id": "tile-buttons", "name": "Tile buttons", "enabled": true,
            "active": true,
            "buttons": [
                {"slot": "tile.title.right", "label": " x ", "on_click": "session.close",
                 "confirm": true, "priority": 5},
                {"slot": "tile.title.right", "label": " del ", "on_click": "session.delete",
                 "confirm": true, "priority": 4}
            ]}),
        );
    }
    json!({ "plugins": plugins })
}

/// What `plugins/closed-chats` pushes for one closed record (its `row`).
fn closed_row(ws: &str, label: &str, reason_hint: &str) -> Value {
    json!({
        "priority": 0,
        "session": id(ws, label),
        "segments": [
            {"text": format!("  o {label} - {reason_hint}"), "style": {"fg": "dim"},
             "click": {"action": "session.restore"}},
            {"text": " x ", "style": {"fg": "dim"}, "click": {"action": "session.delete"}}
        ]
    })
}

fn widget_snapshot(scene: Scene) -> Value {
    // The golden file is the plugin's own output; only its `scopes` is state.
    let golden = |s: &str| -> Value {
        serde_json::from_str::<Value>(s).expect("golden JSON")["scopes"].clone()
    };
    let pet = if scene.waiting() {
        golden(include_str!(
            "../../../../plugins/pit-pet/golden/alert-speech.json"
        ))
    } else {
        golden(include_str!(
            "../../../../plugins/pit-pet/golden/asleep.json"
        ))
    };
    let mut plugins = json!({
        "pit-pet": {"rev": 1, "widgets": {"pet": {"slot": "strip.right", "scopes": pet}}},
        "usage-bar": {"rev": 1, "widgets": {"usage": {"slot": "strip.right",
            "scopes": golden(include_str!("../../../../plugins/usage-bar/golden/both.json"))}}},
    });
    if scene == Scene::Plugins {
        plugins["closed-chats"] = json!({"rev": 1, "widgets": {"rows": {
        "slot": "rail.extra_rows",
        "scopes": {
            "api": [
                closed_row("api", "claude-3", "closed"),
                closed_row("api", "claude-4", "crashed"),
            ],
            "web": [closed_row("web", "claude-3", "closed")],
            "infra": [closed_row("infra", "claude-2", "restart")],
        }}}});
    }
    json!({ "plugins": plugins })
}

// ── tile bodies (ANSI through a real vt100 parser) ────────────────────────

const ORANGE: &str = "38;2;217;119;87";
const GREY: &str = "38;5;245";
const DIMMER: &str = "38;5;240";
const GREEN: &str = "38;5;114";
const WHITE: &str = "38;5;252";
const ADD_BG: &str = "48;2;28;58;36;38;5;114";
const DEL_BG: &str = "48;2;66;30;36;38;5;210";

type Seg = (&'static str, String);

fn s(sgr: &'static str, t: &str) -> Seg {
    (sgr, t.to_owned())
}

/// A bullet line: orange `●` then text.
fn bullet(t: &str) -> Vec<Seg> {
    vec![s(ORANGE, "● "), s(WHITE, t)]
}
fn sub(t: &str) -> Vec<Seg> {
    vec![s(GREY, "  ⎿  "), s(GREY, t)]
}
fn txt(sgr: &'static str, t: &str) -> Vec<Seg> {
    vec![s(sgr, t)]
}
fn blank() -> Vec<Seg> {
    vec![]
}
fn diff(sign: char, n: u32, code: &str, cols: usize, add: bool) -> Vec<Seg> {
    let text = format!("     {n:>3} {sign} {code}");
    let pad = cols.saturating_sub(text.chars().count());
    vec![(
        if add { ADD_BG } else { DEL_BG },
        format!("{text}{}", " ".repeat(pad)),
    )]
}
fn ctx_line(n: u32, code: &str) -> Vec<Seg> {
    vec![s(DIMMER, &format!("     {n:>3}   {code}"))]
}

fn clip(segs: &[Seg], cols: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for (sgr, text) in segs {
        let room = cols.saturating_sub(used);
        let t: String = text.chars().take(room).collect();
        used += t.chars().count();
        out.push_str(&format!("\x1b[0;{sgr}m{t}"));
    }
    out.push_str("\x1b[0m");
    out
}

/// A rounded input box with the permission question inside.
fn permission_box(cols: usize) -> Vec<Vec<Seg>> {
    let inner = cols.saturating_sub(2);
    let row = |t: &str, sgr: &'static str| -> Vec<Seg> {
        let pad = inner.saturating_sub(t.chars().count());
        vec![
            s(ORANGE, "│"),
            (sgr, format!("{t}{}", " ".repeat(pad))),
            s(ORANGE, "│"),
        ]
    };
    vec![
        vec![s(ORANGE, &format!("╭{}╮", "─".repeat(inner)))],
        row(" Bash command", "1;38;5;252"),
        row("", WHITE),
        row("   pnpm vitest run src/checkout", WHITE),
        row("   Run the checkout test suite", GREY),
        row("", WHITE),
        row(" Do you want to proceed?", WHITE),
        row(" ❯ 1. Yes", "1;38;2;217;119;87"),
        row("   2. Yes, and don't ask again for pnpm", WHITE),
        row("   3. No, and tell Claude what to do", WHITE),
        row("      differently (esc)", WHITE),
        vec![s(ORANGE, &format!("╰{}╯", "─".repeat(inner)))],
    ]
}

fn input_box(cols: usize, hint: &str) -> Vec<Vec<Seg>> {
    let inner = cols.saturating_sub(2);
    let pad = inner.saturating_sub(2);
    vec![
        vec![s(DIMMER, &format!("╭{}╮", "─".repeat(inner)))],
        vec![
            s(DIMMER, "│"),
            s(WHITE, " >"),
            s(DIMMER, &" ".repeat(pad)),
            s(DIMMER, "│"),
        ],
        vec![s(DIMMER, &format!("╰{}╯", "─".repeat(inner)))],
        vec![s(GREY, &format!("  {hint}"))],
    ]
}

/// Earlier turns, so a tall tile is full the way a long session is: the body
/// is bottom-anchored and the oldest lines scroll off.
fn history(ws: &str, label: &str) -> Vec<Vec<Seg>> {
    let pairs: &[(&str, &str)] = match (ws, label) {
        ("api", "claude-1") => &[
            ("Read(src/middleware/session.ts)", "Read 84 lines"),
            ("Read(src/auth/token.ts)", "Read 51 lines"),
            ("Grep(\"req.cookies\")", "Found 6 matches in 4 files"),
            ("Read(src/routes/refresh.ts)", "Read 37 lines"),
            ("Bash(git diff --stat)", "3 files changed, 41 insertions(+)"),
            ("Read(src/middleware/rate-limit.ts)", "Read 29 lines"),
            ("Grep(\"verifyToken\")", "Found 3 matches in 2 files"),
        ],
        ("api", "claude-2") => &[
            ("Read(package.json)", "Read 62 lines"),
            ("Bash(pnpm up axios@1.7.2)", "axios 1.6.8 -> 1.7.2"),
            ("Grep(\"axios\")", "Found 9 matches in 5 files"),
            ("Read(src/http/client.ts)", "Read 58 lines"),
            ("Bash(pnpm tsc --noEmit)", "1 error: AxiosHeaders"),
            ("Read(node_modules/axios/index.d.ts)", "Read 120 lines"),
            ("Bash(pnpm tsc --noEmit)", "No errors"),
        ],
        ("web", "claude-1") => &[
            ("Read(src/checkout/total.ts)", "Read 63 lines"),
            ("Read(src/checkout/total.test.ts)", "Read 112 lines"),
            ("Grep(\"taxRate\")", "Found 5 matches in 3 files"),
            ("Bash(pnpm vitest run src/checkout)", "1 failed, 13 passed"),
            ("Read(src/checkout/format.ts)", "Read 24 lines"),
            ("Grep(\"toFixed\")", "Found 2 matches in 1 file"),
        ],
        ("web", _) => &[
            ("Read(src/pages/pricing.tsx)", "Read 96 lines"),
            ("Read(content/plans.json)", "Read 41 lines"),
            ("Grep(\"Team plan\")", "Found 4 matches in 3 files"),
            ("Read(src/components/PlanCard.tsx)", "Read 58 lines"),
            ("Bash(git log --oneline -5)", "5 commits"),
            ("Read(src/pages/pricing.test.tsx)", "Read 44 lines"),
            ("Grep(\"annual\")", "Found 2 matches in 2 files"),
        ],
        _ => &[
            ("Read(main.tf)", "Read 142 lines"),
            ("Bash(terraform init)", "Terraform has been initialized"),
            (
                "Bash(terraform validate)",
                "Success! The configuration is valid.",
            ),
        ],
    };
    let mut out = Vec::new();
    for (call, result) in pairs {
        out.push(bullet(call));
        out.push(sub(result));
        out.push(blank());
    }
    out
}

fn body_for(ws: &str, label: &str, waiting: bool, cols: usize) -> (Vec<Vec<Seg>>, Vec<Vec<Seg>>) {
    let (body, footer) = body_inner(ws, label, waiting, cols);
    let mut all = history(ws, label);
    all.extend(body);
    (all, footer)
}

fn body_inner(ws: &str, label: &str, waiting: bool, cols: usize) -> (Vec<Vec<Seg>>, Vec<Vec<Seg>>) {
    match (ws, label) {
        ("api", "claude-1") => (
            vec![
                bullet("Moving sessions to signed tokens."),
                blank(),
                bullet("Update(src/middleware/session.ts)"),
                sub("Updated session.ts (+3 -2)"),
                ctx_line(11, "function session(req, res) {"),
                diff('-', 12, "const sid = req.cookies.sid", cols, false),
                diff('-', 13, "const row = await db.find(sid)", cols, false),
                diff('+', 12, "const claims = verify(req)", cols, true),
                diff('+', 13, "if (!claims) return reply(401)", cols, true),
                diff('+', 14, "req.user = await user(claims)", cols, true),
                ctx_line(15, "  next()"),
                blank(),
                bullet("Bash(pnpm vitest run src/middleware)"),
                sub("RUN  v1.6.0"),
                vec![
                    s(GREY, "     "),
                    s(GREEN, "✓"),
                    s(GREY, " src/middleware/session.test.ts (9)"),
                ],
                vec![
                    s(GREY, "     "),
                    s(GREEN, "✓"),
                    s(GREY, " src/middleware/limit.test.ts (6)"),
                ],
                txt(GREY, "     Tests  15 passed (15)"),
                blank(),
                vec![
                    s(ORANGE, "✻ Wiring refresh… "),
                    s(GREY, "(47s · esc to interrupt)"),
                ],
            ],
            input_box(cols, "? for shortcuts"),
        ),
        ("api", "claude-2") => (
            vec![
                bullet("Bumped axios to 1.7.2, fixed 1 call."),
                blank(),
                bullet("Update(src/http/client.ts)"),
                sub("Updated client.ts (+2 -1)"),
                diff('-', 3, "import axios from 'axios'", cols, false),
                diff('+', 3, "import axios, { AxiosHeaders }", cols, true),
                diff('+', 18, "headers: new AxiosHeaders(h),", cols, true),
                blank(),
                bullet("Bash(pnpm test)"),
                sub("Test Files  23 passed (23)"),
                txt(GREY, "          Tests  141 passed (141)"),
                txt(GREY, "       Duration  8.42s"),
                blank(),
                bullet("Done. 3 files changed (+9 -6)"),
                txt(GREY, "  - package.json: axios ^1.7.2"),
                txt(GREY, "  - src/http/client.ts: AxiosHeaders"),
                txt(GREY, "  - pnpm-lock.yaml regenerated"),
            ],
            input_box(cols, "? for shortcuts"),
        ),
        ("web", "claude-1") => {
            let mut body = vec![
                bullet("Fixing the checkout total rounding drift."),
                blank(),
                bullet("Update(src/checkout/total.ts)"),
                sub("Updated total.ts (+3 -1)"),
                ctx_line(41, "const sum = items.reduce(add)"),
                diff('-', 42, "return Math.round(sum * tax)/100", cols, false),
                diff('+', 42, "const cents = round(sum*tax)", cols, true),
                diff('+', 43, "// round in cents, never dollars", cols, true),
                diff('+', 44, "return cents / 100", cols, true),
                blank(),
                bullet("Now the checkout tests."),
                blank(),
            ];
            if waiting {
                body.extend(permission_box(cols));
                (body, vec![])
            } else {
                body.push(bullet("Bash(pnpm vitest run src/checkout)"));
                body.push(sub("✓ src/checkout/total.test.ts (14)"));
                body.push(txt(GREY, "     Tests  14 passed (14)"));
                body.push(blank());
                body.push(vec![
                    s(ORANGE, "✻ Checking taxes… "),
                    s(GREY, "(12s · esc to interrupt)"),
                ]);
                (body, input_box(cols, "? for shortcuts"))
            }
        }
        ("web", _) => (
            vec![
                bullet("Rewriting the pricing headline."),
                blank(),
                bullet("Update(src/pages/pricing.tsx)"),
                sub("Updated pricing.tsx (+2 -2)"),
                diff('-', 22, "<h1>Plans and pricing</h1>", cols, false),
                diff('+', 22, "<h1>Pay for what you ship</h1>", cols, true),
                diff('-', 31, "<p>For individuals.</p>", cols, false),
                diff('+', 31, "<p>Solo, unlimited repos.</p>", cols, true),
                blank(),
                bullet("Bash(pnpm lint src/pages/pricing.tsx)"),
                sub("No problems found"),
                blank(),
                vec![
                    s(ORANGE, "✻ Updating plans… "),
                    s(GREY, "(21s · esc to interrupt)"),
                ],
            ],
            input_box(cols, "? for shortcuts"),
        ),
        _ => (
            vec![
                bullet("Staging plan is clean; nothing applied."),
                blank(),
                bullet("Bash(terraform plan -out=tfplan)"),
                sub("Plan: 2 to add, 1 to change."),
                vec![s(GREY, "     "), s(GREEN, "+ aws_s3_bucket.assets_staging")],
                vec![
                    s(GREY, "     "),
                    s(GREEN, "+ aws_s3_bucket_policy.assets_staging"),
                ],
                vec![
                    s(GREY, "     "),
                    s("38;5;180", "~ aws_cloudfront_distribution.cdn"),
                ],
                blank(),
                bullet("Say the word and I'll apply tfplan."),
            ],
            input_box(cols, "? for shortcuts"),
        ),
    }
}

/// The tile's screen: body from the top (older lines scroll off), the input
/// box pinned to the bottom, all with absolute cursor moves so nothing wraps.
fn screen(ws: &str, label: &str, waiting: bool, cols: u16, rows: u16) -> vt100::Parser {
    let mut parser = vt100::Parser::new(rows, cols, 0);
    let (mut body, footer) = body_for(ws, label, waiting, usize::from(cols));
    let avail = usize::from(rows).saturating_sub(footer.len());
    if body.len() > avail {
        body.drain(..body.len() - avail);
    }
    let mut out = String::from("\x1b[2J");
    for (i, line) in body.iter().enumerate() {
        out.push_str(&format!(
            "\x1b[{};1H{}",
            i + 1,
            clip(line, usize::from(cols))
        ));
    }
    for (i, line) in footer.iter().enumerate() {
        let row = usize::from(rows) - footer.len() + i + 1;
        out.push_str(&format!("\x1b[{row};1H{}", clip(line, usize::from(cols))));
    }
    if footer.len() >= 3 {
        // Cursor in the input box (only drawn when engaged).
        out.push_str(&format!("\x1b[{};5H", usize::from(rows) - footer.len() + 2));
    }
    parser.process(out.as_bytes());
    parser
}

// ── driving the real draw path ────────────────────────────────────────────

fn build(scene: Scene, w: u16, h: u16) -> ratatui::buffer::Buffer {
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::with_motion(tx, "http://127.0.0.1:0", true);
    app.store.connection(true);
    app.store.workspaces_fetched(vec![
        workspace("api", "main"),
        workspace("web", "feat/pricing"),
        workspace("infra", "main"),
    ]);
    app.store.sessions_fetched(sessions(scene));

    let list = serde_json::from_value(plugin_listing(scene)).expect("plugin listing");
    app.apply_plugin_listing(list);
    let snap = serde_json::from_value(widget_snapshot(scene)).expect("widget snapshot");
    app.plugins.borrow_mut().apply_widgets(snap, true);

    let focus = match scene {
        Scene::NeedsInput | Scene::Engaged => id("web", "claude-1"),
        Scene::Calm | Scene::Plugins => id("api", "claude-1"),
    };
    app.store.focus_session(&focus);

    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut t = NOW;
    let mut draw = |app: &mut App, t: i64| {
        terminal.draw(|f| app.draw_frame(f, t)).unwrap();
    };
    draw(&mut app, t);
    if scene == Scene::Engaged {
        // The real key: `a` jumps to the blocked tile and engages it.
        app.handle_key(&plain(KeyCode::Char('a')));
        assert_eq!(
            app.store.state().layer,
            KeyLayer::Engaged,
            "`a` should engage"
        );
        assert_eq!(
            app.store.state().focused_session_id.as_deref(),
            Some(focus.as_str())
        );
    }
    draw(&mut app, t);

    // Seat a parser at each tile's inner size, then draw until motion settles.
    let ids = app.store.state().gridded_session_ids.clone();
    app.registry.sync(&ids);
    let tiles = app.layout.as_ref().expect("layout").tiles.clone();
    for (i, sid) in ids.iter().enumerate() {
        let (cols, rows) = tile_inner(tiles[i]);
        let (ws, label) = {
            let st = app.store.state();
            let sess = st.session_by_id(Some(sid)).expect("session");
            (sess.workspace.clone(), sess.label.clone())
        };
        let waiting = app
            .store
            .state()
            .session_by_id(Some(sid))
            .is_some_and(|x| x.needs_input());
        let client =
            crate::pty::TileClient::screen_only(sid, screen(&ws, &label, waiting, cols, rows));
        app.registry.seat_for_render(sid, client);
    }
    for _ in 0..6 {
        t += 700;
        draw(&mut app, t);
    }
    terminal.backend().buffer().clone()
}

#[test]
fn render_brand_frames() {
    if std::env::var_os("RENDER_FRAMES").is_none() {
        return;
    }
    let out = std::env::var_os("RENDER_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/brand/renders")
        });
    std::fs::create_dir_all(&out).unwrap();
    let opts = Options {
        chrome: std::env::var_os("RENDER_CHROME").is_some(),
        ..Options::default()
    };
    for scene in [
        Scene::NeedsInput,
        Scene::Engaged,
        Scene::Calm,
        Scene::Plugins,
    ] {
        for (w, h) in SIZES {
            let name = if (w, h) == SIZES[0] {
                format!("{}.svg", scene.file())
            } else {
                format!("{}-{w}x{h}.svg", scene.file())
            };
            let svg = to_svg(&build(scene, w, h), &opts);
            std::fs::write(out.join(&name), &svg).unwrap();
            println!("wrote {name} ({} bytes)", svg.len());
        }
    }
}
