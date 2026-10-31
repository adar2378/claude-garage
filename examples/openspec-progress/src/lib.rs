//! OpenSpec progress: count `- [x]` against `- [ ]` in every
//! `openspec/changes/*/tasks.md` of a workspace's folder.
//!
//! Everything that decides something is a plain function over text or a
//! folder, so it is tested without a daemon. [`tick`] and [`overlay`] are the
//! thin parts that talk to the daemon through the public SDK.

use std::collections::BTreeMap;
use std::path::Path;

use garage_plugin_sdk::{Api, Click, Error, Segment, Style, WidgetItem, WidgetPut};

pub const WIDGET: &str = "progress";
pub const OPEN_ACTION: &str = "openspec-progress:changes";

/// One change under `openspec/changes/` and its task counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub name: String,
    pub done: usize,
    pub total: usize,
}

impl Change {
    pub fn percent(&self) -> usize {
        if self.total == 0 { 0 } else { self.done * 100 / self.total }
    }

    pub fn complete(&self) -> bool {
        self.total > 0 && self.done == self.total
    }
}

/// (done, total) checkboxes in a tasks file. A checkbox is a list item
/// (`-`, `*` or `+`) starting with `[ ]` or `[x]`; indented sub-tasks count;
/// anything inside a code fence does not.
pub fn count_tasks(markdown: &str) -> (usize, usize) {
    let (mut done, mut total) = (0, 0);
    let mut fenced = false;
    for line in markdown.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let Some(rest) = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .or_else(|| t.strip_prefix("+ "))
        else {
            continue;
        };
        if rest.starts_with("[ ]") {
            total += 1;
        } else if rest.starts_with("[x]") || rest.starts_with("[X]") {
            total += 1;
            done += 1;
        }
    }
    (done, total)
}

/// The changes of the workspace folder `dir`, sorted by name. `archive` is
/// history, not an active change; a change with no checkboxes has no progress
/// to show and is left out.
pub fn read_changes(dir: &Path) -> Vec<Change> {
    let Ok(rd) = std::fs::read_dir(dir.join("openspec/changes")) else {
        return vec![];
    };
    let mut out: Vec<Change> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "archive" || name.starts_with('.') {
                return None;
            }
            let text = std::fs::read_to_string(e.path().join("tasks.md")).ok()?;
            let (done, total) = count_tasks(&text);
            (total > 0).then_some(Change { name, done, total })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// (done, total) over all changes.
pub fn totals(changes: &[Change]) -> (usize, usize) {
    changes
        .iter()
        .fold((0, 0), |(d, t), c| (d + c.done, t + c.total))
}

fn segment(text: &str, fg: &str, click: bool) -> Segment {
    Segment {
        text: text.to_owned(),
        style: Some(Style {
            fg: Some(fg.to_owned()),
            ..Default::default()
        }),
        click: click.then(|| Click {
            action: OPEN_ACTION.to_owned(),
            args: None,
        }),
    }
}

/// The rail-header item for one workspace; `None` when it has no tasks, so
/// workspaces without OpenSpec show nothing.
pub fn header_item(changes: &[Change]) -> Option<WidgetItem> {
    let (done, total) = totals(changes);
    if total == 0 {
        return None;
    }
    let colour = if done == total { "ok" } else { "accent" };
    Some(WidgetItem {
        segments: Some(vec![
            segment("spec ", "dim", true),
            segment(&format!("{done}/{total}"), colour, true),
        ]),
        ..Default::default()
    })
}

/// The whole widget: one scope per workspace that has tasks.
pub fn widget_put(by_workspace: &BTreeMap<String, Vec<Change>>) -> WidgetPut {
    WidgetPut {
        scopes: by_workspace
            .iter()
            .filter_map(|(ws, changes)| header_item(changes).map(|i| (ws.clone(), vec![i])))
            .collect(),
        ttl_secs: None,
    }
}

/// A fixed-width bar: `[#####-----]`.
pub fn bar(done: usize, total: usize, width: usize) -> String {
    let filled = if total == 0 { 0 } else { (done * width).div_ceil(total).min(width) };
    // A bar is full only when everything is done.
    let filled = if done < total { filled.min(width.saturating_sub(1)) } else { filled };
    format!("[{}{}]", "#".repeat(filled), "-".repeat(width - filled))
}

fn fit(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        format!("{s:<width$}")
    } else {
        let cut: String = s.chars().take(width.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// The text of the overlay, one row per change plus a total.
pub fn overlay_lines(workspace: &str, dir: &str, changes: &[Change]) -> Vec<String> {
    let mut lines = vec![format!("OpenSpec changes in {workspace}"), format!("  {dir}"), String::new()];
    if changes.is_empty() {
        lines.push("  No openspec/changes/*/tasks.md with checkboxes here.".into());
    }
    for c in changes {
        lines.push(format!(
            "  {} {} {:>3}/{:<3} {:>3}%{}",
            fit(&c.name, 32),
            bar(c.done, c.total, 10),
            c.done,
            c.total,
            c.percent(),
            if c.complete() { "  done" } else { "" }
        ));
    }
    if changes.len() > 1 {
        let (d, t) = totals(changes);
        lines.push(String::new());
        lines.push(format!("  {} {} {d}/{t}", fit("all changes", 32), bar(d, t, 10)));
    }
    lines.push(String::new());
    lines.push("  press any key to close".into());
    lines
}

/// (name, dir) of every workspace the daemon knows.
fn workspaces(api: &Api) -> Result<Vec<(String, String)>, Error> {
    Ok(api
        .get("/api/workspaces")?
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| Some((w["name"].as_str()?.to_owned(), w["dir"].as_str()?.to_owned())))
        .collect())
}

/// The timer hook: read every workspace and replace the widget. Returns the
/// number of workspaces that have tasks.
pub fn tick(api: &Api) -> Result<usize, Error> {
    let mut by_ws = BTreeMap::new();
    for (name, dir) in workspaces(api)? {
        let changes = read_changes(Path::new(&dir));
        if !changes.is_empty() {
            by_ws.insert(name, changes);
        }
    }
    if by_ws.is_empty() {
        api.clear_widget(WIDGET)?;
    } else {
        api.put_widget(WIDGET, &widget_put(&by_ws))?;
    }
    Ok(by_ws.len())
}

/// The pane: list the changes of the focused workspace's folder.
pub fn overlay(api: &Api, focused: Option<&str>) -> Result<Vec<String>, Error> {
    let all = workspaces(api)?;
    let Some((name, dir)) = focused.and_then(|f| all.iter().find(|(n, _)| n == f)) else {
        return Ok(vec![
            "OpenSpec changes".into(),
            String::new(),
            "  No focused workspace. Focus one in the rail and open this again.".into(),
        ]);
    };
    Ok(overlay_lines(name, dir, &read_changes(Path::new(dir))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use garage_plugin_sdk::testing::FakeDaemon;

    fn change(name: &str, done: usize, total: usize) -> Change {
        Change {
            name: name.into(),
            done,
            total,
        }
    }

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
        // x, ' ', X, ' ', x(no space after box still starts with [x]), ' ' = 6 boxes, 3 done
        assert_eq!(count_tasks(md), (3, 6));
        assert_eq!(count_tasks(""), (0, 0));
    }

    #[test]
    fn changes_come_from_tasks_files_skipping_archive_and_empty_ones_sorted() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().join("openspec/changes");
        for (dir, tasks) in [
            ("p2-b", Some("- [x] a\n- [ ] b\n")),
            ("p1-a", Some("- [x] a\n")),
            ("archive", Some("- [ ] old\n")),
            ("no-checkboxes", Some("just prose\n")),
            ("no-tasks-file", None),
            (".hidden", Some("- [ ] x\n")),
        ] {
            std::fs::create_dir_all(base.join(dir)).unwrap();
            if let Some(t) = tasks {
                std::fs::write(base.join(dir).join("tasks.md"), t).unwrap();
            }
        }
        std::fs::write(base.join("stray-file.md"), "- [ ] x").unwrap();
        assert_eq!(
            read_changes(d.path()),
            vec![change("p1-a", 1, 1), change("p2-b", 1, 2)]
        );
        assert!(read_changes(&d.path().join("nowhere")).is_empty());
    }

    #[test]
    fn percent_and_bar_never_show_full_before_everything_is_done() {
        assert_eq!(change("x", 1, 3).percent(), 33);
        assert_eq!(change("x", 0, 0).percent(), 0);
        assert!(change("x", 3, 3).complete() && !change("x", 2, 3).complete());
        assert_eq!(bar(0, 4, 10), "[----------]");
        assert_eq!(bar(1, 4, 10), "[###-------]");
        assert_eq!(bar(99, 100, 10), "[#########-]", "99% is not a full bar");
        assert_eq!(bar(4, 4, 10), "[##########]");
        assert_eq!(bar(0, 0, 4), "[----]");
    }

    #[test]
    fn the_header_item_shows_done_over_total_and_is_clickable_and_absent_without_tasks() {
        assert!(header_item(&[]).is_none());
        let item = header_item(&[change("a", 1, 2), change("b", 3, 4)]).unwrap();
        let segs = item.segments.unwrap();
        assert_eq!(segs[1].text, "4/6");
        assert_eq!(segs[1].style.as_ref().unwrap().fg.as_deref(), Some("accent"));
        assert_eq!(segs[1].click.as_ref().unwrap().action, OPEN_ACTION);
        let finished = header_item(&[change("a", 2, 2)]).unwrap().segments.unwrap();
        assert_eq!(finished[1].style.as_ref().unwrap().fg.as_deref(), Some("ok"));
    }

    #[test]
    fn the_overlay_lists_each_change_with_a_total_only_when_there_are_several() {
        let one = overlay_lines("app", "/w/app", &[change("add-search", 2, 4)]);
        let text = one.join("\n");
        assert!(text.contains("OpenSpec changes in app") && text.contains("/w/app"));
        assert!(text.contains("add-search") && text.contains("2/4") && text.contains("50%"), "{text}");
        assert!(!text.contains("all changes"));
        let two = overlay_lines("app", "/w/app", &[change("a", 1, 1), change("b", 0, 3)]).join("\n");
        assert!(two.contains("all changes") && two.contains("1/4"), "{two}");
        assert!(two.contains("100%  done"), "{two}");
        let none = overlay_lines("app", "/w/app", &[]).join("\n");
        assert!(none.contains("No openspec/changes"));
        // A long name is cut, not allowed to push the numbers out of place.
        let long = overlay_lines("app", "/w", &[change(&"n".repeat(60), 1, 2)]);
        assert_eq!(long[3].chars().count(), long[3].trim_end().chars().count());
        assert!(long[3].contains('…'));
    }

    fn workspace_dir(root: &Path, name: &str, tasks: Option<&str>) -> String {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("openspec/changes/c1")).unwrap();
        if let Some(t) = tasks {
            std::fs::write(dir.join("openspec/changes/c1/tasks.md"), t).unwrap();
        }
        dir.to_string_lossy().into_owned()
    }

    #[test]
    fn tick_pushes_one_scope_per_workspace_with_tasks_and_clears_when_none_have() {
        let root = tempfile::tempdir().unwrap();
        let a = workspace_dir(root.path(), "a", Some("- [x] 1\n- [ ] 2\n"));
        let b = workspace_dir(root.path(), "b", None);
        let list = format!(r#"[{{"name":"a","dir":{a:?}}},{{"name":"b","dir":{b:?}}}]"#);
        let daemon = FakeDaemon::start(move |req| match req.path.as_str() {
            "/api/workspaces" => (200, list.clone()),
            _ => (200, r#"{"ok":true,"rev":1}"#.into()),
        });
        let api = daemon.api("openspec-progress");
        assert_eq!(tick(&api).unwrap(), 1);
        let put = daemon
            .requests()
            .into_iter()
            .find(|r| r.method == "PUT")
            .unwrap();
        assert_eq!(put.path, "/api/plugins/openspec-progress/widgets/progress");
        let scopes = put.body["scopes"].as_object().unwrap();
        assert_eq!(scopes.keys().collect::<Vec<_>>(), ["a"], "b has no tasks file");
        assert_eq!(scopes["a"][0]["segments"][1]["text"], "1/2");

        // Nothing to show anywhere: the widget is cleared, not left stale.
        let empty = FakeDaemon::start(|_| (200, "[]".into()));
        assert_eq!(tick(&empty.api("openspec-progress")).unwrap(), 0);
        assert_eq!(
            empty.log().last().unwrap(),
            "DELETE /api/plugins/openspec-progress/widgets/progress"
        );
    }

    #[test]
    fn the_overlay_reads_the_focused_workspace_and_says_so_when_none_is_focused() {
        let root = tempfile::tempdir().unwrap();
        let a = workspace_dir(root.path(), "a", Some("- [x] 1\n"));
        let list = format!(r#"[{{"name":"a","dir":{a:?}}}]"#);
        let daemon = FakeDaemon::start(move |_| (200, list.clone()));
        let api = daemon.api("openspec-progress");
        let text = overlay(&api, Some("a")).unwrap().join("\n");
        assert!(text.contains("c1") && text.contains("1/1"), "{text}");
        assert!(overlay(&api, None).unwrap().join("\n").contains("No focused workspace"));
        assert!(overlay(&api, Some("gone")).unwrap().join("\n").contains("No focused workspace"));
    }
}
