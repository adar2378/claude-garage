//! The overlay pane: the CHANGES table of the focused workspace.
//!
//! [`render`] and [`PaneState::on_key`] are pure and tested without a
//! terminal. A worker thread collects the model every 5 seconds (and on `r`)
//! so drawing never waits for a disk or a daemon.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, queue, style, terminal};
use garage_plugin_sdk::{Api, Context, strip_hostile};

use crate::config::{self, Config};
use crate::local::{Collector, Row};

/// How often the worker looks at the files again.
pub const REFRESH_EVERY: Duration = Duration::from_secs(5);

/// One workspace as shown: its changes and its warnings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceView {
    pub name: String,
    pub rows: Vec<Row>,
    pub warnings: Vec<String>,
}

/// What one collection pass found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub workspaces: Vec<WorkspaceView>,
    /// Why there are no workspaces (the daemon did not answer).
    pub error: Option<String>,
}

/// Read the workspaces from the daemon, the config from the state folder and
/// the changes from each repo. Never fails: problems become warnings.
pub fn snapshot(
    api: &Api,
    state_dir: &Path,
    home: Option<&Path>,
    collector: &mut Collector,
) -> Snapshot {
    let cfg = config::load(state_dir);
    let list = match api.workspaces() {
        Ok(v) => v,
        Err(e) => {
            return Snapshot {
                workspaces: vec![],
                error: Some(format!("daemon did not answer: {e}")),
            };
        }
    };
    let workspaces = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| Some((w["name"].as_str()?, w["dir"].as_str()?)))
        .map(|(name, dir)| workspace_view(name, Path::new(dir), &cfg, home, collector))
        .collect();
    Snapshot {
        workspaces,
        error: None,
    }
}

fn workspace_view(
    name: &str,
    dir: &Path,
    cfg: &Config,
    home: Option<&Path>,
    collector: &mut Collector,
) -> WorkspaceView {
    let wcfg = cfg.workspace(name);
    let (repos, mut warnings) = config::resolve_repos(dir, &wcfg, home);
    warnings.extend(cfg.warnings.iter().cloned());
    warnings.extend(
        cfg.workspace_warnings
            .get(name)
            .cloned()
            .unwrap_or_default(),
    );
    let mut rows: Vec<Row> = repos
        .iter()
        .flat_map(|r| collector.collect_repo(r))
        .collect();
    rows.sort_by(|a, b| {
        (a.name.as_str(), a.repo.as_str()).cmp(&(b.name.as_str(), b.repo.as_str()))
    });
    WorkspaceView {
        name: name.to_owned(),
        rows,
        warnings,
    }
}

/// What a key asks the loop to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Close,
    Refresh,
}

/// The pane's state: the last snapshot and the cursor.
#[derive(Debug, Default)]
pub struct PaneState {
    pub snapshot: Snapshot,
    pub active: usize,
    pub selected: usize,
    pub detail: bool,
    /// The workspace to start on, until a snapshot has it.
    want: Option<String>,
    have_snapshot: bool,
}

impl PaneState {
    /// Start on `focus` (the run context's `focus.workspace`), else the first.
    pub fn new(focus: Option<&str>) -> Self {
        PaneState {
            want: focus.map(str::to_owned),
            ..PaneState::default()
        }
    }

    /// Take a new snapshot, keeping the same workspace and row where possible.
    pub fn set_snapshot(&mut self, snap: Snapshot) {
        let keep = if self.have_snapshot {
            self.current().map(|w| w.name.clone())
        } else {
            self.want.take()
        };
        self.snapshot = snap;
        self.have_snapshot = true;
        self.active = keep
            .and_then(|n| self.snapshot.workspaces.iter().position(|w| w.name == n))
            .unwrap_or(0);
        self.selected = self.selected.min(self.row_count().saturating_sub(1));
    }

    pub fn current(&self) -> Option<&WorkspaceView> {
        self.snapshot.workspaces.get(self.active)
    }

    fn row_count(&self) -> usize {
        self.current().map_or(0, |w| w.rows.len())
    }

    pub fn on_key(&mut self, k: KeyEvent) -> Action {
        if k.kind == KeyEventKind::Release {
            return Action::None;
        }
        let n = self.row_count();
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Action::Close,
            KeyCode::Esc | KeyCode::Char('q') => Action::Close,
            KeyCode::Char('j') | KeyCode::Down => {
                if n > 0 {
                    self.selected = (self.selected + 1) % n;
                }
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if n > 0 {
                    self.selected = (self.selected + n - 1) % n;
                }
                Action::None
            }
            KeyCode::Char('c') => {
                self.detail = !self.detail;
                Action::None
            }
            KeyCode::Char('r') => Action::Refresh,
            KeyCode::Tab => {
                let count = self.snapshot.workspaces.len();
                if count > 0 {
                    self.active = (self.active + 1) % count;
                    self.selected = 0;
                }
                Action::None
            }
            _ => Action::None,
        }
    }
}

fn fit(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        format!("{s}{}", " ".repeat(width - n))
    } else {
        let cut: String = s.chars().take(width.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn right(s: &str, width: usize) -> String {
    format!("{s:>width$}")
}

/// `4s`, `2m`, `1h`.
pub fn age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

fn longest(values: impl Iterator<Item = usize>, min: usize, max: usize) -> usize {
    values.max().unwrap_or(0).clamp(min, max)
}

/// The pane as lines, at most `cols` wide; the table is windowed to `rows`. `age_secs` is how long ago the
/// model was collected (`None` before the first pass); it goes in the title.
pub fn render(state: &PaneState, age_secs: Option<u64>, cols: usize, rows: usize) -> Vec<String> {
    let cols = cols.max(40);
    let inner = cols - 4; // "│ " ... " │"
    let mut body: Vec<String> = vec![];

    let ws = state.current();
    let name = ws.map_or("-".to_owned(), |w| strip_hostile(&w.name));
    let updated = match age_secs {
        Some(s) => format!("updated {} ago", age(s)),
        None => "loading…".to_owned(),
    };
    let tabs = if state.snapshot.workspaces.len() > 1 {
        format!(
            " ({}/{})",
            state.active + 1,
            state.snapshot.workspaces.len()
        )
    } else {
        String::new()
    };
    let title = format!(" openspec-board · {name}{tabs} · {updated} ");

    if let Some(e) = &state.snapshot.error {
        body.push(format!("! {}", strip_hostile(e)));
    }
    match ws {
        None if state.snapshot.error.is_none() => {
            body.push(if age_secs.is_some() {
                "no workspaces".to_owned()
            } else {
                "reading…".to_owned()
            });
        }
        None => {}
        Some(w) => {
            for warn in &w.warnings {
                body.push(format!("! {}", strip_hostile(warn)));
            }
            if w.rows.is_empty() {
                body.push(format!("no OpenSpec changes in {}", strip_hostile(&w.name)));
            } else {
                table(&mut body, w, state, inner, rows);
            }
        }
    }

    let footer = "j/k move  c detail  r refresh  Tab workspace  Esc close";
    let mut out = vec![format!("┌{}┐", fit_title(&title, cols - 2))];
    out.extend(body.iter().map(|l| format!("│ {} │", fit(l, inner))));
    out.push(format!("│ {} │", fit(footer, inner)));
    out.push(format!("└{}┘", "─".repeat(cols - 2)));
    out
}

fn fit_title(title: &str, width: usize) -> String {
    let t: String = title.chars().take(width).collect();
    format!("{t}{}", "─".repeat(width - t.chars().count()))
}

fn table(body: &mut Vec<String>, w: &WorkspaceView, state: &PaneState, inner: usize, rows: usize) {
    // Rows that fit: the screen less borders, footer, header, warnings and the
    // detail block. The window follows the selection.
    let reserved = 4 + w.warnings.len() + if state.detail { 15 } else { 0 };
    let window = rows.saturating_sub(reserved).max(3);
    let first = if state.selected < window {
        0
    } else {
        state.selected + 1 - window
    };
    let name_w = longest(w.rows.iter().map(|r| r.name.chars().count()), 6, 28);
    let repo_w = longest(w.rows.iter().map(|r| r.repo.chars().count()), 4, 14);
    let where_w = longest(
        w.rows.iter().map(|r| r.places.join(" + ").chars().count()),
        5,
        20,
    );
    let tasks_w = longest(
        w.rows
            .iter()
            .map(|r| format!("{}/{}", r.done, r.total).len()),
        5,
        9,
    );
    let state_w = 8;
    // "▸ " + name + state + repo + where + tasks + next, one space between.
    let used = 2 + name_w + 1 + state_w + 1 + repo_w + 1 + where_w + 1 + tasks_w + 1;
    let next_w = inner.saturating_sub(used).max(8);
    body.push(format!(
        "  {} {} {} {} {} {}",
        fit("CHANGES", name_w),
        fit("STATE", state_w),
        fit("REPO", repo_w),
        fit("WHERE", where_w),
        right("TASKS", tasks_w),
        "NEXT OPEN"
    ));
    for (i, r) in w.rows.iter().enumerate().skip(first).take(window) {
        let mark = if i == state.selected { "▸" } else { " " };
        body.push(format!(
            "{mark} {} {} {} {} {} {}",
            fit(&strip_hostile(&r.name), name_w),
            fit(r.state.as_str(), state_w),
            fit(&strip_hostile(&r.repo), repo_w),
            fit(&r.places.join(" + "), where_w),
            right(&format!("{}/{}", r.done, r.total), tasks_w),
            fit(&strip_hostile(r.next.as_deref().unwrap_or("-")), next_w).trim_end(),
        ));
    }
    if state.detail
        && let Some(r) = w.rows.get(state.selected)
    {
        body.push(String::new());
        if r.open.is_empty() {
            body.push(format!("{}: no open tasks", strip_hostile(&r.name)));
        } else {
            body.push(format!("{}: {} open", strip_hostile(&r.name), r.open.len()));
            for t in r.open.iter().take(12) {
                body.push(format!("  [ ] {}", strip_hostile(t)));
            }
            if r.open.len() > 12 {
                body.push(format!("  … and {} more", r.open.len() - 12));
            }
        }
    }
}

/// Collect on a worker thread: one snapshot now, then every
/// [`REFRESH_EVERY`] or when `()` is sent on the returned sender.
pub fn spawn_worker(
    api: Api,
    state_dir: PathBuf,
    home: Option<PathBuf>,
) -> (Receiver<Snapshot>, Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (force_tx, force_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut collector = Collector::new();
        loop {
            let snap = snapshot(&api, &state_dir, home.as_deref(), &mut collector);
            if tx.send(snap).is_err() {
                return;
            }
            match force_rx.recv_timeout(REFRESH_EVERY) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    (rx, force_tx)
}

/// Raw mode on the alternate screen, undone on drop (also on panic).
struct Screen;

impl Screen {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn draw(lines: &[String]) -> io::Result<()> {
    let mut out = io::stdout();
    queue!(out, terminal::Clear(terminal::ClearType::All))?;
    for (row, line) in lines.iter().enumerate() {
        queue!(out, cursor::MoveTo(0, row as u16), style::Print(line))?;
    }
    out.flush()
}

/// The `ui` entry: run the pane until the user closes it.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = Context::read()?;
    let api = Api::from_env()?;
    let state_dir = garage_plugin_sdk::state_dir()?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut state = PaneState::new(ctx.workspace());
    let (rx, force) = spawn_worker(api, state_dir, home);
    let mut collected: Option<Instant> = None;
    let _screen = Screen::enter()?;
    loop {
        while let Ok(s) = rx.try_recv() {
            state.set_snapshot(s);
            collected = Some(Instant::now());
        }
        let (cols, rows) = terminal::size().map_or((100, 30), |(c, r)| (c as usize, r as usize));
        draw(&render(
            &state,
            collected.map(|t| t.elapsed().as_secs()),
            cols,
            rows,
        ))?;
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(k) = event::read()?
        {
            match state.on_key(k) {
                Action::Close => return Ok(()),
                Action::Refresh => {
                    let _ = force.send(());
                }
                Action::None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::State;
    use garage_plugin_sdk::testing::FakeDaemon;

    fn row(
        name: &str,
        repo: &str,
        state: State,
        done: usize,
        total: usize,
        next: Option<&str>,
    ) -> Row {
        Row {
            name: name.into(),
            repo: repo.into(),
            places: vec!["local".into()],
            state,
            done,
            total,
            next: next.map(str::to_owned),
            open: next.map(|n| vec![n.to_owned()]).unwrap_or_default(),
        }
    }

    fn view(name: &str, rows: Vec<Row>, warnings: &[&str]) -> WorkspaceView {
        WorkspaceView {
            name: name.into(),
            rows,
            warnings: warnings.iter().map(|w| w.to_string()).collect(),
        }
    }

    fn state_with(ws: Vec<WorkspaceView>) -> PaneState {
        let mut s = PaneState::new(None);
        s.set_snapshot(Snapshot {
            workspaces: ws,
            error: None,
        });
        s
    }

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn snapshot_empty_workspace() {
        let s = state_with(vec![view("kowboy", vec![], &[])]);
        let text = render(&s, Some(4), 60, 40).join("\n");
        let expected = "\
┌ openspec-board · kowboy · updated 4s ago ────────────────┐
│ no OpenSpec changes in kowboy                            │
│ j/k move  c detail  r refresh  Tab workspace  Esc close  │
└──────────────────────────────────────────────────────────┘";
        assert_eq!(text, expected);
    }

    #[test]
    fn snapshot_building_and_proposal() {
        let s = state_with(vec![view(
            "kowboy",
            vec![
                row(
                    "billing-v2",
                    "api",
                    State::Proposal,
                    0,
                    14,
                    Some("1.1 schema"),
                ),
                row(
                    "search-ui",
                    "web",
                    State::Building,
                    6,
                    11,
                    Some("2.1 debounce"),
                ),
            ],
            &[],
        )]);
        let text = render(&s, Some(130), 78, 40).join("\n");
        let expected = "\
┌ openspec-board · kowboy · updated 2m ago ──────────────────────────────────┐
│   CHANGES    STATE    REPO WHERE TASKS NEXT OPEN                           │
│ ▸ billing-v2 proposal api  local  0/14 1.1 schema                          │
│   search-ui  building web  local  6/11 2.1 debounce                        │
│ j/k move  c detail  r refresh  Tab workspace  Esc close                    │
└────────────────────────────────────────────────────────────────────────────┘";
        assert_eq!(text, expected);
    }

    #[test]
    fn snapshot_warnings_and_a_loading_title() {
        let mut s = state_with(vec![view(
            "kowboy",
            vec![],
            &[
                "~/code/kowboy/gone is not a git repo",
                "workspace.kowboy.comments.review is not a valid regex; pattern off",
            ],
        )]);
        let expected = "\
┌ openspec-board · kowboy · loading… ──────────────────────┐
│ ! ~/code/kowboy/gone is not a git repo                   │
│ ! workspace.kowboy.comments.review is not a valid regex… │
│ no OpenSpec changes in kowboy                            │
│ j/k move  c detail  r refresh  Tab workspace  Esc close  │
└──────────────────────────────────────────────────────────┘";
        assert_eq!(render(&s, None, 60, 40).join("\n"), expected);
        // A daemon that did not answer is shown, not hidden.
        s.set_snapshot(Snapshot {
            workspaces: vec![],
            error: Some("daemon did not answer: x".into()),
        });
        let text = render(&s, Some(1), 60, 40).join("\n");
        assert!(text.contains("! daemon did not answer: x"));
    }

    #[test]
    fn a_long_table_is_windowed_to_the_screen_and_follows_the_selection() {
        let rows: Vec<Row> = (0..30)
            .map(|i| {
                row(
                    &format!("change-{i:02}"),
                    "api",
                    State::Proposal,
                    0,
                    1,
                    None,
                )
            })
            .collect();
        let mut s = state_with(vec![view("w", rows, &[])]);
        let lines = render(&s, Some(0), 70, 12);
        assert!(lines.len() <= 12, "{}", lines.len());
        assert!(lines.join("\n").contains("change-00"));
        for _ in 0..20 {
            s.on_key(key(KeyCode::Char('j')));
        }
        let lines = render(&s, Some(0), 70, 12);
        assert!(lines.len() <= 12);
        let text = lines.join("\n");
        assert!(text.contains("▸ change-20"), "{text}");
        assert!(!text.contains("change-00"));
        // The footer is always there.
        assert!(text.contains("Esc close"));
    }

    #[test]
    fn every_line_has_the_same_width_and_hostile_names_are_cleaned() {
        let s = state_with(vec![view(
            "k",
            vec![row(
                "evil\u{1b}[2Jname-that-is-far-too-long-for-the-name-column",
                "r",
                State::Building,
                1,
                2,
                Some(
                    "2.1 a very long task text that has to be cut off at the right edge of the pane",
                ),
            )],
            &[],
        )]);
        for cols in [40, 60, 100] {
            let lines = render(&s, Some(0), cols, 40);
            assert!(
                lines.iter().all(|l| l.chars().count() == cols.max(40)),
                "{lines:#?}"
            );
            assert!(!lines.concat().contains('\u{1b}'));
        }
    }

    #[test]
    fn j_k_wrap_and_c_toggles_detail_and_detail_lists_open_tasks() {
        let mut r = row("a", "api", State::Building, 1, 3, Some("1.2 b"));
        r.open = vec!["1.2 b".into(), "1.3 c".into()];
        let mut s = state_with(vec![view(
            "w",
            vec![r, row("b", "api", State::Complete, 1, 1, None)],
            &[],
        )]);
        assert_eq!(s.selected, 0);
        s.on_key(key(KeyCode::Char('k')));
        assert_eq!(s.selected, 1);
        s.on_key(key(KeyCode::Char('j')));
        assert_eq!(s.selected, 0);
        s.on_key(key(KeyCode::Down));
        s.on_key(key(KeyCode::Down));
        assert_eq!(s.selected, 0);
        assert!(!s.detail);
        s.on_key(key(KeyCode::Char('c')));
        let text = render(&s, Some(0), 70, 40).join("\n");
        assert!(
            text.contains("a: 2 open") && text.contains("[ ] 1.3 c"),
            "{text}"
        );
        s.on_key(key(KeyCode::Char('j')));
        assert!(
            render(&s, Some(0), 70, 40)
                .join("\n")
                .contains("b: no open tasks")
        );
    }

    #[test]
    fn keys_ask_for_close_and_refresh_and_tab_cycles_workspaces() {
        let mut s = state_with(vec![
            view("one", vec![row("a", "r", State::Proposal, 0, 0, None)], &[]),
            view("two", vec![], &[]),
        ]);
        assert_eq!(s.on_key(key(KeyCode::Char('r'))), Action::Refresh);
        assert_eq!(s.on_key(key(KeyCode::Esc)), Action::Close);
        assert_eq!(s.on_key(key(KeyCode::Char('q'))), Action::Close);
        assert_eq!(
            s.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Close
        );
        assert_eq!(s.on_key(key(KeyCode::Char('x'))), Action::None);
        assert_eq!(s.current().unwrap().name, "one");
        s.on_key(key(KeyCode::Tab));
        assert_eq!(s.current().unwrap().name, "two");
        s.on_key(key(KeyCode::Tab));
        assert_eq!(s.current().unwrap().name, "one");
        // The title shows which workspace of how many.
        assert!(render(&s, Some(0), 60, 40)[0].contains("one (1/2)"));
        // j/k on an empty workspace do nothing.
        s.on_key(key(KeyCode::Tab));
        s.on_key(key(KeyCode::Char('j')));
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn starts_on_the_focused_workspace_else_the_first_and_keeps_it_across_snapshots() {
        let snap = || Snapshot {
            workspaces: vec![view("one", vec![], &[]), view("two", vec![], &[])],
            error: None,
        };
        let mut s = PaneState::new(Some("two"));
        s.set_snapshot(snap());
        assert_eq!(s.current().unwrap().name, "two");
        s.set_snapshot(snap());
        assert_eq!(s.current().unwrap().name, "two");
        let mut s = PaneState::new(Some("gone"));
        s.set_snapshot(snap());
        assert_eq!(s.current().unwrap().name, "one");
        let mut s = PaneState::new(None);
        s.set_snapshot(snap());
        assert_eq!(s.current().unwrap().name, "one");
    }

    #[test]
    fn age_is_short() {
        assert_eq!([age(4), age(120), age(7300)], ["4s", "2m", "2h"]);
    }

    #[test]
    fn a_pass_reads_workspaces_from_the_daemon_and_changes_from_disk() {
        let home = tempfile::tempdir().unwrap();
        let ws = home.path().join("kowboy");
        let api_repo = ws.join("api");
        std::fs::create_dir_all(api_repo.join(".git")).unwrap();
        std::fs::create_dir_all(api_repo.join("openspec/changes/search-ui")).unwrap();
        std::fs::write(
            api_repo.join("openspec/changes/search-ui/tasks.md"),
            "- [x] 1.1 a\n- [ ] 2.1 b\n",
        )
        .unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::write(
            state.path().join("config.toml"),
            "[workspace.kowboy.comments]\nreview = '('\n",
        )
        .unwrap();
        let dir = ws.to_string_lossy().into_owned();
        let fake = FakeDaemon::start(move |req| match req.path.as_str() {
            "/api/workspaces" => (200, format!(r#"[{{"name":"kowboy","dir":{dir:?}}}]"#)),
            _ => (404, "{}".into()),
        });
        let snap = snapshot(
            &fake.api("openspec-board"),
            state.path(),
            Some(home.path()),
            &mut Collector::new(),
        );
        assert_eq!(snap.error, None);
        let w = &snap.workspaces[0];
        assert_eq!(w.name, "kowboy");
        assert_eq!(
            (w.rows[0].name.as_str(), w.rows[0].done, w.rows[0].total),
            ("search-ui", 1, 2)
        );
        assert!(
            w.warnings[0].contains("comments.review"),
            "{:?}",
            w.warnings
        );
        // Only reads: one GET, nothing written next to the config.
        assert_eq!(fake.log(), ["GET /api/workspaces"]);
        assert!(!state.path().join("cache").exists());
    }

    #[test]
    fn a_daemon_that_does_not_answer_is_an_error_row_not_a_panic() {
        let state = tempfile::tempdir().unwrap();
        let api = Api::new("http://127.0.0.1:1", "openspec-board", "tok");
        let snap = snapshot(&api, state.path(), None, &mut Collector::new());
        assert!(snap.workspaces.is_empty());
        assert!(snap.error.unwrap().starts_with("daemon did not answer"));
    }

    #[test]
    fn the_worker_sends_a_snapshot_at_once_and_another_on_refresh() {
        let state = tempfile::tempdir().unwrap();
        let fake = FakeDaemon::start(|_| (200, "[]".into()));
        let (rx, force) =
            spawn_worker(fake.api("openspec-board"), state.path().to_path_buf(), None);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        force.send(()).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
