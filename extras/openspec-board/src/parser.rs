//! `tasks.md` parsing and the change state rule. Pure functions over text.
//! The checkbox rules are the ones of `examples/openspec-progress`.

/// The first unchecked task: its leading number (`2.1`) if it has one, and
/// the text after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextTask {
    pub number: Option<String>,
    pub text: String,
}

impl NextTask {
    /// `2.1 debounce input`, or just the text when there is no number.
    pub fn label(&self) -> String {
        match &self.number {
            Some(n) => format!("{n} {}", self.text),
            None => self.text.clone(),
        }
    }
}

/// What a `tasks.md` says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tasks {
    pub done: usize,
    pub total: usize,
    pub next: Option<NextTask>,
    /// Every unchecked item (`2.1 text`), in file order, for the detail view.
    pub open: Vec<String>,
}

/// `proposal` (no task ticked, or no tasks), `building`, `complete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Proposal,
    Building,
    Complete,
}

impl State {
    pub fn of(done: usize, total: usize) -> State {
        if total > 0 && done == total {
            State::Complete
        } else if done > 0 {
            State::Building
        } else {
            State::Proposal
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            State::Proposal => "proposal",
            State::Building => "building",
            State::Complete => "complete",
        }
    }
}

/// `2.1 debounce input` -> (`Some("2.1")`, `debounce input`). A number is a
/// first word made of digits and dots that starts with a digit.
fn split_number(item: &str) -> (Option<String>, String) {
    let item = item.trim();
    let (first, rest) = item.split_once(char::is_whitespace).unwrap_or((item, ""));
    let numbered = first.starts_with(|c: char| c.is_ascii_digit())
        && first.chars().all(|c| c.is_ascii_digit() || c == '.');
    if numbered {
        (
            Some(first.trim_end_matches('.').to_owned()),
            rest.trim().to_owned(),
        )
    } else {
        (None, item.to_owned())
    }
}

/// A checkbox is a list item (`-`, `*` or `+`) starting with `[ ]` or
/// `[x]`/`[X]`; indented sub-tasks count; anything inside a code fence does
/// not.
pub fn parse_tasks(markdown: &str) -> Tasks {
    let mut t = Tasks::default();
    let mut fenced = false;
    for line in markdown.lines() {
        let l = line.trim_start();
        if l.starts_with("```") || l.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let Some(rest) = l
            .strip_prefix("- ")
            .or_else(|| l.strip_prefix("* "))
            .or_else(|| l.strip_prefix("+ "))
        else {
            continue;
        };
        if let Some(item) = rest.strip_prefix("[ ]") {
            t.total += 1;
            let (number, text) = split_number(item);
            let next = NextTask { number, text };
            t.open.push(next.label());
            t.next.get_or_insert(next);
        } else if rest.starts_with("[x]") || rest.starts_with("[X]") {
            t.total += 1;
            t.done += 1;
        }
    }
    t
}

/// (done, total) only.
pub fn count_tasks(markdown: &str) -> (usize, usize) {
    let t = parse_tasks(markdown);
    (t.done, t.total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkboxes_are_counted_by_state_with_subtasks_and_without_fenced_examples() {
        let md = "\
# Tasks
## 1. First
- [x] 1.1 done
- [ ] 1.2 open
  - [X] 1.2.1 nested, capital X
  * [ ] 1.2.2 star bullet
- not a task
- [] missing space
- [x]no space after box
```md
- [ ] example inside a fence
```
Text with - [ ] in the middle
- [ ] 2.1 last
";
        // x, ' ', X, ' ', x (no space after box still starts with [x]), ' ' = 6 boxes, 3 done
        assert_eq!(count_tasks(md), (3, 6));
        assert_eq!(count_tasks(""), (0, 0));
    }

    #[test]
    fn plus_bullets_and_tilde_fences_follow_the_same_rules() {
        let md = "+ [x] a\n~~~\n- [ ] hidden\n~~~\n+ [ ] b\n";
        assert_eq!(count_tasks(md), (1, 2));
    }

    #[test]
    fn next_open_task_is_the_first_unchecked_with_its_number() {
        let md = "- [x] 1.1 a\n- [x] 1.2 b\n- [ ] 2.1 debounce input\n- [ ] 2.2 later\n";
        let t = parse_tasks(md);
        assert_eq!((t.done, t.total), (2, 4));
        let next = t.next.unwrap();
        assert_eq!(next.number.as_deref(), Some("2.1"));
        assert_eq!(next.text, "debounce input");
        assert_eq!(next.label(), "2.1 debounce input");
        assert_eq!(t.open, ["2.1 debounce input", "2.2 later"]);
    }

    #[test]
    fn a_task_without_a_number_keeps_its_whole_text() {
        let t = parse_tasks("- [ ] write the docs\n- [ ] 3. dotted\n");
        assert_eq!(t.next.as_ref().unwrap().number, None);
        assert_eq!(t.next.unwrap().label(), "write the docs");
        assert_eq!(t.open[1], "3 dotted");
    }

    #[test]
    fn a_checkbox_in_a_code_fence_is_not_a_next_task() {
        let t = parse_tasks("```\n- [ ] example\n```\n- [x] real\n");
        assert_eq!((t.done, t.total), (1, 1));
        assert!(t.next.is_none());
    }

    #[test]
    fn state_rule_covers_zero_zero_zero_n_k_n_and_n_n() {
        assert_eq!(State::of(0, 0), State::Proposal);
        assert_eq!(State::of(0, 5), State::Proposal);
        assert_eq!(State::of(2, 5), State::Building);
        assert_eq!(State::of(5, 5), State::Complete);
        assert_eq!(State::Building.as_str(), "building");
        assert!(State::Proposal < State::Building && State::Building < State::Complete);
    }
}
