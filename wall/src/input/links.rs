//! URL-under-click detection (spec tui-key-routing "Click opens links").
//!
//! The wall's mouse capture means the outer terminal never sees clicks, so
//! its own link opening (iTerm's Cmd+click) is dead inside the TUI. Claude
//! Code emits plain-text URLs and relies on the terminal to linkify — on
//! the wall, WE are that terminal. A left click that lands on a URL in a
//! tile's grid text opens it instead of engaging the tile.

/// Characters that may appear inside a URL (RFC 3986 unreserved + reserved
/// + `%`), used to walk outward from the clicked column.
fn is_url_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '-' | '.' | '_' | '~' | ':' | '/' | '?' | '#' | '[' | ']' | '@' | '!' | '$' | '&'
                | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '=' | '%'
        )
}

/// The URL spanning byte-column `col` of `line` (a single terminal row as
/// text, one char per cell), or `None`. Only http(s) schemes — a terminal
/// row's "words" are full of `:`-y things that are not links. Trailing
/// punctuation that reads as prose (`.` `,` `;` `:` `!` `?` `)` `'` `"`)
/// is trimmed, mirroring what terminal linkifiers do.
pub fn url_at(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if col >= chars.len() || !is_url_char(chars[col]) {
        return None;
    }
    // Walk out to the contiguous URL-char run around the click.
    let mut start = col;
    while start > 0 && is_url_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col + 1;
    while end < chars.len() && is_url_char(chars[end]) {
        end += 1;
    }
    let run: String = chars[start..end].iter().collect();
    // The scheme must live inside the run, and the click must land at or
    // after it (clicking prose that merely touches a URL is not a click ON
    // the URL).
    let scheme_at = run.find("https://").or_else(|| run.find("http://"))?;
    if col < start + scheme_at {
        return None;
    }
    let mut url: &str = &run[scheme_at..];
    while let Some(last) = url.chars().last() {
        if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | ')' | '\'' | '"') {
            url = &url[..url.len() - last.len_utf8()];
        } else {
            break;
        }
    }
    // A bare scheme with nothing after it is not a link.
    let rest = url.trim_start_matches("https://").trim_start_matches("http://");
    if rest.is_empty() {
        return None;
    }
    Some(url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_inside_a_url_returns_it() {
        let line = "see https://example.com/a/b?x=1 for details";
        for col in 4..31 {
            assert_eq!(
                url_at(line, col).as_deref(),
                Some("https://example.com/a/b?x=1"),
                "col {col}"
            );
        }
    }

    #[test]
    fn click_outside_returns_none() {
        let line = "see https://example.com for details";
        assert_eq!(url_at(line, 0), None); // "s" of see
        assert_eq!(url_at(line, 3), None); // the space
        assert_eq!(url_at(line, 27), None); // "for"
    }

    #[test]
    fn trailing_prose_punctuation_is_trimmed() {
        let line = "read https://docs.rs/vt100.";
        assert_eq!(url_at(line, 10).as_deref(), Some("https://docs.rs/vt100"));
        let line = "(https://example.com)";
        assert_eq!(url_at(line, 5).as_deref(), Some("https://example.com"));
    }

    #[test]
    fn non_http_schemes_and_bare_words_are_not_links() {
        assert_eq!(url_at("file:///etc/hosts", 3), None);
        assert_eq!(url_at("foo/bar/baz.rs:12", 5), None);
        assert_eq!(url_at("https://", 3), None); // bare scheme
    }

    #[test]
    fn click_on_prose_glued_before_the_scheme_is_not_a_link_click() {
        // "url=https://x.dev" — clicking "url=" should not open anything,
        // clicking the https part should.
        let line = "url=https://x.dev";
        assert_eq!(url_at(line, 1), None);
        assert_eq!(url_at(line, 6).as_deref(), Some("https://x.dev"));
    }

    #[test]
    fn out_of_range_col_is_none() {
        assert_eq!(url_at("https://x.dev", 99), None);
        assert_eq!(url_at("", 0), None);
    }
}
