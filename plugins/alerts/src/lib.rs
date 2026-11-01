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

use garage_plugin_sdk::{Api, Context, Error, StateLock, Value, read_state_in, write_state_in};
use serde::{Deserialize, Serialize};

/// A turn must have run this long for "finished" to be worth an alert.
pub const MIN_WORKING_MS: i64 = 20_000;
/// Alerts this close together are one sound and one banner.
pub const COALESCE: Duration = Duration::from_secs(2);
/// A queue mark older than this belongs to a leader that died.
pub const LEADER_STALE_MS: i64 = 15_000;
/// A `working` mark older than this is forgotten (a session that never ended).
const WORKING_KEEP_MS: i64 = 6 * 60 * 60 * 1000;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "on")]
    pub sound: bool,
    #[serde(default = "on")]
    pub needs_input: bool,
    #[serde(default = "on")]
    pub done: bool,
    #[serde(default = "on")]
    pub banners: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound: true,
            needs_input: true,
            done: true,
            banners: true,
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
    Alert {
        chime,
        banner: Some((title, body)),
    }
}

// ----------------------------------------------------------------- output

/// Where a sound and a banner go. Failures are the implementation's to
/// swallow: an alert that cannot be shown is not an error.
pub trait Out: Sync {
    fn play(&self, wav: &Path);
    fn banner(&self, title: &str, body: &str);
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
pub fn deliver(alert: &Alert, dir: &Path, out: &dyn Out) {
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
            s.spawn(|| out.banner(title, body));
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
        let item = match to {
            "working" => {
                state.working.insert(id.to_owned(), at);
                None
            }
            "needs-input" if settings.needs_input => Some(Item {
                kind: Kind::NeedsInput,
                label,
                workspace,
                message: None,
                secs: None,
                visible,
            }),
            "done" if from == "working" && settings.done => started
                .map(|t| at - t)
                .filter(|ms| *ms >= MIN_WORKING_MS)
                .map(|ms| Item {
                    kind: Kind::Done,
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
    deliver(&compose(&items, &Settings::load(dir)), dir, out);
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
        "alerts\n  sound        {}   chime when a session needs you or finishes\n  needs-input  {}   alert when a session needs you\n  done         {}   alert when a session finishes a long turn\n  banners      {}   desktop banner when no wall is in view\n\nchange: claude-garage alerts <sound|needs-input|done|banners> <on|off>\ntry:    claude-garage alerts test\n",
        onoff(s.sound),
        onoff(s.needs_input),
        onoff(s.done),
        onoff(s.banners),
    )
}

const USAGE: &str = "usage: claude-garage alerts [sound|needs-input|done|banners on|off | test]\n";

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
            out.banner("Sample needs you", "This is how an alert looks");
            Ok("played both chimes and showed a sample banner\n".to_owned())
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

// ---------------------------------------------------------- real player/notifier

/// Argv candidates to play `wav`, in the order to try them.
pub fn sound_commands(os: &str, wav: &Path) -> Vec<Vec<String>> {
    let p = wav.to_string_lossy().into_owned();
    match os {
        "macos" => vec![vec!["afplay".into(), p]],
        "linux" => vec![
            vec!["paplay".into(), p.clone()],
            vec!["aplay".into(), "-q".into(), p],
        ],
        "windows" => vec![powershell(&format!(
            "(New-Object Media.SoundPlayer '{}').PlaySync()",
            ps_quote(&p)
        ))],
        _ => Vec::new(),
    }
}

/// Argv candidates to show a banner. `terminal_notifier` says it is installed.
pub fn banner_commands(
    os: &str,
    terminal_notifier: bool,
    title: &str,
    body: &str,
) -> Vec<Vec<String>> {
    match os {
        "macos" if terminal_notifier => vec![vec![
            "terminal-notifier".into(),
            "-title".into(),
            title.into(),
            "-message".into(),
            body.into(),
        ]],
        "macos" => vec![vec![
            "osascript".into(),
            "-e".into(),
            // JSON string syntax doubles as AppleScript string syntax.
            format!(
                "display notification {} with title {}",
                serde_json::to_string(body).unwrap_or_default(),
                serde_json::to_string(title).unwrap_or_default()
            ),
        ]],
        "linux" => vec![vec![
            "notify-send".into(),
            "--app-name=claude-garage".into(),
            "--".into(),
            title.into(),
            body.into(),
        ]],
        "windows" => vec![powershell(&toast_script(title, body))],
        _ => Vec::new(),
    }
}

fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

fn toast_script(title: &str, body: &str) -> String {
    format!(
        "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
$t = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
$n = $t.GetElementsByTagName('text'); \
$n.Item(0).AppendChild($t.CreateTextNode('{}')) > $null; \
$n.Item(1).AppendChild($t.CreateTextNode('{}')) > $null; \
$app = '{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe'; \
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($t))",
        ps_quote(title),
        ps_quote(body)
    )
}

/// `powershell.exe` argv running `script` through `-EncodedCommand`, so no
/// character of a title or a path changes what PowerShell parses.
fn powershell(script: &str) -> Vec<String> {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    vec![
        "powershell.exe".into(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-EncodedCommand".into(),
        base64(&utf16),
    ]
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
fn run(argv: &[String]) -> bool {
    let Some((program, args)) = argv.split_first() else {
        return false;
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
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
        for argv in sound_commands(std::env::consts::OS, wav) {
            if run(&argv) {
                return;
            }
        }
    }

    fn banner(&self, title: &str, body: &str) {
        let os = std::env::consts::OS;
        let tn = os == "macos" && on_path("terminal-notifier");
        for argv in banner_commands(os, tn, title, body) {
            if run(&argv) {
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
    struct Rec(Mutex<Vec<String>>);

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
        fn banner(&self, title: &str, body: &str) {
            self.0
                .lock()
                .unwrap()
                .push(format!("banner {title} | {body}"));
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
                banners: false
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

    #[test]
    fn players_and_notifiers_per_os() {
        let wav = Path::new("/s/chime.wav");
        assert_eq!(sound_commands("macos", wav), [["afplay", "/s/chime.wav"]]);
        assert_eq!(
            sound_commands("linux", wav),
            [
                vec!["paplay", "/s/chime.wav"],
                vec!["aplay", "-q", "/s/chime.wav"]
            ]
        );
        assert!(sound_commands("freebsd", wav).is_empty());
        assert_eq!(
            banner_commands("macos", true, "t", "b"),
            [["terminal-notifier", "-title", "t", "-message", "b"]]
        );
        let osa = banner_commands("macos", false, "t \"x\"", "b");
        assert_eq!(osa[0][0], "osascript");
        assert_eq!(
            osa[0][2],
            r#"display notification "b" with title "t \"x\"""#
        );
        assert_eq!(
            banner_commands("linux", false, "-t", "b"),
            [vec![
                "notify-send",
                "--app-name=claude-garage",
                "--",
                "-t",
                "b"
            ]]
        );
        assert!(banner_commands("freebsd", false, "t", "b").is_empty());
    }

    #[test]
    fn windows_scripts_go_through_encoded_command_and_quote_paths() {
        let argv = &sound_commands("windows", Path::new(r"C:\Users\o'neil\c.wav"))[0];
        assert_eq!(
            &argv[..4],
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
        assert_eq!(ps_quote("o'neil"), "o''neil");
        assert!(toast_script("a'b", "c").contains("CreateTextNode('a''b')"));
        assert_eq!(
            banner_commands("windows", false, "t", "b")[0][0],
            "powershell.exe"
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
