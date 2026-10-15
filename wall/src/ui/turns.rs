//! Turn markers: where each conversation turn starts in a tile, and which
//! rows make up the *current* turn (last prompt you sent → its answer, up to
//! the prompt box). Claude draws your message as a shaded `❯ …` row; that is
//! the anchor. The wall paints a marker down the tile's left edge so the
//! current turn stands apart from older ones.

/// Rows (tile-local) of one tile's turn structure.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TurnMarks {
    /// Row of every sent prompt, oldest first.
    pub starts: Vec<u16>,
    /// `(first, last)` rows of the current turn, inclusive.
    pub current: Option<(u16, u16)>,
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.chars().count() >= 10 && t.chars().all(|c| c == '─')
}

pub fn turn_marks(screen: &vt100::Screen) -> TurnMarks {
    let (rows, cols) = screen.size();
    let lines: Vec<String> = screen.rows(0, cols).collect();

    // The prompt box: the last pair of full-width rules. Everything above
    // its top rule is conversation.
    let rules: Vec<usize> =
        lines.iter().enumerate().filter(|(_, l)| is_rule(l)).map(|(i, _)| i).collect();
    let box_top = rules
        .windows(2)
        .rev()
        .find(|w| w[1] - w[0] >= 2)
        .map_or(usize::from(rows), |w| w[0]);

    let mut starts = Vec::new();
    for (r, line) in lines.iter().enumerate().take(box_top) {
        if !line.trim_start().starts_with('❯') {
            continue;
        }
        let row = u16::try_from(r).unwrap_or(u16::MAX);
        // A sent prompt is drawn with a background shade; the live prompt
        // box (and plain text that merely starts with ❯) is not.
        let shaded = (0..cols.min(6)).any(|c| {
            screen.cell(row, c).is_some_and(|cell| cell.bgcolor() != vt100::Color::Default)
        });
        if shaded {
            starts.push(row);
        }
    }

    let current = starts.last().and_then(|&first| {
        let last = (usize::from(first)..box_top).rev().find(|&r| !lines[r].trim().is_empty())?;
        Some((first, u16::try_from(last).ok()?))
    });
    TurnMarks { starts, current }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(text: &str) -> vt100::Parser {
        let mut p = vt100::Parser::new(14, 40, 0);
        p.process(text.as_bytes());
        p
    }

    // A shaded sent prompt, like Claude draws it.
    fn sent(msg: &str) -> String {
        format!("\x1b[48;5;238m❯ {msg}\x1b[0m\r\n")
    }

    #[test]
    fn finds_every_sent_prompt_and_the_current_turn() {
        let rule = "─".repeat(30);
        let p = screen(&format!(
            "{}● old answer\r\n\r\n{}● new answer\r\n  more\r\n{rule}\r\n❯ typing\r\n{rule}\r\n",
            sent("first"),
            sent("second"),
        ));
        let m = turn_marks(p.screen());
        assert_eq!(m.starts, vec![0, 3]);
        assert_eq!(m.current, Some((3, 5)), "from the last prompt to the end of its answer");
    }

    #[test]
    fn the_live_prompt_box_is_not_a_turn() {
        let rule = "─".repeat(30);
        let p = screen(&format!("hello\r\n{rule}\r\n❯ typing\r\n{rule}\r\n"));
        assert_eq!(turn_marks(p.screen()), TurnMarks::default());
    }

    #[test]
    fn no_prompt_box_still_marks_a_turn() {
        let p = screen(&format!("{}● answer\r\n", sent("only")));
        let m = turn_marks(p.screen());
        assert_eq!(m.current, Some((0, 1)));
    }
}
