//! Replace in Files: the exact byte ranges a "Find in Files" query matches
//! (same case and whole-word rules) and the text after replacing them.

use std::ops::Range;

use super::TextSearchOptions;

fn is_word_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Length in bytes of `query` matched at the start of `text`, if it matches.
fn match_at(text: &str, query: &str, case_sensitive: bool) -> Option<usize> {
    let mut chars = text.char_indices();
    for q in query.chars() {
        let (_, c) = chars.next()?;
        let same = if case_sensitive {
            c == q
        } else {
            c == q || c.to_lowercase().eq(q.to_lowercase())
        };
        if !same {
            return None;
        }
    }
    Some(chars.next().map_or(text.len(), |(ix, _)| ix))
}

/// Non-overlapping matches of `query` in `text`, left to right.
pub fn match_ranges(text: &str, query: &str, options: TextSearchOptions) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    if query.is_empty() {
        return ranges;
    }
    let mut start = 0;
    while start < text.len() {
        let Some(len) = match_at(&text[start..], query, options.case_sensitive) else {
            start += text[start..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let end = start + len;
        let bounded =
            !is_word_char(text[..start].chars().next_back()) && !is_word_char(text[end..].chars().next());
        if !options.whole_word || bounded {
            ranges.push(start..end);
            start = end;
        } else {
            start += text[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    ranges
}

/// `text` with every match replaced, and how many were replaced.
pub fn replace_all(
    text: &str,
    query: &str,
    replacement: &str,
    options: TextSearchOptions,
) -> (String, usize) {
    let ranges = match_ranges(text, query, options);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for range in &ranges {
        out.push_str(&text[cursor..range.start]);
        out.push_str(replacement);
        cursor = range.end;
    }
    out.push_str(&text[cursor..]);
    (out, ranges.len())
}

#[cfg(test)]
mod tests {
    use super::{match_ranges, replace_all};
    use crate::TextSearchOptions;

    fn options(case_sensitive: bool, whole_word: bool) -> TextSearchOptions {
        TextSearchOptions {
            case_sensitive,
            whole_word,
            limit: 0,
        }
    }

    #[test]
    fn matches_with_case_and_word_rules() {
        let text = "User user_id USER Users";
        assert_eq!(match_ranges(text, "user", options(false, false)).len(), 4);
        assert_eq!(match_ranges(text, "user", options(true, false)), vec![5..9]);
        assert_eq!(match_ranges(text, "user", options(false, true)), [0..4, 13..17]);
        assert_eq!(
            match_ranges(text, "", options(false, false)),
            [] as [std::ops::Range<usize>; 0]
        );
        // Non-ASCII text keeps byte ranges on char boundaries.
        assert_eq!(
            match_ranges("Ärger ärger", "ÄRGER", options(false, true)),
            [0..6, 7..13]
        );
    }

    #[test]
    fn replaces_every_match() {
        let (text, count) = replace_all("foo(foo_bar, Foo)\nfoo", "foo", "baz", options(false, true));
        assert_eq!((text.as_str(), count), ("baz(foo_bar, baz)\nbaz", 3));
        let (text, count) = replace_all("aaa", "aa", "b", options(true, false));
        assert_eq!((text.as_str(), count), ("ba", 1));
    }
}
