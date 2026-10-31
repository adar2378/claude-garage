//! A tiny raw-mode toolkit for pane plugins (feature `tui`): a fuzzy list
//! picker and a one-line prompt. A pane is an ordinary process in a PTY, so
//! this draws with plain cursor control; there is nothing else to learn.
//!
//! Both widgets are pure state machines: `on_key` returns an [`Outcome`],
//! `lines` returns the screen text. [`run_picker`] and [`run_prompt`] are the
//! thin loops that connect them to a terminal.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, queue, style, terminal};

use crate::fuzzy;

/// What a key did to a widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome<T> {
    Continue,
    Done(T),
    Cancel,
}

fn is_cancel(k: &KeyEvent) -> bool {
    k.code == KeyCode::Esc
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
}

/// A filterable list. `Done(i)` is the index into the original `items`.
#[derive(Debug, Clone)]
pub struct Picker {
    title: String,
    items: Vec<String>,
    query: String,
    shown: Vec<usize>,
    sel: usize,
}

impl Picker {
    pub fn new(title: &str, items: Vec<String>) -> Self {
        let shown = (0..items.len()).collect();
        Self {
            title: title.to_owned(),
            items,
            query: String::new(),
            shown,
            sel: 0,
        }
    }

    fn refilter(&mut self) {
        self.shown = fuzzy::filter(&self.query, &self.items);
        self.sel = 0;
    }

    pub fn on_key(&mut self, k: KeyEvent) -> Outcome<usize> {
        if k.kind == KeyEventKind::Release {
            return Outcome::Continue;
        }
        if is_cancel(&k) {
            return Outcome::Cancel;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Enter => {
                if let Some(&i) = self.shown.get(self.sel) {
                    return Outcome::Done(i);
                }
            }
            KeyCode::Down | KeyCode::Tab => self.step(1),
            KeyCode::Up | KeyCode::BackTab => self.step(-1),
            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::Char('p') if ctrl => self.step(-1),
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refilter();
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.refilter();
            }
            _ => {}
        }
        Outcome::Continue
    }

    fn step(&mut self, by: isize) {
        let n = self.shown.len();
        if n > 0 {
            self.sel = (self.sel as isize + by).rem_euclid(n as isize) as usize;
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The matching item texts, best first.
    pub fn visible(&self) -> Vec<&str> {
        self.shown.iter().map(|&i| self.items[i].as_str()).collect()
    }

    pub fn selected(&self) -> usize {
        self.sel
    }

    /// The screen: title, query line, then as many matches as fit `rows`,
    /// scrolled to keep the selection visible. The first selected row starts
    /// with `> `.
    pub fn lines(&self, cols: usize, rows: usize) -> Vec<String> {
        let mut out = vec![
            clip(&self.title, cols),
            clip(&format!("> {}", self.query), cols),
        ];
        let room = rows.saturating_sub(out.len()).max(1);
        if self.shown.is_empty() {
            out.push("  no matches".to_owned());
            return out;
        }
        let start = self.sel.saturating_sub(room - 1).min(self.shown.len() - 1);
        let start = if self.sel < room { 0 } else { start };
        for (row, &i) in self.shown.iter().enumerate().skip(start).take(room) {
            let mark = if row == self.sel { "> " } else { "  " };
            out.push(clip(&format!("{mark}{}", self.items[i]), cols));
        }
        out
    }
}

/// Cut `s` to `cols` display characters (wide characters count as one: the
/// texts shown here are ids and paths).
fn clip(s: &str, cols: usize) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    s.chars().take(cols).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Typing,
    Confirm,
}

/// A one-line prompt with a built-in confirm step: Enter on a non-empty line
/// asks again, the second Enter returns the text. Nothing leaves the prompt
/// without two deliberate key presses.
#[derive(Debug, Clone)]
pub struct Prompt {
    title: String,
    confirm_text: String,
    text: String,
    stage: Stage,
}

impl Prompt {
    /// `confirm_text` is shown on the second step, e.g. "Enter to send".
    pub fn new(title: &str, confirm_text: &str) -> Self {
        Self {
            title: title.to_owned(),
            confirm_text: confirm_text.to_owned(),
            text: String::new(),
            stage: Stage::Typing,
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) -> Outcome<String> {
        if k.kind == KeyEventKind::Release {
            return Outcome::Continue;
        }
        if is_cancel(&k) {
            return Outcome::Cancel;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match (self.stage, k.code) {
            (Stage::Typing, KeyCode::Enter) => {
                if !self.text.trim().is_empty() {
                    self.stage = Stage::Confirm;
                }
            }
            (Stage::Confirm, KeyCode::Enter) => return Outcome::Done(self.text.clone()),
            // Editing after the confirm prompt goes back to typing, so the
            // text that is sent is always the text that was on screen.
            (Stage::Confirm, KeyCode::Backspace) => {
                self.stage = Stage::Typing;
                self.text.pop();
            }
            (Stage::Confirm, _) => {}
            (Stage::Typing, KeyCode::Backspace) => {
                self.text.pop();
            }
            (Stage::Typing, KeyCode::Char('u')) if ctrl => self.text.clear(),
            (Stage::Typing, KeyCode::Char(c)) if !ctrl => self.text.push(c),
            _ => {}
        }
        Outcome::Continue
    }

    /// Bracketed paste: text is added in one go (newlines become spaces: the
    /// line is one line).
    pub fn paste(&mut self, s: &str) {
        if self.stage == Stage::Typing {
            self.text.extend(
                s.chars()
                    .map(|c| if c == '\n' || c == '\r' { ' ' } else { c }),
            );
        }
    }

    pub fn lines(&self, cols: usize) -> Vec<String> {
        let status = match self.stage {
            Stage::Typing => "Enter to review, Esc to cancel".to_owned(),
            Stage::Confirm => format!(
                "{} - Enter again to confirm, Esc to cancel",
                self.confirm_text
            ),
        };
        vec![
            clip(&self.title, cols),
            clip(&format!("> {}", self.text), cols),
            clip(&status, cols),
        ]
    }

    pub fn in_confirm(&self) -> bool {
        self.stage == Stage::Confirm
    }
}

/// Raw mode on the alternate screen, undone on drop (also on panic).
struct Screen;

impl Screen {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        // Bracketed paste: a pasted newline must not act as Enter (a prompt confirms on Enter).
        execute!(
            io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            event::EnableBracketedPaste
        )?;
        Ok(Self)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            event::DisableBracketedPaste,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
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

fn size() -> (usize, usize) {
    terminal::size()
        .map(|(c, r)| (c as usize, r as usize))
        .unwrap_or((80, 24))
}

/// Run a picker until the user chooses (`Some(index)`) or cancels (`None`).
pub fn run_picker(mut picker: Picker) -> io::Result<Option<usize>> {
    let _screen = Screen::enter()?;
    loop {
        let (cols, rows) = size();
        draw(&picker.lines(cols, rows))?;
        if let Event::Key(k) = event::read()? {
            match picker.on_key(k) {
                Outcome::Continue => {}
                Outcome::Done(i) => return Ok(Some(i)),
                Outcome::Cancel => return Ok(None),
            }
        }
    }
}

/// Run a prompt until it returns the confirmed text or is cancelled.
pub fn run_prompt(mut prompt: Prompt) -> io::Result<Option<String>> {
    let _screen = Screen::enter()?;
    loop {
        let (cols, _) = size();
        draw(&prompt.lines(cols))?;
        match event::read()? {
            Event::Key(k) => match prompt.on_key(k) {
                Outcome::Continue => {}
                Outcome::Done(s) => return Ok(Some(s)),
                Outcome::Cancel => return Ok(None),
            },
            Event::Paste(s) => prompt.paste(&s),
            _ => {}
        }
    }
}

/// Show `lines` until any key is pressed (or `wait` passes, if given).
pub fn show(lines: &[String], wait: Option<std::time::Duration>) -> io::Result<()> {
    let _screen = Screen::enter()?;
    draw(lines)?;
    match wait {
        Some(d) => {
            if event::poll(d)? {
                let _ = event::read()?;
            }
        }
        None => loop {
            if let Event::Key(k) = event::read()?
                && k.kind != KeyEventKind::Release
            {
                break;
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_str(p: &mut Picker, s: &str) {
        for c in s.chars() {
            p.on_key(key(KeyCode::Char(c)));
        }
    }

    fn items() -> Vec<String> {
        [
            "garage/main  idle",
            "garage/fix-auth  working",
            "docs/main  idle",
        ]
        .map(String::from)
        .to_vec()
    }

    #[test]
    fn typing_filters_and_enter_returns_the_original_index() {
        let mut p = Picker::new("Jump", items());
        type_str(&mut p, "docs");
        assert_eq!(p.visible(), ["docs/main  idle"]);
        assert_eq!(p.on_key(key(KeyCode::Enter)), Outcome::Done(2));
    }

    #[test]
    fn arrows_move_wrap_and_reset_on_typing() {
        let mut p = Picker::new("Jump", items());
        p.on_key(key(KeyCode::Down));
        assert_eq!(p.selected(), 1);
        p.on_key(key(KeyCode::Up));
        p.on_key(key(KeyCode::Up));
        assert_eq!(p.selected(), 2, "wraps to the last");
        p.on_key(ctrl('n'));
        assert_eq!(p.selected(), 0);
        p.on_key(key(KeyCode::Down));
        type_str(&mut p, "g");
        assert_eq!(p.selected(), 0, "a new query starts at the top");
        p.on_key(key(KeyCode::Backspace));
        assert_eq!(p.visible().len(), 3);
    }

    #[test]
    fn enter_on_no_match_does_nothing_and_escape_cancels() {
        let mut p = Picker::new("Jump", items());
        type_str(&mut p, "zzz");
        assert!(p.visible().is_empty());
        assert_eq!(p.on_key(key(KeyCode::Enter)), Outcome::Continue);
        assert_eq!(p.on_key(key(KeyCode::Esc)), Outcome::Cancel);
        assert_eq!(p.on_key(ctrl('c')), Outcome::Cancel);
    }

    #[test]
    fn the_screen_scrolls_to_keep_the_selection_visible() {
        let many: Vec<String> = (0..10).map(|i| format!("item-{i}")).collect();
        let mut p = Picker::new("T", many);
        for _ in 0..7 {
            p.on_key(key(KeyCode::Down));
        }
        let lines = p.lines(40, 6); // title + query + 4 rows
        assert_eq!(lines.len(), 6);
        assert!(lines.iter().any(|l| l == "> item-7"), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("item-0")));
        assert_eq!(p.lines(5, 6)[1], "> ", "long text is clipped to the width");
    }

    #[test]
    fn the_prompt_needs_two_enters_and_refuses_an_empty_line() {
        let mut p = Prompt::new("Send", "Send to 2 sessions");
        assert_eq!(p.on_key(key(KeyCode::Enter)), Outcome::Continue);
        assert!(!p.in_confirm(), "empty line stays in typing");
        for c in "ls".chars() {
            p.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(p.on_key(key(KeyCode::Enter)), Outcome::Continue);
        assert!(p.in_confirm());
        assert!(p.lines(80)[2].contains("Send to 2 sessions"));
        assert_eq!(p.on_key(key(KeyCode::Char('x'))), Outcome::Continue);
        assert_eq!(
            p.on_key(key(KeyCode::Enter)),
            Outcome::Done("ls".into()),
            "stray keys while confirming change nothing"
        );
    }

    #[test]
    fn editing_after_the_review_goes_back_to_typing() {
        let mut p = Prompt::new("Send", "ok");
        for c in "rm".chars() {
            p.on_key(key(KeyCode::Char(c)));
        }
        p.on_key(key(KeyCode::Enter));
        p.on_key(key(KeyCode::Backspace));
        assert!(!p.in_confirm());
        assert_eq!(
            p.on_key(key(KeyCode::Enter)),
            Outcome::Continue,
            "needs review again"
        );
        assert_eq!(p.on_key(key(KeyCode::Enter)), Outcome::Done("r".into()));
    }

    #[test]
    fn escape_cancels_at_both_stages_and_paste_stays_one_line() {
        let mut p = Prompt::new("Send", "ok");
        p.paste("a\nb");
        assert_eq!(p.lines(80)[1], "> a b");
        assert_eq!(p.on_key(key(KeyCode::Esc)), Outcome::Cancel);
        p.on_key(key(KeyCode::Enter));
        assert!(p.in_confirm());
        assert_eq!(p.on_key(key(KeyCode::Esc)), Outcome::Cancel);
        p.paste("late");
        assert_eq!(p.lines(80)[1], "> a b", "paste is ignored while confirming");
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut p = Prompt::new("S", "ok");
        let mut up = key(KeyCode::Char('a'));
        up.kind = KeyEventKind::Release;
        p.on_key(up);
        assert_eq!(p.lines(80)[1], "> ");
    }
}
