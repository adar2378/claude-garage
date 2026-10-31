//! `claude-garage plugin search [words]` (p27): public GitHub repositories
//! that carry the topic `claude-garage-plugin`, found through the GitHub
//! search API over HTTPS (the same `curl` seam as release downloads).
//!
//! Nothing here vouches for what it finds. Every result is labelled
//! unreviewed, and descriptions (text written by strangers) are stripped of
//! control characters before they reach the terminal.

use clap::ValueEnum;
use garage_plugin::validate::is_hostile_char;
use serde_json::{Value, json};

use super::fetch::Fetch;

pub const TOPIC: &str = "claude-garage-plugin";
pub const API: &str = "https://api.github.com";
const MAX_BODY: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Sort {
    /// Most starred first.
    Stars,
    /// Most recently pushed first.
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub full_name: String,
    pub description: String,
    pub stars: u64,
    pub pushed_at: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub total: u64,
    pub repos: Vec<Repo>,
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn search_url(base: &str, words: &[String], sort: Sort, limit: u32) -> String {
    let words = words.join(" ");
    let words = words.trim();
    let mut q = format!("topic:{TOPIC}");
    if !words.is_empty() {
        q.push(' ');
        q.push_str(words);
        q.push_str(" in:name,description,readme");
    }
    let sort = match sort {
        Sort::Stars => "stars",
        Sort::Updated => "updated",
    };
    format!(
        "{base}/search/repositories?q={}&sort={sort}&order=desc&per_page={}",
        encode(&q),
        limit.clamp(1, 100)
    )
}

/// Text from a stranger, safe to print: no control or invisible characters,
/// one line, at most `max` characters.
fn clean(s: &str, max: usize) -> String {
    let one_line: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !is_hostile_char(*c))
        .collect();
    let one_line = one_line.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > max {
        let cut: String = one_line.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        one_line
    }
}

pub fn parse_results(body: &str) -> Result<Found, String> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| format!("GitHub sent something unexpected: {e}"))?;
    let items = v["items"]
        .as_array()
        .ok_or("GitHub sent something unexpected: no `items` list")?;
    let repos = items
        .iter()
        .filter_map(|i| {
            let full_name = clean(i["full_name"].as_str()?, 100);
            // Only owner/name is a valid install argument; anything else is dropped.
            let (owner, name) = full_name.split_once('/')?;
            if owner.is_empty() || name.is_empty() || full_name.contains(' ') {
                return None;
            }
            Some(Repo {
                url: format!("https://github.com/{full_name}"),
                description: clean(i["description"].as_str().unwrap_or(""), 100),
                stars: i["stargazers_count"].as_u64().unwrap_or(0),
                pushed_at: clean(i["pushed_at"].as_str().unwrap_or(""), 30),
                full_name,
            })
        })
        .collect();
    Ok(Found {
        total: v["total_count"].as_u64().unwrap_or(0),
        repos,
    })
}

/// A readable reason for the failures people actually hit.
fn explain(err: String) -> String {
    if err.contains("403") || err.contains("429") {
        format!(
            "GitHub refused the search, most likely its limit of 10 searches a minute without a login. Wait a minute and try again. ({})",
            err.lines().next().unwrap_or("")
        )
    } else if err.contains("422") {
        "GitHub rejected the search words; try plain words without special characters".into()
    } else {
        format!("search failed: {err}")
    }
}

pub fn search(
    fetch: &dyn Fetch,
    base: &str,
    words: &[String],
    sort: Sort,
    limit: u32,
) -> Result<Found, String> {
    let body = fetch
        .get(&search_url(base, words, sort, limit), MAX_BODY)
        .map_err(explain)?;
    parse_results(&String::from_utf8_lossy(&body))
}

/// Seconds since the epoch of `2026-10-28T12:34:56Z` (UTC only).
pub fn parse_utc(s: &str) -> Option<i64> {
    let (date, time) = s.trim_end_matches('Z').split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hh * 3600 + mm * 60 + ss)
}

pub fn age(pushed_at: &str, now: i64) -> String {
    let Some(then) = parse_utc(pushed_at) else {
        return "unknown".into();
    };
    let secs = (now - then).max(0);
    match secs {
        0..=3599 => "just now".into(),
        3600..=86399 => format!("{}h ago", secs / 3600),
        86400..=5_183_999 => format!("{}d ago", secs / 86400),
        5_184_000..=63_071_999 => format!("{}mo ago", secs / 2_592_000),
        _ => format!("{}y ago", secs / 31_536_000),
    }
}

const WARNING: &str = "UNREVIEWED: these are public GitHub repositories with the topic `claude-garage-plugin`. Nobody has checked them, and a plugin runs as you with no sandbox. `plugin install` shows every command and permission before anything runs; read the code first.";

pub fn render_text(found: &Found, words: &[String], now: i64) -> String {
    let mut o = format!("{WARNING}\n\n");
    if found.repos.is_empty() {
        let what = if words.is_empty() {
            String::new()
        } else {
            format!(" for \"{}\"", clean(&words.join(" "), 60))
        };
        o.push_str(&format!(
            "No plugins found{what}. Try fewer words, or browse https://github.com/topics/{TOPIC}"
        ));
        return o;
    }
    for r in &found.repos {
        o.push_str(&format!(
            "{}   {} stars   updated {}\n",
            r.full_name,
            r.stars,
            age(&r.pushed_at, now)
        ));
        if !r.description.is_empty() {
            o.push_str(&format!("  {}\n", r.description));
        }
        o.push_str(&format!(
            "  claude-garage plugin install {}\n\n",
            r.full_name
        ));
    }
    o.push_str(&format!(
        "{} of {} result{}. A repository without a valid garage-plugin.toml will not install.",
        found.repos.len(),
        found.total.max(found.repos.len() as u64),
        if found.total == 1 { "" } else { "s" }
    ));
    o
}

pub fn render_json(found: &Found) -> Value {
    json!({
        "unreviewed": true,
        "warning": WARNING,
        "topic": TOPIC,
        "total": found.total,
        "results": found.repos.iter().map(|r| json!({
            "name": r.full_name.split_once('/').map(|x| x.1).unwrap_or(&r.full_name),
            "repo": r.full_name,
            "description": r.description,
            "stars": r.stars,
            "pushed_at": r.pushed_at,
            "url": r.url,
            "install": format!("claude-garage plugin install {}", r.full_name),
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::testkit::FakeFetch;

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| s.to_string()).collect()
    }

    const BODY: &str = r#"{"total_count": 3, "items": [
        {"full_name": "alice/usage-bar", "description": "5h and weekly usage", "stargazers_count": 42, "pushed_at": "2026-10-30T10:00:00Z"},
        {"full_name": "bob/quiet", "description": null, "stargazers_count": 0, "pushed_at": "2025-01-01T00:00:00Z"},
        {"full_name": "eve/evil", "description": "hi\u001b[2J\u001b]0;pwned\u0007 there\nnext line", "stargazers_count": 7, "pushed_at": "2026-10-01T00:00:00Z"},
        {"full_name": "not a repo", "stargazers_count": 9}
    ]}"#;

    #[test]
    fn the_url_asks_for_the_topic_and_encodes_the_words() {
        let u = search_url(API, &words(&["usage", "bar & more"]), Sort::Stars, 20);
        assert_eq!(
            u,
            "https://api.github.com/search/repositories?q=topic%3Aclaude-garage-plugin%20usage%20bar%20%26%20more%20in%3Aname%2Cdescription%2Creadme&sort=stars&order=desc&per_page=20"
        );
        let all = search_url(API, &[], Sort::Updated, 500);
        assert!(
            all.contains("q=topic%3Aclaude-garage-plugin&sort=updated"),
            "{all}"
        );
        assert!(all.ends_with("per_page=100"), "limit is clamped: {all}");
    }

    #[test]
    fn results_are_parsed_and_a_strangers_text_cannot_carry_terminal_escapes() {
        let f = parse_results(BODY).unwrap();
        assert_eq!(f.total, 3);
        let names: Vec<&str> = f.repos.iter().map(|r| r.full_name.as_str()).collect();
        assert_eq!(
            names,
            ["alice/usage-bar", "bob/quiet", "eve/evil"],
            "the malformed entry is dropped"
        );
        assert_eq!(f.repos[0].stars, 42);
        assert_eq!(f.repos[1].description, "");
        let evil = &f.repos[2].description;
        assert!(!evil.chars().any(char::is_control), "{evil:?}");
        assert!(!evil.contains('\n'));
        let text = render_text(&f, &[], 0);
        assert!(!text.contains('\u{1b}') && !text.contains('\u{7}'));
    }

    #[test]
    fn garbage_from_the_server_is_an_error_not_a_panic() {
        assert!(parse_results("<html>").unwrap_err().contains("unexpected"));
        assert!(parse_results("{}").unwrap_err().contains("items"));
    }

    #[test]
    fn dates_and_ages() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_utc("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(parse_utc("garbage"), None);
        assert_eq!(parse_utc("2024-13-01T00:00:00Z"), None);
        let now = parse_utc("2026-11-01T00:00:00Z").unwrap();
        assert_eq!(age("2026-10-31T23:30:00Z", now), "just now");
        assert_eq!(age("2026-10-31T19:00:00Z", now), "5h ago");
        assert_eq!(age("2026-10-29T00:00:00Z", now), "3d ago");
        assert_eq!(age("2026-04-01T00:00:00Z", now), "7mo ago");
        assert_eq!(age("2024-01-01T00:00:00Z", now), "2y ago");
        assert_eq!(
            age("2030-01-01T00:00:00Z", now),
            "just now",
            "a future date is not negative"
        );
        assert_eq!(age("", now), "unknown");
    }

    #[test]
    fn the_text_is_labelled_unreviewed_and_shows_name_stars_age_and_the_install_line() {
        let f = parse_results(BODY).unwrap();
        let now = parse_utc("2026-11-01T00:00:00Z").unwrap();
        let t = render_text(&f, &[], now);
        assert!(t.starts_with("UNREVIEWED"), "{t}");
        assert!(
            t.contains("alice/usage-bar   42 stars   updated 1d ago"),
            "{t}"
        );
        assert!(t.contains("  5h and weekly usage"));
        assert!(t.contains("claude-garage plugin install alice/usage-bar"));
        assert!(t.contains("3 of 3 results"), "{t}");
        let none = render_text(
            &Found {
                total: 0,
                repos: vec![],
            },
            &words(&["zzz"]),
            now,
        );
        assert!(none.contains("No plugins found for \"zzz\"") && none.contains("UNREVIEWED"));
    }

    #[test]
    fn json_output_says_unreviewed_and_names_the_install_command() {
        let j = render_json(&parse_results(BODY).unwrap());
        assert_eq!(j["unreviewed"], true);
        assert_eq!(j["results"][0]["name"], "usage-bar");
        assert_eq!(j["results"][0]["repo"], "alice/usage-bar");
        assert_eq!(
            j["results"][0]["install"],
            "claude-garage plugin install alice/usage-bar"
        );
        assert_eq!(j["results"][0]["stars"], 42);
    }

    #[test]
    fn search_goes_through_the_fetch_seam_and_explains_a_rate_limit() {
        let url = search_url("http://fake", &words(&["x"]), Sort::Stars, 20);
        let fetch = FakeFetch::new(&[(url.as_str(), BODY.as_bytes())]);
        let f = search(&fetch, "http://fake", &words(&["x"]), Sort::Stars, 20).unwrap();
        assert_eq!(f.repos.len(), 3);
        assert_eq!(fetch.calls.lock().unwrap().as_slice(), [url]);
        struct Limited;
        impl Fetch for Limited {
            fn get(&self, _: &str, _: usize) -> Result<Vec<u8>, String> {
                Err("download of x failed: curl: (22) The requested URL returned error: 403".into())
            }
        }
        let e = search(&Limited, "http://fake", &[], Sort::Stars, 20).unwrap_err();
        assert!(e.contains("10 searches a minute"), "{e}");
    }
}
