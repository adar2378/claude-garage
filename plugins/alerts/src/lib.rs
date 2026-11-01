//! alerts: a chime, and a desktop banner when no wall is in view, when a
//! session needs you or finishes a long turn.
//!
//! The plugin runs once per `session.status_changed` event and does the work
//! itself (it is a local process): it plays the sound and shows the banner.
//!
//! * A session that enters `needs-input` is an alert at once.
//! * A session that goes `working` -> `done` is an alert only when it had been
//!   working for [`MIN_WORKING_MS`] or more. The time it entered `working` is
//!   kept in the plugin's storage (the event only carries the new state's
//!   time).
//! * Alerts within [`COALESCE`] of each other make one sound and one banner.
//!   The first run to queue an alert becomes the "leader": it waits, then
//!   delivers everything queued meanwhile. The others only queue and exit.
//!   A leader that died leaves a stale mark ([`LEADER_STALE_MS`]) and the next
//!   run takes over.
//!
//! The sound and the banner go through [`Out`], and time through [`Clock`], so
//! tests never play a sound, show a banner or wait.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use garage_plugin_sdk::{
    Api, Context, Error, StateLock, Value, json, read_state_in, write_state_in,
};
use serde::{Deserialize, Serialize};

/// A turn must have run this long for "finished" to be worth an alert.
pub const MIN_WORKING_MS: i64 = 20_000;
/// Alerts this close together are one sound and one banner.
pub const COALESCE: Duration = Duration::from_secs(2);
/// A queue mark older than this belongs to a leader that died.
pub const LEADER_STALE_MS: i64 = 15_000;
/// A `working` mark older than this is forgotten (a session that never ended).
const WORKING_KEEP_MS: i64 = 6 * 60 * 60 * 1000;
/// A `done` this soon after `idle` still belongs to the turn that just ended.
pub const IDLE_THEN_DONE_MS: i64 = 10_000;
/// How long one player or notifier may run.
const PROGRAM_TIMEOUT: Duration = Duration::from_secs(5);

const SETTINGS: &str = "settings.json";
const STATE: &str = "state.json";

pub const CHIME_NEEDS_INPUT: &[u8] = include_bytes!("../assets/chime-needs-input.wav");
pub const CHIME_DONE: &[u8] = include_bytes!("../assets/chime-done.wav");

// ---------------------------------------------------------------- settings

fn on() -> bool {
    true
}

/// The options of `claude-garage alerts`. All on by default; a missing or
/// unreadable file is the defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "on")]
    pub sound: bool,
    #[serde(default = "on")]
    pub needs_input: bool,
    #[serde(default = "on")]
    pub done: bool,
    #[serde(default = "on")]
    pub banners: bool,
    /// Bundle id of the terminal a clicked banner brings forward; `None` is
    /// "work it out from what the wall reported".
    #[serde(default)]
    pub terminal: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound: true,
            needs_input: true,
            done: true,
            banners: true,
            terminal: None,
        }
    }
}

impl Settings {
    pub fn load(dir: &Path) -> Settings {
        read_state_in(dir, SETTINGS).unwrap_or_default()
    }
}

// ------------------------------------------------------------------- items

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    NeedsInput,
    Done,
}

/// One session that earned an alert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub kind: Kind,
    /// The session id (`garage/<workspace>/<label>`).
    pub id: String,
    pub label: String,
    pub workspace: String,
    /// Claude's own message (needs-input), when the daemon had one.
    pub message: Option<String>,
    /// How long the turn ran (done).
    pub secs: Option<u64>,
    /// A wall was in view when it happened.
    pub visible: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// Session id -> epoch ms it entered `working`.
    #[serde(default)]
    working: HashMap<String, i64>,
    /// Session id -> (epoch ms it entered `working`, epoch ms it went idle).
    /// The daemon's poller can see a turn end (idle) a moment before the Stop
    /// hook says done, so a `done` right after `idle` still counts.
    #[serde(default)]
    ended: HashMap<String, (i64, i64)>,
    /// Alerts waiting for the leader.
    #[serde(default)]
    pending: Vec<Item>,
    /// Epoch ms the current leader started waiting.
    #[serde(default)]
    leader_at: Option<i64>,
}

fn id_parts(id: &str) -> (String, String) {
    let mut parts = id.split('/').skip(1);
    let workspace = parts.next().unwrap_or("").to_owned();
    let label = parts.next().unwrap_or("").to_owned();
    if label.is_empty() {
        (workspace, id.to_owned())
    } else {
        (workspace, label)
    }
}

/// Claude's message made safe for a banner: one line, no control characters, cut.
fn clean(text: &str) -> String {
    let one: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 200 {
        let cut: String = one.chars().take(199).collect();
        format!("{cut}…")
    } else {
        one
    }
}

// ------------------------------------------------------------------- compose

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Chime {
    NeedsInput,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alert {
    pub chime: Option<Chime>,
    /// `(title, body)`.
    pub banner: Option<(String, String)>,
    /// What a click on the banner jumps to.
    pub open: Option<Open>,
}

/// Where a clicked banner leads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Open {
    /// That session.
    Session(String),
    /// The session that has waited longest for the user.
    Blocked,
}

fn human(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn names(items: &[&Item]) -> String {
    let all: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    clean(&all.join(", "))
}

/// What to play and show for the queued items under the current options.
pub fn compose(items: &[Item], s: &Settings) -> Alert {
    let needs: Vec<&Item> = items
        .iter()
        .filter(|i| i.kind == Kind::NeedsInput && s.needs_input)
        .collect();
    let done: Vec<&Item> = items
        .iter()
        .filter(|i| i.kind == Kind::Done && s.done)
        .collect();
    if needs.is_empty() && done.is_empty() {
        return Alert {
            chime: None,
            banner: None,
            open: None,
        };
    }
    let chime = s.sound.then_some(if needs.is_empty() {
        Chime::Done
    } else {
        Chime::NeedsInput
    });
    let seen = needs.iter().chain(done.iter()).any(|i| i.visible);
    if !s.banners || seen {
        return Alert {
            chime,
            banner: None,
            open: None,
        };
    }
    let (mut title, mut body) = match needs.as_slice() {
        [] => (String::new(), String::new()),
        [one] => (
            format!("{} needs you", one.label),
            one.message
                .clone()
                .unwrap_or_else(|| format!("Waiting for your input in {}", one.workspace)),
        ),
        many => (format!("{} sessions need you", many.len()), names(many)),
    };
    match done.as_slice() {
        [] => {}
        rest if !needs.is_empty() => {
            body = format!("{body} · finished: {}", names(rest));
        }
        [one] => {
            title = format!("{} finished", one.label);
            body = match one.secs {
                Some(s) => format!("Done after {} in {}", human(s), one.workspace),
                None => format!("Done in {}", one.workspace),
            };
        }
        many => {
            title = format!("{} sessions finished", many.len());
            body = names(many);
        }
    }
    let open = match (needs.as_slice(), done.as_slice()) {
        ([one], []) | ([], [one]) => Some(Open::Session(one.id.clone())),
        ([], _) => None,
        _ => Some(Open::Blocked),
    };
    Alert {
        chime,
        banner: Some((title, body)),
        open,
    }
}

// ----------------------------------------------------------------- output

/// Where a sound and a banner go. Failures are the implementation's to
/// swallow: an alert that cannot be shown is not an error.
pub trait Out: Sync {
    fn play(&self, wav: &Path);
    fn banner(&self, title: &str, body: &str, click: &Click);
}

/// Time, injectable so tests neither wait nor read the clock.
pub trait Clock {
    fn now_ms(&self) -> i64;
    fn sleep(&self, d: Duration);
}

pub struct RealClock;

impl Clock for RealClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Write the chimes into `<dir>/sounds/` when missing or different; returns
/// `(needs-input, done)` paths.
pub fn ensure_chimes(dir: &Path) -> Result<(PathBuf, PathBuf), Error> {
    let sounds = dir.join("sounds");
    std::fs::create_dir_all(&sounds)
        .map_err(|e| Error::Context(format!("{}: {e}", sounds.display())))?;
    let put = |name: &str, bytes: &[u8]| -> Result<PathBuf, Error> {
        let path = sounds.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(bytes) {
            std::fs::write(&path, bytes)
                .map_err(|e| Error::Context(format!("{}: {e}", path.display())))?;
        }
        Ok(path)
    };
    Ok((
        put("chime-needs-input.wav", CHIME_NEEDS_INPUT)?,
        put("chime-done.wav", CHIME_DONE)?,
    ))
}

/// Play and show `alert`. The sound and the banner start together.
pub fn deliver(alert: &Alert, dir: &Path, out: &dyn Out, host: &Host, settings: &Settings) {
    let wav = alert.chime.and_then(|c| {
        let (needs, done) = ensure_chimes(dir).ok()?;
        Some(match c {
            Chime::NeedsInput => needs,
            Chime::Done => done,
        })
    });
    std::thread::scope(|s| {
        if let Some(wav) = &wav {
            s.spawn(|| out.play(wav));
        }
        if let Some((title, body)) = &alert.banner {
            let click = click_for(alert.open.as_ref(), host, settings);
            s.spawn(move || out.banner(title, body, &click));
        }
    });
}

// ------------------------------------------------------------------ events

fn lock(dir: &Path) -> Result<StateLock, Error> {
    StateLock::acquire(
        dir,
        "alerts",
        Duration::from_secs(10),
        Duration::from_secs(5),
    )
}

fn read_state(dir: &Path) -> State {
    read_state_in(dir, STATE).unwrap_or_default()
}

/// Claude's message for `id` from `GET /api/sessions`, if the daemon has one.
fn message_of(api: &Api, id: &str) -> Option<String> {
    let sessions = api.sessions().ok()?;
    sessions
        .as_array()?
        .iter()
        .find(|s| s.get("id").and_then(Value::as_str) == Some(id))?
        .get("message")?
        .as_str()
        .map(clean)
        .filter(|m| !m.is_empty())
}

/// Handle one `session.status_changed` run.
pub fn handle_event(
    ctx: &Context,
    api: &Api,
    dir: &Path,
    out: &dyn Out,
    clock: &dyn Clock,
) -> Result<(), Error> {
    if ctx.event_name() != Some("session.status_changed") {
        return Ok(());
    }
    let data = ctx.event_data();
    let text = |k: &str| data.get(k).and_then(Value::as_str).unwrap_or("");
    let (id, from, to) = (text("id"), text("from"), text("to"));
    if id.is_empty() {
        return Ok(());
    }
    let at = data
        .get("since")
        .and_then(Value::as_i64)
        .or_else(|| ctx.raw().pointer("/event/at").and_then(Value::as_i64))
        .unwrap_or_else(|| clock.now_ms());
    let visible = ctx
        .raw()
        .pointer("/ui/visible")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let settings = Settings::load(dir);
    let (workspace, label) = id_parts(id);

    // Step 1: note when work started, and decide whether this is an alert.
    let item = {
        let Some(_hold) = held(lock(dir))? else {
            return Ok(());
        };
        let mut state = read_state(dir);
        let started = state.working.remove(id);
        let now = clock.now_ms();
        state.working.retain(|_, t| now - *t < WORKING_KEEP_MS);
        state
            .ended
            .retain(|_, (_, idle)| now - *idle < WORKING_KEEP_MS);
        let ended = state.ended.remove(id);
        // When did the turn that this `done` ends start?
        let turn_start = match from {
            "working" => started,
            "idle" => ended
                .filter(|(_, idle)| (0..=IDLE_THEN_DONE_MS).contains(&(at - idle)))
                .map(|(t, _)| t),
            _ => None,
        };
        let item = match to {
            "working" => {
                state.working.insert(id.to_owned(), at);
                None
            }
            "idle" => {
                if let (Some(t), "working") = (started, from) {
                    state.ended.insert(id.to_owned(), (t, at));
                }
                None
            }
            "needs-input" if settings.needs_input => Some(Item {
                kind: Kind::NeedsInput,
                id: id.to_owned(),
                label,
                workspace,
                message: None,
                secs: None,
                visible,
            }),
            "done" if settings.done => turn_start
                .map(|t| at - t)
                .filter(|ms| *ms >= MIN_WORKING_MS)
                .map(|ms| Item {
                    kind: Kind::Done,
                    id: id.to_owned(),
                    label,
                    workspace,
                    message: None,
                    secs: Some((ms / 1000) as u64),
                    visible,
                }),
            _ => None,
        };
        write_state_in(dir, STATE, &state)?;
        item
    };
    let Some(mut item) = item else {
        return Ok(());
    };
    if item.kind == Kind::NeedsInput {
        item.message = message_of(api, id);
    }

    // Step 2: queue it; the first to queue waits for the others.
    let leader = {
        let Some(_hold) = held(lock(dir))? else {
            return Ok(());
        };
        let mut state = read_state(dir);
        let now = clock.now_ms();
        let live = state.leader_at.is_some_and(|t| now - t < LEADER_STALE_MS);
        state.pending.push(item);
        if !live {
            state.leader_at = Some(now);
        }
        write_state_in(dir, STATE, &state)?;
        !live
    };
    if !leader {
        return Ok(());
    }

    // Step 3: the leader waits, then delivers everything queued.
    clock.sleep(COALESCE);
    let items = {
        let _hold = lock(dir)?;
        let mut state = read_state(dir);
        state.leader_at = None;
        let items = std::mem::take(&mut state.pending);
        write_state_in(dir, STATE, &state)?;
        items
    };
    let settings = Settings::load(dir);
    deliver(
        &compose(&items, &settings),
        dir,
        out,
        &Host::from_context(ctx),
        &settings,
    );
    Ok(())
}

/// A lock another run holds past the wait means "skip", not "fail".
fn held(lock: Result<StateLock, Error>) -> Result<Option<StateLock>, Error> {
    match lock {
        Ok(l) => Ok(Some(l)),
        Err(Error::LockHeld(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

// --------------------------------------------------------------------- cli

const OPTIONS: [&str; 4] = ["sound", "needs-input", "done", "banners"];

fn show(s: &Settings) -> String {
    let onoff = |b: bool| if b { "on" } else { "off" };
    format!(
        "alerts\n  sound        {}   chime when a session needs you or finishes\n  needs-input  {}   alert when a session needs you\n  done         {}   alert when a session finishes a long turn\n  banners      {}   desktop banner when no wall is in view\n\nchange: claude-garage alerts <sound|needs-input|done|banners> <on|off>\n        claude-garage alerts terminal <bundle-id|auto>\ntry:    claude-garage alerts test\n",
        onoff(s.sound),
        onoff(s.needs_input),
        onoff(s.done),
        onoff(s.banners),
    ) + &format!(
        "  terminal     {}   app a clicked banner brings forward (macOS)\n",
        s.terminal.as_deref().unwrap_or("auto")
    )
}

const USAGE: &str = "usage: claude-garage alerts [sound|needs-input|done|banners on|off | terminal <bundle-id|auto> | open <session-id>|--blocked | test]\n";

/// `claude-garage alerts ...`. `Ok` is printed to stdout, `Err` to stderr with
/// exit code 2.
pub fn cli(
    words: &[String],
    dir: &Path,
    out: &dyn Out,
    clock: &dyn Clock,
) -> Result<String, String> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => Ok(show(&Settings::load(dir))),
        ["test"] => {
            let (needs, done) = ensure_chimes(dir).map_err(|e| format!("alerts: {e}\n"))?;
            out.play(&needs);
            clock.sleep(Duration::from_millis(600));
            out.play(&done);
            out.banner(
                "Sample needs you",
                "This is how an alert looks",
                &click_for(None, &Host::default(), &Settings::load(dir)),
            );
            Ok("played both chimes and showed a sample banner\n".to_owned())
        }
        ["terminal", value] => {
            let terminal = match *value {
                "auto" => None,
                v if valid_bundle_id(v) => Some(v.to_owned()),
                v => {
                    return Err(format!(
                        "alerts: {v:?} is not a bundle id (like com.mitchellh.ghostty) or auto\n{USAGE}"
                    ));
                }
            };
            let _hold = lock(dir).map_err(|e| format!("alerts: {e}\n"))?;
            let mut s = Settings::load(dir);
            s.terminal = terminal;
            write_state_in(dir, SETTINGS, &s).map_err(|e| format!("alerts: {e}\n"))?;
            Ok(show(&s))
        }
        [name, value] if OPTIONS.contains(name) => {
            let value = match *value {
                "on" => true,
                "off" => false,
                _ => {
                    return Err(format!(
                        "alerts: expected on or off, not {value:?}\n{USAGE}"
                    ));
                }
            };
            let _hold = lock(dir).map_err(|e| format!("alerts: {e}\n"))?;
            let mut s = Settings::load(dir);
            match *name {
                "sound" => s.sound = value,
                "needs-input" => s.needs_input = value,
                "done" => s.done = value,
                _ => s.banners = value,
            }
            write_state_in(dir, SETTINGS, &s).map_err(|e| format!("alerts: {e}\n"))?;
            Ok(show(&s))
        }
        _ => Err(USAGE.to_owned()),
    }
}

// ------------------------------------------------------------- clicking a banner

/// `^garage/[a-z0-9-]+/[a-z0-9-]+$`
pub fn valid_session_id(id: &str) -> bool {
    let mut parts = id.split('/');
    let name = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
    };
    parts.next() == Some("garage")
        && name(parts.next())
        && name(parts.next())
        && parts.next().is_none()
}

/// `^[A-Za-z0-9.-]+$`, with a dot: a macOS bundle id.
pub fn valid_bundle_id(s: &str) -> bool {
    s.contains('.')
        && !s.is_empty()
        && s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// The bundle id of the terminal the wall reported (`ui.terminal`): a
/// `TERM_PROGRAM` name we know, else a value that already is a bundle id.
pub fn terminal_bundle(reported: &str) -> Option<String> {
    let known = match reported {
        "ghostty" => "com.mitchellh.ghostty",
        "iTerm.app" => "com.googlecode.iterm2",
        "Apple_Terminal" => "com.apple.Terminal",
        "WezTerm" => "com.github.wez.wezterm",
        "kitty" => "net.kovidgoyal.kitty",
        other => return valid_bundle_id(other).then(|| other.to_owned()),
    };
    Some(known.to_owned())
}

/// What the run context says about this machine: the terminal, and how to run
/// `claude-garage` again.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Host {
    pub terminal: Option<String>,
    pub bin: Option<String>,
    pub port: Option<u16>,
    pub dir: Option<String>,
    pub tmux_socket: Option<String>,
}

impl Host {
    pub fn from_context(ctx: &Context) -> Host {
        let raw = ctx.raw();
        let text = |p: &str| {
            raw.pointer(p)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Host {
            terminal: text("/ui/terminal"),
            bin: text("/garage/bin"),
            port: raw
                .pointer("/garage/port")
                .and_then(Value::as_u64)
                .and_then(|p| u16::try_from(p).ok()),
            dir: text("/garage/dir"),
            tmux_socket: text("/garage/tmux_socket"),
        }
    }
}

/// What terminal-notifier is told about a banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Click {
    /// Banners with the same group replace each other.
    pub group: String,
    /// Bundle id of the app to bring forward.
    pub activate: Option<String>,
    /// A shell command to run on click.
    pub execute: Option<String>,
}

/// `'...'` with every `'` inside written as `'\''`. `None` when the text has
/// a control character: nothing like that belongs in a path or a name.
pub fn sh_quote(s: &str) -> Option<String> {
    if s.chars().any(char::is_control) {
        return None;
    }
    Some(format!("'{}'", s.replace('\'', "'\\''")))
}

/// The shell command a click runs, built only from validated parts: the
/// binary (an absolute path), the port / dir / tmux socket the daemon runs
/// with, and a validated session id. Never from Claude's text.
pub fn execute_command(open: &Open, host: &Host) -> Option<String> {
    let bin = host.bin.as_deref().filter(|b| b.starts_with('/'))?;
    let target = match open {
        Open::Session(id) if valid_session_id(id) => sh_quote(id)?,
        Open::Session(_) => return None,
        Open::Blocked => "--blocked".to_owned(),
    };
    let mut cmd = String::new();
    if let Some(port) = host.port {
        cmd += &format!("GARAGE_PORT={} ", sh_quote(&port.to_string())?);
    }
    for (name, value) in [
        ("GARAGE_DIR", &host.dir),
        ("GARAGE_TMUX_SOCKET", &host.tmux_socket),
    ] {
        if let Some(v) = value {
            cmd += &format!("{name}={} ", sh_quote(v)?);
        }
    }
    Some(format!("{cmd}{} alerts open {target}", sh_quote(bin)?))
}

/// The click behaviour of a banner.
pub fn click_for(open: Option<&Open>, host: &Host, settings: &Settings) -> Click {
    let group = match open {
        Some(Open::Session(id)) if valid_session_id(id) => {
            format!("garage-alerts-{}", id.replace('/', "-"))
        }
        _ => "garage-alerts-many".to_owned(),
    };
    let activate = settings
        .terminal
        .clone()
        .filter(|t| valid_bundle_id(t))
        .or_else(|| host.terminal.as_deref().and_then(terminal_bundle));
    Click {
        group,
        activate,
        execute: open.and_then(|o| execute_command(o, host)),
    }
}

/// `claude-garage alerts open <session-id>|--blocked`: ask the wall to focus
/// that session, or the one waiting longest. With no wall running (or no
/// daemon) there is nothing to do: the banner already brought the terminal
/// forward, so those are not errors. Only a bad argument is.
pub fn open(words: &[String], api: &Api) -> Result<(), String> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let body = match words.as_slice() {
        ["--blocked"] => json!({"action": "jump.blocked"}),
        [id] if valid_session_id(id) => {
            json!({"action": "session.focus", "target": {"session": id}})
        }
        _ => return Err(USAGE.to_owned()),
    };
    let _ = api.post("/api/plugins/ui/action", &body);
    Ok(())
}

// ---------------------------------------------------------- real player/notifier

/// A program to run: its argv and the environment it gets on top of ours.
///
/// Anything untrusted (Claude's message, a path) travels in `env` or as a plain
/// argument, never inside a script string: the scripts below are fixed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Cmd {
    fn new(argv: &[&str]) -> Cmd {
        Cmd {
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
            env: Vec::new(),
        }
    }
    fn with_env(mut self, key: &str, value: &str) -> Cmd {
        self.env.push((key.to_owned(), value.to_owned()));
        self
    }
}

const ENV_WAV: &str = "GARAGE_ALERT_WAV";
const ENV_TITLE: &str = "GARAGE_ALERT_TITLE";
const ENV_BODY: &str = "GARAGE_ALERT_BODY";

/// Fixed PowerShell: the path comes from the environment.
const SOUND_SCRIPT_WINDOWS: &str =
    "(New-Object Media.SoundPlayer $env:GARAGE_ALERT_WAV).PlaySync()";

/// Fixed PowerShell: the toast text comes from the environment.
const TOAST_SCRIPT_WINDOWS: &str = "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
$t = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
$n = $t.GetElementsByTagName('text'); \
$n.Item(0).AppendChild($t.CreateTextNode($env:GARAGE_ALERT_TITLE)) > $null; \
$n.Item(1).AppendChild($t.CreateTextNode($env:GARAGE_ALERT_BODY)) > $null; \
$app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe'; \
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($t))";

/// Fixed AppleScript: the text arrives as `run` arguments (item 2 and 3; item 1
/// is a fixed word so that a title or body starting with `-` is never taken
/// for an option by osascript).
const NOTIFY_SCRIPT_MACOS: [&str; 3] = [
    "on run argv",
    "display notification (item 3 of argv) with title (item 2 of argv)",
    "end run",
];

/// A value for `terminal-notifier -title/-message`: one that starts with `-`
/// could be read as the next option, so it gets a leading space.
fn dash_safe(s: &str) -> String {
    if s.starts_with('-') {
        format!(" {s}")
    } else {
        s.to_owned()
    }
}

/// Programs to play `wav`, in the order to try them.
pub fn sound_commands(os: &str, wav: &Path) -> Vec<Cmd> {
    let p = wav.to_string_lossy().into_owned();
    match os {
        "macos" => vec![Cmd::new(&["afplay", &p])],
        "linux" => vec![Cmd::new(&["paplay", &p]), Cmd::new(&["aplay", "-q", &p])],
        "windows" => vec![powershell(SOUND_SCRIPT_WINDOWS).with_env(ENV_WAV, &p)],
        _ => Vec::new(),
    }
}

/// Programs to show a banner. `terminal_notifier` says it is installed.
pub fn banner_commands(
    os: &str,
    terminal_notifier: bool,
    title: &str,
    body: &str,
    click: &Click,
) -> Vec<Cmd> {
    match os {
        "macos" if terminal_notifier => {
            let mut c = Cmd::new(&[
                "terminal-notifier",
                "-title",
                &dash_safe(title),
                "-message",
                &dash_safe(body),
                "-group",
                &click.group,
            ]);
            if let Some(app) = &click.activate {
                c.argv.extend(["-activate".to_owned(), app.clone()]);
            }
            if let Some(cmd) = &click.execute {
                c.argv.extend(["-execute".to_owned(), cmd.clone()]);
            }
            vec![c]
        }
        "macos" => {
            let mut argv = vec!["osascript".to_owned()];
            for line in NOTIFY_SCRIPT_MACOS {
                argv.extend(["-e".to_owned(), line.to_owned()]);
            }
            argv.extend(["alerts".to_owned(), title.to_owned(), body.to_owned()]);
            vec![Cmd {
                argv,
                env: Vec::new(),
            }]
        }
        "linux" => vec![Cmd::new(&[
            "notify-send",
            "--app-name=claude-garage",
            "--",
            title,
            body,
        ])],
        "windows" => vec![
            powershell(TOAST_SCRIPT_WINDOWS)
                .with_env(ENV_TITLE, title)
                .with_env(ENV_BODY, body),
        ],
        _ => Vec::new(),
    }
}

/// `powershell.exe` running the fixed `script` through `-EncodedCommand`.
fn powershell(script: &str) -> Cmd {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    Cmd {
        argv: vec![
            "powershell.exe".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-EncodedCommand".into(),
            base64(&utf16),
        ],
        env: Vec::new(),
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Run `argv` to the end, killing it after [`PROGRAM_TIMEOUT`]. `false` when
/// the program does not exist (so the next candidate is tried).
fn run(c: &Cmd) -> bool {
    let Some((program, args)) = c.argv.split_first() else {
        return false;
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(c.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return true,
            Ok(None) if start.elapsed() > PROGRAM_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn on_path(program: &str) -> bool {
    let extra = ["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from);
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(extra)
        .any(|d| d.join(program).is_file())
}

/// The real thing: plays with the OS player, shows with the OS notifier.
pub struct SystemOut;

impl Out for SystemOut {
    fn play(&self, wav: &Path) {
        for c in sound_commands(std::env::consts::OS, wav) {
            if run(&c) {
                return;
            }
        }
    }

    fn banner(&self, title: &str, body: &str, click: &Click) {
        let os = std::env::consts::OS;
        let tn = os == "macos" && on_path("terminal-notifier");
        for c in banner_commands(os, tn, title, body, click) {
            if run(&c) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::fake::FakeDaemon;
    use garage_plugin_sdk::json;
    use std::cell::{Cell, RefCell};
    use std::sync::Mutex;

    // ---- fakes: nothing here plays a sound, shows a banner or waits ----

    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>, Mutex<Vec<Click>>);

    impl Rec {
        fn all(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Out for Rec {
        fn play(&self, wav: &Path) {
            let name = wav.file_name().unwrap().to_string_lossy().into_owned();
            self.0.lock().unwrap().push(format!("play {name}"));
        }
        fn banner(&self, title: &str, body: &str, click: &Click) {
            self.0
                .lock()
                .unwrap()
                .push(format!("banner {title} | {body}"));
            self.1.lock().unwrap().push(click.clone());
        }
    }

    struct Fake {
        now: Cell<i64>,
        slept: RefCell<Vec<Duration>>,
        /// Runs once, inside the leader's wait: another event arriving.
        during_wait: RefCell<Option<Box<dyn FnOnce()>>>,
    }

    impl Fake {
        fn at(now: i64) -> Fake {
            Fake {
                now: Cell::new(now),
                slept: RefCell::default(),
                during_wait: RefCell::default(),
            }
        }
    }

    impl Clock for Fake {
        fn now_ms(&self) -> i64 {
            self.now.get()
        }
        fn sleep(&self, d: Duration) {
            self.slept.borrow_mut().push(d);
            if let Some(f) = self.during_wait.borrow_mut().take() {
                f();
            }
            self.now.set(self.now.get() + d.as_millis() as i64);
        }
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("alerts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ctx(id: &str, from: &str, to: &str, since: i64, visible: bool) -> Context {
        Context::from_value(json!({
            "event": {"name": "session.status_changed", "at": since,
                "data": {"id": id, "dir": "", "workspace": "ws", "from": from, "to": to, "since": since}},
            "ui": {"slots": {}, "visible": visible},
        }))
    }

    fn api(d: &FakeDaemon) -> Api {
        Api::new(&d.base, "alerts", "tok")
    }

    fn sessions(d: &FakeDaemon) {
        d.route(
            "GET",
            "/api/sessions",
            200,
            r#"[{"id":"garage/ws/fix","status":"needs-input","message":"Claude needs your permission to use Bash"},
                {"id":"garage/ws/other","status":"needs-input","message":null}]"#,
        );
    }

    fn set(dir: &Path, f: impl FnOnce(&mut Settings)) {
        let mut s = Settings::load(dir);
        f(&mut s);
        write_state_in(dir, SETTINGS, &s).unwrap();
    }

    // ---- the two events ----

    #[test]
    fn needs_input_plays_the_rising_chime_and_shows_the_message_when_no_wall_is_in_view() {
        let (dir, d, out, clock) = (
            tmp("n1"),
            FakeDaemon::start(),
            Rec::default(),
            Fake::at(1_000),
        );
        sessions(&d);
        handle_event(
            &ctx("garage/ws/fix", "working", "needs-input", 1_000, false),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        // The sound and the banner start together, so their order is not fixed.
        let mut got = out.all();
        got.sort();
        assert_eq!(
            got,
            [
                "banner fix needs you | Claude needs your permission to use Bash",
                "play chime-needs-input.wav"
            ]
        );
        assert_eq!(
            *clock.slept.borrow(),
            [COALESCE],
            "waited the coalescing window once"
        );
    }

    #[test]
    fn a_visible_wall_gets_the_sound_only() {
        let (dir, d, out, clock) = (tmp("vis"), FakeDaemon::start(), Rec::default(), Fake::at(0));
        sessions(&d);
        handle_event(
            &ctx("garage/ws/fix", "working", "needs-input", 0, true),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        assert_eq!(out.all(), ["play chime-needs-input.wav"]);
    }

    #[test]
    fn without_a_daemon_message_the_banner_says_where() {
        let (dir, d, out, clock) = (tmp("gen"), FakeDaemon::start(), Rec::default(), Fake::at(0));
        sessions(&d);
        handle_event(
            &ctx("garage/ws/other", "idle", "needs-input", 0, false),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        assert!(
            out.all()
                .contains(&"banner other needs you | Waiting for your input in ws".to_string()),
            "{:?}",
            out.all()
        );
        // and an unreachable daemon is the same
        let out = Rec::default();
        handle_event(
            &ctx("garage/ws/x", "idle", "needs-input", 0, false),
            &Api::new("http://127.0.0.1:1", "alerts", "t"),
            &tmp("gen2"),
            &out,
            &Fake::at(0),
        )
        .unwrap();
        assert!(
            out.all()
                .contains(&"banner x needs you | Waiting for your input in ws".to_string())
        );
    }

    #[test]
    fn done_alerts_only_after_twenty_seconds_of_work() {
        let d = FakeDaemon::start();
        for (secs, alerts) in [(5, false), (19, false), (20, true), (95, true)] {
            let (dir, out) = (tmp(&format!("d{secs}")), Rec::default());
            let clock = Fake::at(0);
            let id = "garage/ws/fix";
            handle_event(
                &ctx(id, "idle", "working", 1_000, false),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            assert!(out.all().is_empty(), "starting work says nothing");
            handle_event(
                &ctx(id, "working", "done", 1_000 + secs * 1000, false),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            if alerts {
                let want = format!(
                    "banner fix finished | Done after {} in ws",
                    human(secs as u64)
                );
                assert!(
                    out.all().contains(&"play chime-done.wav".to_string()),
                    "{secs}s: {:?}",
                    out.all()
                );
                assert!(out.all().contains(&want), "{secs}s: {:?}", out.all());
            } else {
                assert!(
                    out.all().is_empty(),
                    "{secs}s must stay quiet: {:?}",
                    out.all()
                );
            }
        }
    }

    #[test]
    fn done_with_no_recorded_start_or_not_from_working_stays_quiet() {
        let (dir, d, out, clock) = (
            tmp("noval"),
            FakeDaemon::start(),
            Rec::default(),
            Fake::at(0),
        );
        handle_event(
            &ctx("garage/ws/a", "working", "done", 99_000, false),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        handle_event(
            &ctx("garage/ws/a", "idle", "working", 1_000, false),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        handle_event(
            &ctx("garage/ws/a", "needs-input", "done", 99_000, false),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        assert!(out.all().is_empty(), "{:?}", out.all());
        assert!(
            read_state(&dir).working.is_empty(),
            "the start mark is used up"
        );
    }

    #[test]
    fn a_long_turn_is_timed_from_its_latest_return_to_working() {
        let (dir, d, out, clock) = (
            tmp("latest"),
            FakeDaemon::start(),
            Rec::default(),
            Fake::at(0),
        );
        sessions(&d);
        let id = "garage/ws/fix";
        handle_event(
            &ctx(id, "idle", "working", 0, true),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        handle_event(
            &ctx(id, "working", "needs-input", 60_000, true),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        handle_event(
            &ctx(id, "needs-input", "working", 70_000, true),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        let before = out.all().len();
        handle_event(
            &ctx(id, "working", "done", 75_000, true),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        assert_eq!(out.all().len(), before, "5 s since the last working: quiet");
    }

    #[test]
    fn a_poller_idle_just_before_the_done_hook_still_alerts() {
        let d = FakeDaemon::start();
        // (seconds worked, seconds idle before done, chime expected)
        for (worked, gap, chime) in [
            (30, 1, true),
            (30, 10, true),
            (30, 15, false),
            (5, 1, false),
        ] {
            let (dir, out, clock) = (
                tmp(&format!("idle{worked}-{gap}")),
                Rec::default(),
                Fake::at(0),
            );
            let id = "garage/ws/fix";
            let ev = |from: &str, to: &str, at: i64| ctx(id, from, to, at, false);
            handle_event(&ev("idle", "working", 0), &api(&d), &dir, &out, &clock).unwrap();
            let idle_at = worked * 1000;
            handle_event(
                &ev("working", "idle", idle_at),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            assert!(out.all().is_empty(), "idle is quiet");
            handle_event(
                &ev("idle", "done", idle_at + gap * 1000),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            assert_eq!(
                out.all().contains(&"play chime-done.wav".to_string()),
                chime,
                "worked {worked}s, done {gap}s after idle: {:?}",
                out.all()
            );
            // the mark is used up: a second done does not repeat it
            let n = out.all().len();
            handle_event(
                &ev("idle", "done", idle_at + gap * 1000 + 500),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            assert_eq!(out.all().len(), n);
        }
    }

    #[test]
    fn idle_then_working_again_or_needs_input_forgets_the_ended_turn() {
        let (dir, d, out, clock) = (
            tmp("idle-forget"),
            FakeDaemon::start(),
            Rec::default(),
            Fake::at(0),
        );
        sessions(&d);
        let id = "garage/ws/fix";
        let ev = |from: &str, to: &str, at: i64| ctx(id, from, to, at, true);
        handle_event(&ev("idle", "working", 0), &api(&d), &dir, &out, &clock).unwrap();
        handle_event(&ev("working", "idle", 30_000), &api(&d), &dir, &out, &clock).unwrap();
        handle_event(&ev("idle", "working", 31_000), &api(&d), &dir, &out, &clock).unwrap();
        assert!(read_state(&dir).ended.is_empty());
        let n = out.all().len();
        handle_event(&ev("idle", "done", 32_000), &api(&d), &dir, &out, &clock).unwrap();
        assert_eq!(
            out.all().len(),
            n,
            "idle->done after a new working has no ended turn"
        );
    }

    // ---- coalescing ----

    #[test]
    fn events_inside_the_window_make_one_sound_and_one_banner() {
        let (dir, d, out) = (tmp("co"), FakeDaemon::start(), Rec::default());
        sessions(&d);
        let clock = Fake::at(10_000);
        let (d2, dir2) = (api(&d), dir.clone());
        // A second and a third session need you while the first one waits.
        let hook_out = Rec::default();
        *clock.during_wait.borrow_mut() = Some(Box::new(move || {
            for id in ["garage/ws/other", "garage/ws/third"] {
                let c = Fake::at(10_500);
                handle_event(
                    &ctx(id, "working", "needs-input", 10_500, false),
                    &d2,
                    &dir2,
                    &hook_out,
                    &c,
                )
                .unwrap();
                assert!(hook_out.all().is_empty(), "a follower never plays");
                assert!(c.slept.borrow().is_empty(), "a follower never waits");
            }
        }));
        handle_event(
            &ctx("garage/ws/fix", "working", "needs-input", 10_000, false),
            &api(&d),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        let all = out.all();
        assert_eq!(
            all.iter().filter(|l| l.starts_with("play ")).count(),
            1,
            "{all:?}"
        );
        assert_eq!(
            all.iter().filter(|l| l.starts_with("banner ")).count(),
            1,
            "{all:?}"
        );
        assert!(
            all.contains(&"banner 3 sessions need you | fix, other, third".to_string()),
            "{all:?}"
        );
        let s = read_state(&dir);
        assert!(
            s.pending.is_empty() && s.leader_at.is_none(),
            "queue is empty again"
        );
    }

    #[test]
    fn a_leader_that_died_is_replaced_but_a_live_one_is_not() {
        let (d, out) = (FakeDaemon::start(), Rec::default());
        let queued = Item {
            kind: Kind::NeedsInput,
            id: "garage/ws/old".into(),
            label: "old".into(),
            workspace: "ws".into(),
            message: None,
            secs: None,
            visible: false,
        };
        let mk = |tag: &str, leader_at: i64| {
            let dir = tmp(tag);
            write_state_in(
                &dir,
                STATE,
                &State {
                    pending: vec![queued.clone()],
                    leader_at: Some(leader_at),
                    ..Default::default()
                },
            )
            .unwrap();
            dir
        };
        // 1 s old: someone is waiting, so this run only queues.
        let live = mk("live", 9_000);
        handle_event(
            &ctx("garage/ws/new", "idle", "needs-input", 10_000, false),
            &api(&d),
            &live,
            &out,
            &Fake::at(10_000),
        )
        .unwrap();
        assert!(out.all().is_empty());
        assert_eq!(read_state(&live).pending.len(), 2);
        // 20 s old: the leader is gone; this run delivers both.
        let dead = mk("dead", -10_000);
        handle_event(
            &ctx("garage/ws/new", "idle", "needs-input", 10_000, false),
            &api(&d),
            &dead,
            &out,
            &Fake::at(10_000),
        )
        .unwrap();
        assert!(
            out.all()
                .contains(&"banner 2 sessions need you | old, new".to_string()),
            "{:?}",
            out.all()
        );
    }

    // ---- options ----

    #[test]
    fn each_option_suppresses_exactly_its_part() {
        let d = FakeDaemon::start();
        sessions(&d);
        let run = |tag: &str, f: &dyn Fn(&mut Settings)| {
            let (dir, out, clock) = (tmp(tag), Rec::default(), Fake::at(0));
            set(&dir, |s| f(s));
            handle_event(
                &ctx("garage/ws/fix", "idle", "working", 0, false),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            handle_event(
                &ctx("garage/ws/fix", "working", "needs-input", 1_000, false),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            handle_event(
                &ctx("garage/ws/fix", "needs-input", "working", 2_000, false),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            handle_event(
                &ctx("garage/ws/fix", "working", "done", 60_000, false),
                &api(&d),
                &dir,
                &out,
                &clock,
            )
            .unwrap();
            out.all()
        };
        let all = run("o-all", &|_| {});
        assert_eq!(all.len(), 4, "{all:?}");
        let no_sound = run("o-sound", &|s| s.sound = false);
        assert!(
            no_sound.iter().all(|l| l.starts_with("banner ")) && no_sound.len() == 2,
            "{no_sound:?}"
        );
        let no_banner = run("o-banner", &|s| s.banners = false);
        assert!(
            no_banner.iter().all(|l| l.starts_with("play ")) && no_banner.len() == 2,
            "{no_banner:?}"
        );
        let no_needs = run("o-needs", &|s| s.needs_input = false);
        assert!(
            no_needs
                .iter()
                .all(|l| l.contains("finished") || l == "play chime-done.wav")
                && no_needs.len() == 2,
            "{no_needs:?}"
        );
        let no_done = run("o-done", &|s| s.done = false);
        assert!(
            no_done
                .iter()
                .all(|l| l.contains("needs you") || l == "play chime-needs-input.wav")
                && no_done.len() == 2,
            "{no_done:?}"
        );
        assert!(
            run("o-none", &|s| {
                s.needs_input = false;
                s.done = false
            })
            .is_empty()
        );
    }

    #[test]
    fn missing_or_damaged_files_are_defaults_and_never_an_error() {
        let dir = tmp("dmg");
        assert_eq!(Settings::load(&dir), Settings::default());
        std::fs::write(dir.join(SETTINGS), "{not json").unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
        std::fs::write(dir.join(SETTINGS), r#"{"sound": false}"#).unwrap();
        assert_eq!(
            Settings::load(&dir),
            Settings {
                sound: false,
                ..Default::default()
            }
        );
        std::fs::write(dir.join(STATE), "\u{0}garbage").unwrap();
        let (d, out) = (FakeDaemon::start(), Rec::default());
        handle_event(
            &ctx("garage/ws/a", "idle", "needs-input", 0, false),
            &api(&d),
            &dir,
            &out,
            &Fake::at(0),
        )
        .unwrap();
        assert!(!out.all().is_empty());
    }

    #[test]
    fn other_events_are_ignored() {
        let (dir, d, out) = (tmp("oth"), FakeDaemon::start(), Rec::default());
        let c = Context::from_value(
            json!({"event": {"name": "session.turn", "at": 1, "data": {"id": "garage/ws/a"}}}),
        );
        handle_event(&c, &api(&d), &dir, &out, &Fake::at(0)).unwrap();
        assert!(out.all().is_empty() && !dir.join(STATE).exists());
    }

    // ---- the cli ----

    #[test]
    fn the_cli_shows_changes_and_persists_options() {
        let (dir, out, clock) = (tmp("cli"), Rec::default(), Fake::at(0));
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let shown = cli(&w(&[]), &dir, &out, &clock).unwrap();
        for line in [
            "sound        on",
            "needs-input  on",
            "done         on",
            "banners      on",
        ] {
            assert!(shown.contains(line), "{shown}");
        }
        let after = cli(&w(&["done", "off"]), &dir, &out, &clock).unwrap();
        assert!(
            after.contains("done         off") && after.contains("sound        on"),
            "{after}"
        );
        cli(&w(&["banners", "off"]), &dir, &out, &clock).unwrap();
        cli(&w(&["needs-input", "off"]), &dir, &out, &clock).unwrap();
        cli(&w(&["sound", "off"]), &dir, &out, &clock).unwrap();
        assert_eq!(
            Settings::load(&dir),
            Settings {
                sound: false,
                needs_input: false,
                done: false,
                banners: false,
                terminal: None
            }
        );
        cli(&w(&["sound", "on"]), &dir, &out, &clock).unwrap();
        assert!(Settings::load(&dir).sound);
        for bad in [
            w(&["sound"]),
            w(&["sound", "maybe"]),
            w(&["volume", "on"]),
            w(&["test", "x"]),
        ] {
            assert!(cli(&bad, &dir, &out, &clock).is_err(), "{bad:?}");
        }
        assert!(out.all().is_empty(), "options never play anything");
    }

    #[test]
    fn the_cli_test_plays_both_chimes_and_a_sample_banner() {
        let (dir, out, clock) = (tmp("clit"), Rec::default(), Fake::at(0));
        cli(&["test".to_string()], &dir, &out, &clock).unwrap();
        let all = out.all();
        assert_eq!(
            all.iter()
                .filter(|l| l.as_str() == "play chime-needs-input.wav")
                .count(),
            1
        );
        assert_eq!(
            all.iter()
                .filter(|l| l.as_str() == "play chime-done.wav")
                .count(),
            1
        );
        assert_eq!(all.iter().filter(|l| l.starts_with("banner ")).count(), 1);
    }

    // ---- compose, goldens ----

    fn item(kind: Kind, label: &str, msg: Option<&str>, secs: Option<u64>, visible: bool) -> Item {
        Item {
            kind,
            id: format!("garage/app/{label}"),
            label: label.into(),
            workspace: "app".into(),
            message: msg.map(Into::into),
            secs,
            visible,
        }
    }

    #[test]
    fn compose_matches_the_golden_alerts() {
        let n = |l: &str, m: Option<&str>| item(Kind::NeedsInput, l, m, None, false);
        let f = |l: &str, s: u64| item(Kind::Done, l, None, Some(s), false);
        let off = Settings {
            sound: false,
            ..Default::default()
        };
        let cases: Vec<(&str, Vec<Item>, Settings)> = vec![
            (
                "one_needs_input_with_message",
                vec![n("fix", Some("Claude needs your permission to use Bash"))],
                Settings::default(),
            ),
            (
                "one_needs_input_generic",
                vec![n("fix", None)],
                Settings::default(),
            ),
            (
                "three_need_you",
                vec![n("a", None), n("b", None), n("c", None)],
                Settings::default(),
            ),
            ("one_done", vec![f("fix", 95)], Settings::default()),
            (
                "two_done",
                vec![f("a", 30), f("b", 3700)],
                Settings::default(),
            ),
            (
                "needs_and_done",
                vec![n("a", Some("ok?")), f("b", 40)],
                Settings::default(),
            ),
            (
                "wall_visible",
                vec![item(Kind::NeedsInput, "fix", None, None, true)],
                Settings::default(),
            ),
            ("sound_off", vec![n("fix", None)], off),
            (
                "banners_off",
                vec![f("fix", 25)],
                Settings {
                    banners: false,
                    ..Default::default()
                },
            ),
            (
                "message_is_one_clean_line",
                vec![n("fix", Some(&clean("line one\n\tline\u{7} two")))],
                Settings::default(),
            ),
        ];
        let got: Value = cases
            .iter()
            .map(|(name, items, s)| {
                (
                    (*name).to_string(),
                    serde_json::to_value(compose(items, s)).unwrap(),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("golden/compose.json");
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        }
        let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn long_or_messy_messages_are_cut_to_one_line() {
        let m = clean(&format!("a\n\nb {}", "x".repeat(400)));
        assert!(m.starts_with("a b xxx") && m.chars().count() == 200 && m.ends_with('…'));
    }

    // ---- the real player and notifier: argv only, never run ----

    fn no_click() -> Click {
        click_for(None, &Host::default(), &Settings::default())
    }

    fn argv(c: &[Cmd]) -> Vec<Vec<&str>> {
        c.iter()
            .map(|c| c.argv.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn players_and_notifiers_per_os() {
        let wav = Path::new("/s/chime.wav");
        assert_eq!(
            argv(&sound_commands("macos", wav)),
            [["afplay", "/s/chime.wav"]]
        );
        assert_eq!(
            argv(&sound_commands("linux", wav)),
            [
                vec!["paplay", "/s/chime.wav"],
                vec!["aplay", "-q", "/s/chime.wav"]
            ]
        );
        assert!(sound_commands("freebsd", wav).is_empty());
        assert_eq!(
            argv(&banner_commands("macos", true, "t", "b", &no_click())),
            [[
                "terminal-notifier",
                "-title",
                "t",
                "-message",
                "b",
                "-group",
                "garage-alerts-many"
            ]]
        );
        assert_eq!(
            argv(&banner_commands("linux", false, "-t", "b", &no_click())),
            [vec![
                "notify-send",
                "--app-name=claude-garage",
                "--",
                "-t",
                "b"
            ]]
        );
        assert!(banner_commands("freebsd", false, "t", "b", &no_click()).is_empty());
        // terminal-notifier must not read a leading dash as an option
        let tn = &banner_commands("macos", true, "-evil", "--also", &no_click())[0].argv;
        assert_eq!((tn[2].as_str(), tn[4].as_str()), (" -evil", " --also"));
    }

    /// Text an attacker could put in a notification message: every quote
    /// PowerShell and AppleScript know, escapes, newlines, a non-BMP char.
    const HOSTILE: &str =
        "x'\u{2018}\u{2019}\u{201A}\u{201B}\"`$(calc);\\\n\r\u{0}\u{1F600} -e do shell script";

    #[test]
    fn untrusted_text_never_enters_a_script() {
        let plain = banner_commands("windows", false, "title", "body", &no_click());
        let evil = banner_commands("windows", false, HOSTILE, HOSTILE, &no_click());
        assert_eq!(
            plain[0].argv, evil[0].argv,
            "the script does not depend on the text"
        );
        assert!(evil[0].argv.iter().all(|a| !a.contains("calc")));
        assert!(
            evil[0]
                .env
                .contains(&(ENV_TITLE.to_owned(), HOSTILE.to_owned()))
        );
        assert!(
            evil[0]
                .env
                .contains(&(ENV_BODY.to_owned(), HOSTILE.to_owned()))
        );
        assert!(
            !TOAST_SCRIPT_WINDOWS.contains(HOSTILE)
                && TOAST_SCRIPT_WINDOWS.contains("$env:GARAGE_ALERT_BODY")
        );

        // a path with every quote in it, too
        let p = Path::new("C:\\o\u{2019}neil\\a'b.wav");
        let (a, b) = (
            &sound_commands("windows", p)[0],
            &sound_commands("windows", Path::new("C:\\x.wav"))[0],
        );
        assert_eq!(a.argv, b.argv);
        assert_eq!(
            a.env,
            [(ENV_WAV.to_owned(), p.to_string_lossy().into_owned())]
        );

        // osascript: the -e scripts are fixed; the text is only a trailing argument
        let plain = &banner_commands("macos", false, "title", "body", &no_click())[0].argv;
        let evil = &banner_commands("macos", false, HOSTILE, HOSTILE, &no_click())[0].argv;
        let n = plain.len();
        assert_eq!(plain[..n - 2], evil[..n - 2]);
        assert_eq!(evil[n - 2..], [HOSTILE, HOSTILE]);
        assert_eq!(
            evil[n - 3],
            "alerts",
            "a fixed word precedes the text, so a leading dash is no option"
        );
        for (i, a) in evil.iter().enumerate() {
            if evil[i.saturating_sub(1)] == "-e" {
                assert!(!a.contains("calc"), "script line {a:?}");
            }
        }
    }

    #[test]
    fn windows_commands_use_encoded_command() {
        let c = &sound_commands("windows", Path::new("C:\\x.wav"))[0];
        assert_eq!(
            &c.argv[..4],
            [
                "powershell.exe",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand"
            ]
        );
        // "ab" in UTF-16LE is 61 00 62 00 -> YQBiAA==
        assert_eq!(
            base64(
                &"ab"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>()
            ),
            "YQBiAA=="
        );
        assert_eq!(
            banner_commands("windows", false, "t", "b", &no_click())[0].argv[0],
            "powershell.exe"
        );
    }

    // ---- clicking a banner ----

    fn host() -> Host {
        Host {
            terminal: Some("ghostty".into()),
            bin: Some("/usr/local/bin/claude-garage".into()),
            port: Some(4747),
            dir: Some("/Users/me/.garage".into()),
            tmux_socket: None,
        }
    }

    fn tn(click: &Click) -> Vec<String> {
        banner_commands("macos", true, "t", "b", click)[0]
            .argv
            .clone()
    }

    #[test]
    fn a_click_banner_has_group_activate_and_execute() {
        let one = click_for(
            Some(&Open::Session("garage/app/fix".into())),
            &host(),
            &Settings::default(),
        );
        assert_eq!(
            tn(&one),
            [
                "terminal-notifier",
                "-title",
                "t",
                "-message",
                "b",
                "-group",
                "garage-alerts-garage-app-fix",
                "-activate",
                "com.mitchellh.ghostty",
                "-execute",
                "GARAGE_PORT='4747' GARAGE_DIR='/Users/me/.garage' '/usr/local/bin/claude-garage' alerts open 'garage/app/fix'",
            ]
        );
        let many = click_for(Some(&Open::Blocked), &host(), &Settings::default());
        let argv = tn(&many);
        assert_eq!(argv[6], "garage-alerts-many");
        assert_eq!(
            argv.last().unwrap(),
            "GARAGE_PORT='4747' GARAGE_DIR='/Users/me/.garage' '/usr/local/bin/claude-garage' alerts open --blocked"
        );
        // osascript has no click: it keeps the plain script
        assert!(
            banner_commands("macos", false, "t", "b", &one)[0]
                .argv
                .iter()
                .all(|a| !a.contains("alerts open"))
        );
        // a tmux socket is passed along
        let h = Host {
            tmux_socket: Some("test sock".into()),
            ..host()
        };
        assert!(
            execute_command(&Open::Blocked, &h)
                .unwrap()
                .contains("GARAGE_TMUX_SOCKET='test sock' ")
        );
    }

    #[test]
    fn what_a_click_is_built_from_and_what_it_refuses() {
        // Claude's message goes through compose into the banner, never into the command.
        let evil = "'; touch /tmp/pwned; echo '";
        let items = [Item {
            kind: Kind::NeedsInput,
            id: "garage/app/fix".into(),
            label: "fix".into(),
            workspace: "app".into(),
            message: Some(evil.into()),
            secs: None,
            visible: false,
        }];
        let alert = compose(&items, &Settings::default());
        assert!(alert.banner.as_ref().unwrap().1.contains("touch"));
        let click = click_for(alert.open.as_ref(), &host(), &Settings::default());
        assert!(!click.execute.as_ref().unwrap().contains("touch"));
        // an id that is not a garage session id gets no -execute (and its own group)
        for bad in [
            "garage/app",
            "garage/a/b/c",
            "x/app/fix",
            "garage/App/fix",
            "garage/a b/fix",
            "garage/app/fix'; rm -rf ~; '",
            "garage//fix",
            "",
        ] {
            assert!(!valid_session_id(bad), "{bad:?}");
            let c = click_for(
                Some(&Open::Session(bad.into())),
                &host(),
                &Settings::default(),
            );
            assert_eq!(c.execute, None, "{bad:?}");
            assert_eq!(c.group, "garage-alerts-many");
        }
        // a relative or missing binary: no -execute
        for bin in [None, Some("claude-garage"), Some("./claude-garage")] {
            let h = Host {
                bin: bin.map(Into::into),
                ..host()
            };
            assert_eq!(execute_command(&Open::Blocked, &h), None, "{bin:?}");
        }
        // a control character in a path refuses it rather than smuggling a newline
        let h = Host {
            bin: Some("/bin/x\nrm".into()),
            ..host()
        };
        assert_eq!(execute_command(&Open::Blocked, &h), None);
        // a lone done and a lone needs-input click their own session, a mix goes to the blocked one
        let n = |l| item(Kind::NeedsInput, l, None, None, false);
        let f = |l| item(Kind::Done, l, None, Some(30), false);
        let open = |v: Vec<Item>| compose(&v, &Settings::default()).open;
        assert_eq!(
            open(vec![f("a")]),
            Some(Open::Session("garage/app/a".into()))
        );
        assert_eq!(open(vec![n("a"), n("b")]), Some(Open::Blocked));
        assert_eq!(open(vec![n("a"), f("b")]), Some(Open::Blocked));
        assert_eq!(open(vec![f("a"), f("b")]), None);
    }

    #[test]
    fn shell_quoting_survives_spaces_and_single_quotes() {
        let path = "/Users/o'neil/My Apps/$HOME `x`/claude-garage";
        let quoted = sh_quote(path).unwrap();
        assert_eq!(
            quoted,
            r#"'/Users/o'\''neil/My Apps/$HOME `x`/claude-garage'"#
        );
        // the shell hands the program back exactly the path
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf %s {quoted}"))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap(), path);
        assert_eq!(sh_quote("a\nb"), None);
        let h = Host {
            bin: Some(path.into()),
            ..host()
        };
        assert!(
            execute_command(&Open::Blocked, &h)
                .unwrap()
                .contains(&format!("{quoted} alerts open --blocked"))
        );
    }

    #[test]
    fn the_terminal_comes_from_the_setting_first_then_what_the_wall_reported() {
        for (reported, bundle) in [
            ("ghostty", "com.mitchellh.ghostty"),
            ("iTerm.app", "com.googlecode.iterm2"),
            ("Apple_Terminal", "com.apple.Terminal"),
            ("WezTerm", "com.github.wez.wezterm"),
            ("kitty", "net.kovidgoyal.kitty"),
            ("com.example.term", "com.example.term"),
        ] {
            assert_eq!(
                terminal_bundle(reported).as_deref(),
                Some(bundle),
                "{reported}"
            );
        }
        for unknown in ["vscode", "tmux", "x y.z", "a;b.c", ""] {
            assert_eq!(terminal_bundle(unknown), None, "{unknown:?}");
        }
        let with = |terminal: Option<&str>, setting: Option<&str>| {
            let h = Host {
                terminal: terminal.map(Into::into),
                ..host()
            };
            let s = Settings {
                terminal: setting.map(Into::into),
                ..Default::default()
            };
            click_for(Some(&Open::Blocked), &h, &s).activate
        };
        assert_eq!(
            with(Some("kitty"), None).as_deref(),
            Some("net.kovidgoyal.kitty")
        );
        assert_eq!(
            with(Some("kitty"), Some("com.apple.Terminal")).as_deref(),
            Some("com.apple.Terminal")
        );
        assert_eq!(with(Some("vscode"), None), None, "unknown: no -activate");
        assert_eq!(with(None, None), None);
        let argv = tn(&click_for(
            Some(&Open::Blocked),
            &Host {
                terminal: None,
                ..host()
            },
            &Settings::default(),
        ));
        assert!(!argv.contains(&"-activate".to_string()));
    }

    #[test]
    fn the_terminal_option_is_set_validated_and_reset_by_the_cli() {
        let (dir, out, clock) = (tmp("term"), Rec::default(), Fake::at(0));
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let shown = cli(
            &w(&["terminal", "com.googlecode.iterm2"]),
            &dir,
            &out,
            &clock,
        )
        .unwrap();
        assert!(
            shown.contains("terminal     com.googlecode.iterm2"),
            "{shown}"
        );
        assert_eq!(
            Settings::load(&dir).terminal.as_deref(),
            Some("com.googlecode.iterm2")
        );
        for bad in ["iTerm", "a b.c", "x;y.z"] {
            assert!(
                cli(&w(&["terminal", bad]), &dir, &out, &clock).is_err(),
                "{bad}"
            );
        }
        assert!(
            cli(&w(&["terminal", "auto"]), &dir, &out, &clock)
                .unwrap()
                .contains("terminal     auto")
        );
        assert_eq!(Settings::load(&dir).terminal, None);
    }

    #[test]
    fn a_needs_input_event_carries_the_click_from_the_run_context() {
        let (dir, d, out, clock) = (tmp("clk"), FakeDaemon::start(), Rec::default(), Fake::at(0));
        sessions(&d);
        let c = Context::from_value(json!({
            "event": {"name": "session.status_changed", "at": 1,
                "data": {"id": "garage/ws/fix", "from": "working", "to": "needs-input", "since": 1}},
            "ui": {"visible": false, "terminal": "WezTerm"},
            "garage": {"bin": "/opt/g/claude-garage", "port": 5151, "dir": "/tmp/g", "tmux_socket": "t1"},
        }));
        handle_event(&c, &api(&d), &dir, &out, &clock).unwrap();
        let clicks = out.1.lock().unwrap().clone();
        assert_eq!(clicks.len(), 1);
        assert_eq!(
            clicks[0].activate.as_deref(),
            Some("com.github.wez.wezterm")
        );
        assert_eq!(clicks[0].group, "garage-alerts-garage-ws-fix");
        assert_eq!(
            clicks[0].execute.as_deref(),
            Some(
                "GARAGE_PORT='5151' GARAGE_DIR='/tmp/g' GARAGE_TMUX_SOCKET='t1' '/opt/g/claude-garage' alerts open 'garage/ws/fix'"
            )
        );
    }

    // ---- alerts open ----

    #[test]
    fn open_asks_the_wall_to_focus_the_session_or_the_blocked_one() {
        let d = FakeDaemon::start();
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        open(&w(&["garage/app/fix"]), &api(&d)).unwrap();
        open(&w(&["--blocked"]), &api(&d)).unwrap();
        assert_eq!(
            d.bodies("POST", "/api/plugins/ui/action"),
            [
                json!({"action": "session.focus", "target": {"session": "garage/app/fix"}}),
                json!({"action": "jump.blocked"}),
            ]
        );
    }

    #[test]
    fn open_with_no_daemon_or_no_wall_is_quiet_and_a_bad_id_is_an_error() {
        let w = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let nobody = Api::new("http://127.0.0.1:1", "alerts", "t");
        assert!(
            open(&w(&["--blocked"]), &nobody).is_ok(),
            "exit 0 when nothing listens"
        );
        let d = FakeDaemon::start();
        d.route(
            "POST",
            "/api/plugins/ui/action",
            422,
            r#"{"error":"x","code":"invalid_target"}"#,
        );
        assert!(
            open(&w(&["garage/app/gone"]), &api(&d)).is_ok(),
            "a refusal is not worth an error either"
        );
        for bad in [
            w(&[]),
            w(&["nope"]),
            w(&["garage/a/b/c"]),
            w(&["--blocked", "x"]),
        ] {
            assert!(open(&bad, &api(&d)).is_err(), "{bad:?}");
        }
        assert_eq!(
            d.bodies("POST", "/api/plugins/ui/action").len(),
            1,
            "only the valid call reached the daemon"
        );
    }

    // ---- the chimes ----

    fn wav(bytes: &[u8]) -> (u16, u32, u16, Vec<i16>) {
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        let u16at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        let u32at =
            |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        assert_eq!(u16at(20), 1, "PCM");
        assert_eq!(&bytes[36..40], b"data");
        let n = u32at(40) as usize;
        let samples = bytes[44..44 + n]
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        (u16at(22), u32at(24), u16at(34), samples)
    }

    #[test]
    fn the_chimes_are_short_quiet_mono_22k_16_bit() {
        for (name, bytes) in [("needs-input", CHIME_NEEDS_INPUT), ("done", CHIME_DONE)] {
            let (channels, rate, bits, s) = wav(bytes);
            assert_eq!((channels, rate, bits), (1, 22_050, 16), "{name}");
            assert!(bytes.len() < 30_000, "{name}: {} bytes", bytes.len());
            let secs = s.len() as f64 / 22_050.0;
            assert!((0.2..0.5).contains(&secs), "{name}: {secs} s");
            let peak = s.iter().map(|v| i32::from(*v).abs()).max().unwrap();
            assert!(
                (16_300..=16_450).contains(&peak),
                "{name}: peak {peak} is about -6 dBFS"
            );
            assert_eq!(
                (s[0], s[s.len() - 1]),
                (0, 0),
                "{name}: starts and ends at zero, no click"
            );
            let tail = &s[s.len() - 40..];
            assert!(tail.iter().all(|v| v.abs() < 200), "{name}: fades out");
        }
    }

    #[test]
    fn chimes_are_written_once_and_repaired_when_changed() {
        let dir = tmp("wavs");
        let (a, b) = ensure_chimes(&dir).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), CHIME_NEEDS_INPUT);
        assert_eq!(std::fs::read(&b).unwrap(), CHIME_DONE);
        std::fs::write(&a, b"damaged").unwrap();
        ensure_chimes(&dir).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), CHIME_NEEDS_INPUT);
    }
}
