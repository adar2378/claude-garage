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
use crate::fetch;
use crate::github::{self, Github, Status};
use crate::linking::{self, LinkedPr};
use crate::local::{Collector, Row};
use crate::remote::{self, Remote};

/// How often the worker looks at the files again.
pub const REFRESH_EVERY: Duration = Duration::from_secs(5);

/// One workspace as shown: its changes and its warnings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceView {
    pub name: String,
    pub rows: Vec<Row>,
    pub warnings: Vec<String>,
    /// `fetch failed 12m ago` when the last fetch of a repo failed.
    pub fetch_note: Option<String>,
    /// Linked pull requests (GitHub on for the workspace), under their change.
    pub prs: Vec<LinkedPr>,
    /// The GitHub status chip for the title: `gh off`, `gh offline 9m`, ...
    pub gh_chip: Option<String>,
    /// Plain information lines (the first-use line for `gh`).
    pub notes: Vec<String>,
    /// A `review` pattern is configured: a PR without a verdict for its head
    /// reads `review pending`.
    pub review_tracked: bool,
}

/// Whether a pass may run `git fetch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMode {
    /// Read the refs as they are.
    Never,
    /// Fetch the repos whose throttle has passed.
    IfDue,
    /// Fetch every repo now (the user pressed `r`).
    Force,
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
    remote: &mut Remote,
    gh: &mut Github,
    mode: FetchMode,
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
    let found: Vec<(&str, &str)> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| Some((w["name"].as_str()?, w["dir"].as_str()?)))
        .collect();
    // One GitHub pass for all workspaces, only when one of them opted in and
    // the interval has passed (or the user pressed `r`); never while the
    // first, cache-only snapshot is drawn.
    let now = Instant::now();
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let wants_github = found.iter().any(|(name, _)| cfg.workspace(name).github);
    let poll = wants_github && mode != FetchMode::Never && gh.due(mode == FetchMode::Force, now);
    if poll {
        gh.begin_pass(now, now_unix);
    }
    let workspaces = found
        .into_iter()
        .map(|(name, dir)| {
            workspace_view(
                name,
                Path::new(dir),
                &cfg,
                home,
                (collector, remote, gh),
                (mode, poll),
            )
        })
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
    (collector, remote, gh): (&mut Collector, &mut Remote, &mut Github),
    (mode, poll): (FetchMode, bool),
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
    let now = Instant::now();
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let (mut local, mut remote_rows) = (vec![], vec![]);
    let mut failed: Option<Instant> = None;
    for repo in &repos {
        if wcfg.fetch && mode != FetchMode::Never {
            let _ = remote.fetch.fetch(
                Path::new("git"),
                repo,
                mode == FetchMode::Force,
                now,
                fetch::TIMEOUT,
            );
        }
        if let Some(at) = remote.fetch.failed_at(repo) {
            failed = Some(failed.map_or(at, |f| f.min(at)));
        }
        local.extend(collector.collect_repo(repo));
        remote_rows.extend(remote.collect_repo(repo, &wcfg.branches, now_unix));
    }
    let rows = remote::merge(local, remote_rows);
    let (mut prs, mut notes) = (vec![], vec![]);
    let gh_chip = if wcfg.github {
        notes.push(
            "gh: this workspace has github = true, so `gh api` (GET only) runs against github.com"
                .to_owned(),
        );
        for repo in &repos {
            let folder = repo_folder(repo);
            let Some(slug) = github::repo_slug(repo) else {
                warnings.push(format!("{folder} has no github.com origin: no PR data"));
                continue;
            };
            let names: Vec<String> = rows
                .iter()
                .filter(|r| r.repo == folder)
                .map(|r| r.name.clone())
                .collect();
            if poll {
                gh.read_repo(&slug, &names, &wcfg.patterns, wcfg.checks);
            }
            if let Some(data) = gh.data.get(&slug) {
                prs.extend(linking::link_all(
                    &folder,
                    &slug,
                    data,
                    &names,
                    wcfg.patterns.task.as_deref(),
                ));
            }
        }
        warnings.extend(gh.status.warning());
        Some(if gh.status == Status::Off {
            "gh …".to_owned()
        } else {
            gh.status.chip(now)
        })
    } else {
        Some("gh off".to_owned())
    };
    WorkspaceView {
        name: name.to_owned(),
        rows,
        warnings,
        fetch_note: failed.map(|at| {
            format!(
                "fetch failed {} ago",
                age(now.saturating_duration_since(at).as_secs())
            )
        }),
        prs,
        gh_chip,
        notes,
        review_tracked: wcfg.patterns.review.is_some(),
    }
}

fn repo_folder(repo: &Path) -> String {
    repo.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string())
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
    let fetch = ws
        .and_then(|w| w.fetch_note.as_deref())
        .map_or(String::new(), |n| format!(" · {}", strip_hostile(n)));
    let chip = ws
        .and_then(|w| w.gh_chip.as_deref())
        .map_or(String::new(), |c| format!(" · {}", strip_hostile(c)));
    let title = format!(" openspec-board · {name}{tabs} · {updated}{fetch}{chip} ");

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
            for note in &w.notes {
                body.push(format!("i {}", strip_hostile(note)));
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

/// The pull requests linked to a change row.
fn prs_of<'a>(w: &'a WorkspaceView, r: &'a Row) -> impl Iterator<Item = &'a LinkedPr> {
    w.prs
        .iter()
        .filter(move |l| l.change == r.name && l.repo == r.repo)
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// One PR under its change: `✗ [1.3] web#88 title · CI FAIL e4f5a6b (head) ·
/// review PASS 5b6c7d8 (STALE: head is 77aa11c) · review pending`.
fn pr_line(l: &LinkedPr, review_tracked: bool) -> String {
    let (pr, c) = (&l.pr, &l.pr.checks);
    let mark = match c.ci.as_ref() {
        Some(ci) if ci.current && ci.status == "FAIL" => "✗",
        Some(ci) if ci.current && ci.status == "PASS" => "✓",
        _ => "◆",
    };
    let mut line = format!("{mark} ");
    if let Some(t) = &l.task {
        line.push_str(&format!("[{t}] "));
    }
    line.push_str(&format!("{}#{}", strip_hostile(&l.repo), pr.number));
    if pr.draft {
        line.push_str(" draft");
    }
    if pr.merged {
        line.push_str(" merged");
    }
    line.push_str(&format!(" {}", strip_hostile(&pr.title)));
    if let Some(ci) = &c.ci {
        let which = if ci.current {
            "head".to_owned()
        } else {
            format!("STALE: head is {}", short(&pr.head_sha))
        };
        line.push_str(&format!(" · CI {} {} ({which})", ci.status, short(&ci.sha)));
    }
    let mut pending = review_tracked && c.review.is_none();
    if let Some(r) = &c.review {
        if r.current {
            line.push_str(&format!(" · review {} {} (head)", r.verdict, short(&r.sha)));
        } else {
            pending = true;
            line.push_str(&format!(
                " · review {} {} (STALE: head is {})",
                r.verdict,
                short(&r.sha),
                short(&pr.head_sha)
            ));
        }
    }
    if pending && !pr.merged {
        line.push_str(" · review pending");
    }
    line
}

/// `6/10`, with `≠` after it when the sources of the change disagree.
fn tasks_cell(r: &Row) -> String {
    format!(
        "{}/{}{}",
        r.done,
        r.total,
        if r.disagree { "≠" } else { "" }
    )
}

fn table(body: &mut Vec<String>, w: &WorkspaceView, state: &PaneState, inner: usize, rows: usize) {
    // Rows that fit: the screen less borders, footer, header, warnings and the
    // detail block. The window follows the selection.
    let reserved = 4 + w.warnings.len() + w.notes.len() + if state.detail { 15 } else { 0 };
    let budget = rows.saturating_sub(reserved).max(3);
    // Each change takes one line plus one per linked PR; the window starts
    // late enough for the selected change to be inside it.
    let lines_of = |i: usize| 1 + prs_of(w, &w.rows[i]).count();
    let mut first = 0;
    while first < state.selected && (first..=state.selected).map(lines_of).sum::<usize>() > budget {
        first += 1;
    }
    let name_w = longest(w.rows.iter().map(|r| r.name.chars().count()), 6, 28);
    let repo_w = longest(w.rows.iter().map(|r| r.repo.chars().count()), 4, 14);
    let where_w = longest(
        w.rows.iter().map(|r| r.places.join(" + ").chars().count()),
        5,
        34,
    );
    let tasks_w = longest(w.rows.iter().map(|r| tasks_cell(r).chars().count()), 5, 9);
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
    let mut used_lines = 0;
    for (i, r) in w.rows.iter().enumerate().skip(first) {
        if used_lines + lines_of(i) > budget && used_lines > 0 {
            break;
        }
        used_lines += lines_of(i);
        let mark = if i == state.selected { "▸" } else { " " };
        body.push(format!(
            "{mark} {} {} {} {} {} {}",
            fit(&strip_hostile(&r.name), name_w),
            fit(r.state.as_str(), state_w),
            fit(&strip_hostile(&r.repo), repo_w),
            fit(&strip_hostile(&r.places.join(" + ")), where_w),
            right(&tasks_cell(r), tasks_w),
            fit(&strip_hostile(r.next.as_deref().unwrap_or("-")), next_w).trim_end(),
        ));
        let linked: Vec<&LinkedPr> = prs_of(w, r).collect();
        for (k, l) in linked.iter().enumerate() {
            let branch = if k + 1 == linked.len() { "└" } else { "├" };
            body.push(format!("  {branch} {}", pr_line(l, w.review_tracked)));
        }
    }
    if state.detail
        && let Some(r) = w.rows.get(state.selected)
    {
        body.push(String::new());
        if r.disagree {
            body.push("sources differ: the most advanced one is shown".to_owned());
        }
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
        let (mut collector, mut remote) = (Collector::new(), Remote::new());
        let mut gh = Github::new("gh", state_dir.clone());
        let mut mode = FetchMode::IfDue;
        let mut first = true;
        loop {
            // Show what is already known before a fetch can take its time.
            if first || mode == FetchMode::Force {
                let snap = snapshot(
                    &api,
                    &state_dir,
                    home.as_deref(),
                    &mut collector,
                    &mut remote,
                    &mut gh,
                    FetchMode::Never,
                );
                if tx.send(snap).is_err() {
                    return;
                }
            }
            first = false;
            let snap = snapshot(
                &api,
                &state_dir,
                home.as_deref(),
                &mut collector,
                &mut remote,
                &mut gh,
                mode,
            );
            if tx.send(snap).is_err() {
                return;
            }
            mode = match force_rx.recv_timeout(REFRESH_EVERY) {
                Ok(()) => FetchMode::Force,
                Err(RecvTimeoutError::Timeout) => FetchMode::IfDue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
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
            disagree: false,
        }
    }

    fn view(name: &str, rows: Vec<Row>, warnings: &[&str]) -> WorkspaceView {
        WorkspaceView {
            name: name.into(),
            rows,
            warnings: warnings.iter().map(|w| w.to_string()).collect(),
            fetch_note: None,
            ..WorkspaceView::default()
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
    fn a_remote_row_shows_its_branch_the_differing_mark_and_the_fetch_note() {
        let mut r = row("x", "api", State::Building, 6, 10, Some("2.1 c"));
        r.places = vec!["local".into(), "origin/feat/x".into()];
        r.disagree = true;
        let mut v = view("kowboy", vec![r], &[]);
        v.fetch_note = Some("fetch failed 12m ago".into());
        let text = render(&state_with(vec![v]), Some(3), 110, 20).join("\n");
        assert!(text.contains("local + origin/feat/x"), "{text}");
        assert!(text.contains("6/10≠"), "{text}");
        assert!(text.contains("fetch failed 12m ago"), "{text}");
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
            &mut Remote::new(),
            &mut Github::new("/nonexistent/gh", state.path().to_path_buf()),
            FetchMode::Never,
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
        let snap = snapshot(
            &api,
            state.path(),
            None,
            &mut Collector::new(),
            &mut Remote::new(),
            &mut Github::new("/nonexistent/gh", state.path().to_path_buf()),
            FetchMode::Never,
        );
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

    use crate::fakegh::FakeGh;
    use crate::github::{Checks, CiResult, PrData, PrFile, ReviewResult};

    /// A workspace `kowboy` with one repo `api` holding the change
    /// `search-ui`, a github.com `origin`, and the config text given.
    fn github_fixture(config: &str) -> (tempfile::TempDir, tempfile::TempDir, FakeDaemon) {
        let home = tempfile::tempdir().unwrap();
        let ws = home.path().join("kowboy");
        let api_repo = ws.join("api");
        std::fs::create_dir_all(api_repo.join("openspec/changes/search-ui")).unwrap();
        std::fs::write(
            api_repo.join("openspec/changes/search-ui/tasks.md"),
            "- [x] 1.1 a\n- [ ] 1.3 debounce\n",
        )
        .unwrap();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&api_repo)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["remote", "add", "origin", "https://github.com/o/r.git"]);
        let state = tempfile::tempdir().unwrap();
        std::fs::write(state.path().join("config.toml"), config).unwrap();
        let dir = ws.to_string_lossy().into_owned();
        let fake = FakeDaemon::start(move |req| match req.path.as_str() {
            "/api/workspaces" => (200, format!(r#"[{{"name":"kowboy","dir":{dir:?}}}]"#)),
            _ => (404, "{}".into()),
        });
        (home, state, fake)
    }

    fn pass(home: &Path, state: &Path, fake: &FakeDaemon, gh: &mut Github) -> WorkspaceView {
        let snap = snapshot(
            &fake.api("openspec-board"),
            state,
            Some(home),
            &mut Collector::new(),
            &mut Remote::new(),
            gh,
            FetchMode::IfDue,
        );
        snap.workspaces.into_iter().next().unwrap()
    }

    #[test]
    fn github_off_runs_no_gh_at_all_and_the_status_reads_gh_off() {
        let (home, state, fake) = github_fixture("");
        let gh = FakeGh::new();
        let mut g = Github::new(gh.program(), state.path().to_path_buf());
        let w = pass(home.path(), state.path(), &fake, &mut g);
        assert!(gh.log().is_empty(), "{:?}", gh.log());
        assert_eq!(w.gh_chip.as_deref(), Some("gh off"));
        assert!(w.prs.is_empty());
        let mut s = PaneState::new(None);
        s.set_snapshot(Snapshot {
            workspaces: vec![w],
            error: None,
        });
        let text = render(&s, Some(1), 100, 20).join("\n");
        assert!(text.contains("· gh off"), "{text}");
        assert!(!state.path().join("cache").exists());
    }

    const OPEN: &str = "repos/o/r/pulls?state=open&per_page=100";
    const CLOSED: &str = "repos/o/r/pulls?state=closed&sort=updated&direction=desc&per_page=100";
    const CONFIG: &str = "[workspace.kowboy]\ngithub = true\nfetch = false\n\
        [workspace.kowboy.comments]\n\
        ci = 'Claude CI \\((?P<mode>\\w+)\\): (?P<status>PASS|FAIL) (?P<sha>[0-9a-f]{7,40})'\n\
        review = 'Claude review: (?P<verdict>[A-Za-z _-]+?) (?P<sha>[0-9a-f]{7,40})'\n";

    #[test]
    fn a_pass_with_github_on_shows_the_linked_pr_with_its_marks_and_only_gets() {
        let (home, state, fake) = github_fixture(CONFIG);
        let gh = FakeGh::new();
        gh.ok(
            OPEN,
            "\"a\"",
            "X-Ratelimit-Remaining: 4000\r\n",
            r#"[{"number":88,"html_url":"https://github.com/o/r/pull/88","title":"[search-ui 1.3] debounce","draft":false,"updated_at":"2026-11-01T00:00:00Z","head":{"ref":"claude/x","sha":"e4f5a6b1234"}}]"#,
        );
        gh.ok(CLOSED, "\"b\"", "", "[]");
        gh.ok("repos/o/r/pulls/88/files?per_page=100", "\"c\"", "", "[]");
        gh.ok(
            "repos/o/r/issues/88/comments?per_page=100",
            "\"d\"",
            "",
            r#"[{"body":"Claude CI (full): FAIL e4f5a6b"},{"body":"Claude review: PASS 5b6c7d8"}]"#,
        );
        let mut g = Github::new(gh.program(), state.path().to_path_buf());
        let w = pass(home.path(), state.path(), &fake, &mut g);
        assert_eq!(w.prs.len(), 1);
        assert_eq!(w.prs[0].task.as_deref(), Some("1.3"));
        let mut s = PaneState::new(None);
        s.set_snapshot(Snapshot {
            workspaces: vec![w],
            error: None,
        });
        let text = render(&s, Some(1), 150, 30).join("\n");
        assert!(text.contains("· gh ok"), "{text}");
        assert!(
            text.contains(
                "└ ✗ [1.3] api#88 [search-ui 1.3] debounce · CI FAIL e4f5a6b (head) · \
                 review PASS 5b6c7d8 (STALE: head is e4f5a6b) · review pending"
            ),
            "{text}"
        );
        // Every call was a GET.
        assert!(gh.log().len() >= 4);
        assert!(
            gh.log()
                .iter()
                .all(|l| l.starts_with("api -i --method GET "))
        );
        // The cache holds private text: mode 0700.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(state.path().join("cache")).unwrap();
            assert_eq!(m.permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn a_missing_gh_is_a_warning_row_and_the_changes_are_still_listed() {
        let (home, state, fake) = github_fixture(CONFIG);
        let mut g = Github::new("/nonexistent/gh", state.path().to_path_buf());
        let w = pass(home.path(), state.path(), &fake, &mut g);
        assert_eq!(w.rows.len(), 1);
        assert!(
            w.warnings.iter().any(|x| x.contains("gh was not found")),
            "{:?}",
            w.warnings
        );
        assert_eq!(w.gh_chip.as_deref(), Some("gh off"));
    }

    #[test]
    fn pr_rows_show_draft_merged_and_a_current_review_and_a_windowed_table_keeps_the_selection() {
        let pr = |n: u64, draft: bool, merged: bool, checks: Checks| LinkedPr {
            repo: "api".into(),
            slug: "o/r".into(),
            change: "search-ui".into(),
            task: None,
            pr: PrData {
                number: n,
                url: String::new(),
                title: "t".into(),
                draft,
                head_ref: String::new(),
                head_sha: "9c8d7e6aa".into(),
                updated_at: String::new(),
                merged,
                files: vec![PrFile {
                    path: "x".into(),
                    status: "added".into(),
                }],
                checks,
            },
        };
        let ok = Checks {
            ci: Some(CiResult {
                status: "PASS".into(),
                sha: "9c8d7e6".into(),
                mode: None,
                current: true,
            }),
            review: Some(ReviewResult {
                verdict: "APPROVED".into(),
                sha: "9c8d7e6".into(),
                current: true,
            }),
            owner_needed: false,
        };
        let mut v = view(
            "kowboy",
            vec![row(
                "search-ui",
                "api",
                State::Building,
                1,
                2,
                Some("1.3 a"),
            )],
            &[],
        );
        v.review_tracked = true;
        v.prs = vec![
            pr(1, true, false, ok.clone()),
            pr(2, false, true, Checks::default()),
        ];
        let text = render(&state_with(vec![v]), Some(1), 120, 20).join("\n");
        assert!(
            text.contains(
                "├ ✓ api#1 draft t · CI PASS 9c8d7e6 (head) · review APPROVED 9c8d7e6 (head)"
            ),
            "{text}"
        );
        assert!(text.contains("└ ◆ api#2 merged t"), "{text}");
        assert!(!text.contains("api#2 merged t · review pending"), "{text}");
    }
}
