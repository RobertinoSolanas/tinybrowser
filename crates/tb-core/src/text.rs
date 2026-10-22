//! Text helpers shared by HTML, CSS, layout and the console output.

/// HTML whitespace (`<TAB>`, `<LF>`, `<FF>`, `<CR>`, `<SP>`).
pub fn is_html_ws(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0c' | '\r' | ' ')
}

/// CSS whitespace incl. newline semantics.
pub fn is_css_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// Replace runs of HTML whitespace with a single space.
pub fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if is_html_ws(c) {
            in_ws = true;
            continue;
        }
        if in_ws {
            if !out.is_empty() {
                out.push(' ');
            }
            in_ws = false;
        }
        out.push(c);
    }
    out
}

/// Trim + collapse + truncate to `max` characters (adds `…`).
pub fn truncate(s: &str, max: usize) -> String {
    let s = collapse_whitespace(s);
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Normalise line breaks to `\n` (also strips `<cr><lf>` and lone `<cr>`).
pub fn normalize_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\x00' => out.push('\u{fffd}'),
            _ => out.push(c),
        }
    }
    out
}

/// `true` when every byte is ASCII (used by the charset sniffer).
pub fn is_ascii(s: &str) -> bool {
    s.bytes().all(|b| b < 0x80)
}

/// Case-insensitive equality (ASCII, as used for tag/attr names).
pub fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

/// ASCII lowercase without allocation when possible.
pub fn to_ascii_lowercase(s: &str) -> String {
    if s.bytes().all(|b| !b.is_ascii_uppercase()) {
        return s.to_string();
    }
    s.to_ascii_lowercase()
}

/// Split on any of the given separators, skipping empty pieces.
pub fn split_any<'a>(s: &'a str, seps: &str) -> Vec<&'a str> {
    s.split(|c: char| seps.contains(c))
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_runs() {
        assert_eq!(collapse_whitespace(" a \n\t b  c "), "a b c");
        assert_eq!(collapse_whitespace("   "), "");
        assert_eq!(collapse_whitespace("x"), "x");
    }

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("hello world", 6), "hello…");
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn normalizes_newlines_and_nul() {
        assert_eq!(normalize_newlines("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(normalize_newlines("a\0b"), "a\u{fffd}b");
    }

    #[test]
    fn ascii_case_helpers() {
        assert!(eq_ignore_ascii_case("DiV", "div"));
        assert!(!eq_ignore_ascii_case("div", "dive"));
        assert_eq!(to_ascii_lowercase("H1"), "h1");
        assert_eq!(to_ascii_lowercase("h1"), "h1");
    }
}
