//! Which change a pull request belongs to, and which task it names.
//!
//! A PR links to a change when the change name appears, case-insensitively,
//! anywhere in its head branch or title, or when a changed file lies under
//! `openspec/changes/<name>/`. A diff path wins over a name in the branch or
//! title; among several matches the longest name wins. There is no
//! first-token branch pattern.

use regex::Regex;

use crate::github::PrData;

/// The change `name` a PR links to, from the known change `names`.
pub fn link_change<'a>(
    names: &[String],
    head_ref: &str,
    title: &str,
    paths: impl Iterator<Item = &'a str>,
) -> Option<String> {
    let longest = |found: Vec<&String>| {
        found
            .into_iter()
            .max_by(|a, b| a.len().cmp(&b.len()).then_with(|| b.cmp(a)))
            .cloned()
    };
    let paths: Vec<String> = paths.map(str::to_ascii_lowercase).collect();
    let by_path = names.iter().filter(|n| {
        !n.is_empty() && {
            let prefix = format!("openspec/changes/{}/", n.to_ascii_lowercase());
            paths.iter().any(|p| p.starts_with(&prefix))
        }
    });
    if let Some(n) = longest(by_path.collect()) {
        return Some(n);
    }
    let hay = format!(
        "{}\n{}",
        head_ref.to_ascii_lowercase(),
        title.to_ascii_lowercase()
    );
    longest(
        names
            .iter()
            .filter(|n| !n.is_empty() && hay.contains(&n.to_ascii_lowercase()))
            .collect(),
    )
}

/// The task number the title names, from the `task` group of `pattern`.
pub fn task_of(title: &str, pattern: Option<&str>) -> Option<String> {
    let re = Regex::new(pattern?).ok()?;
    let t = re.captures(title)?.name("task")?.as_str().to_owned();
    Some(t).filter(|t| !t.is_empty())
}

/// A PR linked to a change of one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedPr {
    /// The repo folder's name (as in the change rows).
    pub repo: String,
    pub slug: String,
    pub change: String,
    pub task: Option<String>,
    pub pr: PrData,
}

/// Link every PR of a repo to one of its changes; those linking nothing are
/// left out.
pub fn link_all(
    repo: &str,
    slug: &str,
    prs: &[PrData],
    names: &[String],
    task_pattern: Option<&str>,
) -> Vec<LinkedPr> {
    prs.iter()
        .filter_map(|pr| {
            let change = link_change(
                names,
                &pr.head_ref,
                &pr.title,
                pr.files.iter().map(|f| f.path.as_str()),
            )?;
            Some(LinkedPr {
                repo: repo.to_owned(),
                slug: slug.to_owned(),
                change,
                task: task_of(&pr.title, task_pattern),
                pr: pr.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_TASK_PATTERN;
    use crate::github::{Checks, PrFile};

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn pr(title: &str, branch: &str, paths: &[&str]) -> PrData {
        PrData {
            number: 1,
            url: "https://github.com/o/r/pull/1".into(),
            title: title.into(),
            draft: false,
            head_ref: branch.into(),
            head_sha: "abc1234".into(),
            updated_at: String::new(),
            merged: false,
            files: paths
                .iter()
                .map(|p| PrFile {
                    path: (*p).into(),
                    status: "added".into(),
                })
                .collect(),
            checks: Checks::default(),
        }
    }

    fn linked(p: &PrData, n: &[&str]) -> Option<LinkedPr> {
        link_all(
            "r",
            "o/r",
            std::slice::from_ref(p),
            &names(n),
            Some(DEFAULT_TASK_PATTERN),
        )
        .pop()
    }

    #[test]
    fn linked_by_title() {
        let p = pr("docs(openspec): propose search-ui", "claude/anything", &[]);
        assert_eq!(linked(&p, &["search-ui"]).unwrap().change, "search-ui");
    }

    #[test]
    fn linked_by_a_claude_branch_with_no_task() {
        let p = pr("fix typo", "claude/search-ui-x7q", &[]);
        let l = linked(&p, &["search-ui"]).unwrap();
        assert_eq!((l.change.as_str(), l.task), ("search-ui", None));
    }

    #[test]
    fn linked_by_diff_path() {
        let p = pr(
            "wip",
            "claude/abc123",
            &["openspec/changes/search-ui/proposal.md"],
        );
        assert_eq!(linked(&p, &["search-ui"]).unwrap().change, "search-ui");
    }

    #[test]
    fn a_diff_path_beats_a_longer_name_in_the_branch() {
        let p = pr(
            "wip",
            "feat/search-ui-extra",
            &["openspec/changes/search/proposal.md"],
        );
        assert_eq!(
            linked(&p, &["search", "search-ui"]).unwrap().change,
            "search"
        );
    }

    #[test]
    fn the_longest_name_wins() {
        let p = pr("t", "feat/search-ui", &[]);
        assert_eq!(
            linked(&p, &["search", "search-ui"]).unwrap().change,
            "search-ui"
        );
        assert_eq!(
            linked(&p, &["search-ui", "search"]).unwrap().change,
            "search-ui"
        );
    }

    #[test]
    fn the_task_comes_from_the_title() {
        let p = pr("[search-ui 1.3] debounce", "x", &[]);
        let l = linked(&p, &["search-ui"]).unwrap();
        assert_eq!(
            (l.change.as_str(), l.task.as_deref()),
            ("search-ui", Some("1.3"))
        );
    }

    #[test]
    fn matching_ignores_case_and_a_pr_linking_nothing_is_left_out() {
        assert_eq!(
            linked(&pr("Search-UI", "x", &[]), &["search-ui"])
                .unwrap()
                .change,
            "search-ui"
        );
        assert!(linked(&pr("unrelated", "main-fix", &[]), &["search-ui"]).is_none());
    }
}
