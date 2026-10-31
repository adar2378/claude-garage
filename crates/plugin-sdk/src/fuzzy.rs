//! Fuzzy matching for pickers: a query matches when its characters appear in
//! order in the text (case-insensitive). Better matches score higher.

/// `None` when `query` is not a subsequence of `text`. An empty query matches
/// everything with score 0.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let (mut qi, mut total) = (0usize, 0i32);
    let mut prev: Option<usize> = None;
    for (i, c) in t.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            let mut s = 10;
            if i == 0 {
                s += 15; // starts the text
            } else if !t[i - 1].is_alphanumeric() {
                s += 8; // starts a word (after / - space ...)
            }
            if prev == Some(i.wrapping_sub(1)) && i > 0 {
                s += 6; // contiguous with the previous match
            }
            total += s;
            prev = Some(i);
            qi += 1;
        }
    }
    // Shorter texts win ties: the match covers more of them.
    (qi == q.len()).then(|| total * 4 - (t.len().min(200) as i32) / 4)
}

/// Indices of `items` that match, best first; equal scores keep their order.
pub fn filter(query: &str, items: &[String]) -> Vec<usize> {
    let mut hits: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, t)| score(query, t).map(|s| (s, i)))
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(q: &str, items: &[&str]) -> Vec<String> {
        let owned: Vec<String> = items.iter().map(|s| (*s).to_owned()).collect();
        filter(q, &owned)
            .into_iter()
            .map(|i| owned[i].clone())
            .collect()
    }

    #[test]
    fn a_query_must_be_an_in_order_subsequence() {
        assert!(score("gma", "garage/main").is_some());
        assert!(score("mag", "garage/main").is_none(), "order matters");
        assert!(score("xyz", "garage/main").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn matching_ignores_case() {
        assert!(score("GARAGE", "garage/main").is_some());
        assert!(score("garage", "GARAGE/Main").is_some());
    }

    #[test]
    fn better_matches_rank_first() {
        // a prefix beats a mid-word hit; a word start beats a scattered match
        assert_eq!(
            order("mai", &["domain-x", "garage/main", "main-app"]),
            ["main-app", "garage/main", "domain-x"]
        );
        // shorter text wins when the match is otherwise equal
        assert_eq!(
            order("app", &["app-long-name", "app"]),
            ["app", "app-long-name"]
        );
    }

    #[test]
    fn equal_scores_keep_the_original_order() {
        assert_eq!(order("a", &["a1", "a2", "a3"]), ["a1", "a2", "a3"]);
        assert_eq!(order("", &["b", "a"]), ["b", "a"]);
    }
}
