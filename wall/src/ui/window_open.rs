//! `t`'s opener: launches the focused session in a fresh OS terminal window
//! (spec tui-key-routing addendum, p12-standalone-window) — the "standalone
//! window" companion to `m` maximize. tmux already multiplexes multiple
//! attach clients onto one session, so this is purely additive: the wall
//! keeps its own client (see [`crate::pty::TileClient`]) and a second one
//! opens alongside it in a brand-new terminal process.
//!
//! macOS only for now (the router gates this before ever building an
//! [`crate::runtime::Effect::OpenWindow`] — see `GarageRouter::open_window_pressed`).
//! iTerm2 gets its native AppleScript API when installed (`/Applications/
//! iTerm.app` exists); otherwise Terminal.app's `do script`. Both drive a
//! detached `exec tmux attach -t '=<id>'` in the new window's own shell —
//! `exec` replaces the shell with the attach client, so closing the window
//! cleanly detaches (same target form [`crate::pty::TileClient::new`] uses:
//! `=<session>` for an exact-match target, quoted here since the id now
//! passes through a shell). Unlike `TileClient`, `TERM`/`TMUX` are NOT
//! stripped here — this is a fresh terminal-app process with its own
//! environment, not a PTY we're hand-wiring underneath an existing one.

use std::process::{Command, Stdio};

/// True when iTerm2 is installed — decides which AppleScript dialect
/// [`argv_for`] builds. A plain existence check (no version/capability
/// probing): the create-window + write-text API has been stable across
/// every iTerm2 release that matters here.
pub fn iterm_installed() -> bool {
    std::path::Path::new("/Applications/iTerm.app").exists()
}

/// Quote `s` as a single POSIX shell argument. Single quotes suppress every
/// shell special character except `'` itself, so a literal quote is handled
/// by closing the quoted string, emitting an escaped quote, and reopening:
/// `'\''`. Backslashes need no doubling inside single quotes — the shell
/// treats them literally there.
pub fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Escape `s` for interpolation inside an AppleScript double-quoted string
/// literal: backslash and double-quote are the only two characters
/// AppleScript treats specially there, and backslash must be escaped FIRST
/// so an already-escaped quote isn't double-escaped.
fn applescript_string_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The command line run inside the new window's shell: `exec tmux attach -t
/// '=<id>'` — the same `=`-exact-match target [`crate::pty::TileClient`]
/// uses, shell-quoted since this string now passes through `sh -c`-style
/// AppleScript `do script`/`write text`.
fn tmux_attach_line(session_id: &str) -> String {
    format!(
        "exec tmux attach -t {}",
        shell_single_quote(&format!("={session_id}"))
    )
}

/// argv for opening iTerm2 via AppleScript: create a window on the default
/// profile, then write the attach command into its (now current) session.
pub fn iterm_argv(session_id: &str) -> Vec<String> {
    let line = tmux_attach_line(session_id);
    vec![
        "osascript".to_owned(),
        "-e".to_owned(),
        "tell application \"iTerm2\" to create window with default profile".to_owned(),
        "-e".to_owned(),
        format!(
            "tell application \"iTerm2\" to tell current session of current window to write text \"{}\"",
            applescript_string_escape(&line)
        ),
    ]
}

/// argv for opening Terminal.app via AppleScript: `do script` opens (or
/// reuses) a window running the attach command, then `activate` raises it.
pub fn terminal_argv(session_id: &str) -> Vec<String> {
    let line = tmux_attach_line(session_id);
    vec![
        "osascript".to_owned(),
        "-e".to_owned(),
        format!(
            "tell application \"Terminal\" to do script \"{}\"",
            applescript_string_escape(&line)
        ),
        "-e".to_owned(),
        "tell application \"Terminal\" to activate".to_owned(),
    ]
}

/// Which app's argv to build, given whether iTerm2 is installed (the
/// installed-check is injected so this stays pure/testable — see
/// [`iterm_installed`] for the actual filesystem probe).
pub fn argv_for(session_id: &str, iterm_installed: bool) -> Vec<String> {
    if iterm_installed {
        iterm_argv(session_id)
    } else {
        terminal_argv(session_id)
    }
}

/// Launch the window, detached: spawn and drop the child immediately — never
/// `wait()` (that would block the caller's blocking-task thread on a window
/// the user may leave open indefinitely). `argv[0]` is always `"osascript"`
/// (from [`argv_for`]), so this never runs on a shell-interpreted string.
pub fn launch(session_id: &str) -> std::io::Result<()> {
    let argv = argv_for(session_id, iterm_installed());
    Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── escaping: the fiddly part — adversarial ids even though current
    // ids (garage/<workspace>/<label>) can't actually contain quotes or
    // backslashes ────────────────────────────────────────────────────────

    #[test]
    fn shell_single_quote_wraps_a_plain_string() {
        assert_eq!(shell_single_quote("garage/proj/one"), "'garage/proj/one'");
    }

    #[test]
    fn shell_single_quote_escapes_an_embedded_single_quote() {
        assert_eq!(shell_single_quote("o'brien"), r"'o'\''brien'");
    }

    #[test]
    fn shell_single_quote_leaves_backslashes_untouched() {
        // Backslash is not special inside single quotes.
        assert_eq!(shell_single_quote(r"back\slash"), r"'back\slash'");
    }

    #[test]
    fn shell_single_quote_handles_leading_and_trailing_quotes() {
        assert_eq!(shell_single_quote("'lead"), r"''\''lead'");
        assert_eq!(shell_single_quote("trail'"), r"'trail'\'''");
    }

    #[test]
    fn shell_single_quote_handles_multiple_quotes() {
        assert_eq!(shell_single_quote("a'b'c"), r"'a'\''b'\''c'");
    }

    #[test]
    fn applescript_escape_doubles_backslashes_before_quoting_quotes() {
        assert_eq!(applescript_string_escape(r#"a\b"c"#), r#"a\\b\"c"#);
    }

    #[test]
    fn applescript_escape_is_a_noop_on_plain_text() {
        assert_eq!(
            applescript_string_escape("exec tmux attach -t 'a'"),
            "exec tmux attach -t 'a'"
        );
    }

    // ── tmux_attach_line ────────────────────────────────────────────────

    #[test]
    fn tmux_attach_line_uses_the_exact_match_target_form() {
        assert_eq!(
            tmux_attach_line("garage/proj/one"),
            "exec tmux attach -t '=garage/proj/one'"
        );
    }

    #[test]
    fn tmux_attach_line_shell_escapes_an_adversarial_id() {
        assert_eq!(
            tmux_attach_line("o'brien"),
            r"exec tmux attach -t '=o'\''brien'"
        );
    }

    // ── argv construction (pure — no GUI needed) ───────────────────────

    #[test]
    fn iterm_argv_is_exactly_right() {
        assert_eq!(
            iterm_argv("garage/proj/one"),
            vec![
                "osascript".to_owned(),
                "-e".to_owned(),
                "tell application \"iTerm2\" to create window with default profile".to_owned(),
                "-e".to_owned(),
                "tell application \"iTerm2\" to tell current session of current window to write text \"exec tmux attach -t '=garage/proj/one'\"".to_owned(),
            ]
        );
    }

    #[test]
    fn terminal_argv_is_exactly_right() {
        assert_eq!(
            terminal_argv("garage/proj/one"),
            vec![
                "osascript".to_owned(),
                "-e".to_owned(),
                "tell application \"Terminal\" to do script \"exec tmux attach -t '=garage/proj/one'\"".to_owned(),
                "-e".to_owned(),
                "tell application \"Terminal\" to activate".to_owned(),
            ]
        );
    }

    #[test]
    fn argv_for_dispatches_on_the_injected_iterm_flag() {
        assert_eq!(argv_for("x", true), iterm_argv("x"));
        assert_eq!(argv_for("x", false), terminal_argv("x"));
    }

    #[test]
    fn argv_for_embeds_an_adversarial_id_safely_in_both_dialects() {
        let id = r#"weird"id\with'quotes"#;
        let iterm = argv_for(id, true);
        let terminal = argv_for(id, false);
        // The write-text/do-script line is the last element carrying user
        // input in each argv; assert the raw shell-quoted attach line
        // (single-quote-escaped, then AppleScript-escaped) appears intact.
        let expected_line = applescript_string_escape(&tmux_attach_line(id));
        assert!(iterm.last().unwrap().contains(&expected_line));
        assert!(terminal[2].contains(&expected_line));
    }
}
