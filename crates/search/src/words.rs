//! Word completion for any language: identifiers already written in the file
//! and in other open files that complete the word being typed.

use std::collections::HashMap;

/// Shorter words are cheaper to type than to pick from a list.
const MIN_WORD: usize = 3;

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The part of an identifier typed before byte `offset` of `text`.
pub fn word_before(text: &str, offset: usize) -> &str {
    let before = text.get(..offset).unwrap_or(text);
    let start = before
        .rfind(|c: char| !is_word_char(c))
        .map_or(0, |ix| ix + before[ix..].chars().next().map_or(1, char::len_utf8));
    &before[start..]
}

/// Identifiers in `text` with their byte offsets; numbers aren't words.
fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = None;
    text.char_indices()
        .chain(std::iter::once((text.len(), ' ')))
        .filter_map(move |(ix, c)| match (is_word_char(c), start) {
            (true, None) => {
                start = Some(ix);
                None
            }
            (false, Some(from)) => {
                start = None;
                Some((from, &text[from..ix]))
            }
            _ => None,
        })
        .filter(|(_, word)| {
            word.chars().count() >= MIN_WORD && !word.starts_with(|c: char| c.is_ascii_digit())
        })
}

/// Words completing `prefix` (the identifier before byte `offset` of `current`): matching
/// case first, then words of the current file nearest to the cursor, then words of `others`
/// used most. With an empty prefix every word qualifies.
pub fn word_completions(
    prefix: &str,
    current: &str,
    offset: usize,
    others: &[&str],
    limit: usize,
) -> Vec<String> {
    let lower = prefix.to_lowercase();
    // (case differs, source, distance or negated count)
    let mut best: HashMap<&str, (bool, u8, i64)> = HashMap::new();
    let mut consider = |word, key: (bool, u8, i64)| {
        best.entry(word)
            .and_modify(|old| *old = (*old).min(key))
            .or_insert(key);
    };
    let matches = |word: &str| -> Option<bool> {
        if word == prefix || word.len() < prefix.len() {
            return None;
        }
        if word.starts_with(prefix) {
            Some(false)
        } else {
            word.to_lowercase().starts_with(&lower).then_some(true)
        }
    };
    for (at, word) in words(current) {
        // The word under the cursor is the one being typed.
        if (at..=at + word.len()).contains(&offset) {
            continue;
        }
        if let Some(case_differs) = matches(word) {
            // Distance from the cursor to the nearer end of the word.
            let distance = if at + word.len() <= offset {
                offset - at - word.len()
            } else {
                at - offset
            };
            consider(word, (case_differs, 0, distance as i64));
        }
    }
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for text in others {
        for (_, word) in words(text) {
            *counts.entry(word).or_default() += 1;
        }
    }
    for (word, count) in counts {
        if let Some(case_differs) = matches(word) {
            consider(word, (case_differs, 1, -count));
        }
    }
    let mut ranked: Vec<(&str, (bool, u8, i64))> = best.into_iter().collect();
    ranked.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(word, _)| word.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{word_before, word_completions};

    #[test]
    fn finds_the_word_being_typed() {
        assert_eq!(word_before("user.full_na", 12), "full_na");
        assert_eq!(word_before("let x = ", 8), "");
        assert_eq!(word_before("назва_змін", "назва_змін".len()), "назва_змін");
        assert_eq!(word_before("a(кл", "a(кл".len()), "кл");
    }

    #[test]
    fn nearest_words_of_the_file_come_first() {
        let text = "def total_price\nend\n\ntotal_tax = 1\ntot\n\n\n\nputs 1, total_items";
        let offset = text.find("tot\n").unwrap() + 3;
        assert_eq!(
            word_completions("tot", text, offset, &[], 10),
            ["total_tax", "total_items", "total_price"]
        );
    }

    #[test]
    fn skips_the_typed_word_short_words_and_numbers() {
        let text = "ab abc 123abc user user_id us";
        assert_eq!(
            word_completions("us", text, text.len(), &[], 10),
            ["user_id", "user"]
        );
        assert_eq!(word_completions("user", text, text.len(), &[], 10), ["user_id"]);
        assert_eq!(
            word_completions("12", text, text.len(), &[], 10),
            [] as [String; 0]
        );
    }

    #[test]
    fn other_files_add_their_most_used_words_after_the_file_ones() {
        let others = ["render_view render_view render_list", "render_view"];
        let text = "render_here\nren";
        assert_eq!(
            word_completions("ren", text, text.len(), &others, 10),
            ["render_here", "render_view", "render_list"]
        );
        assert_eq!(word_completions("ren", text, text.len(), &others, 2).len(), 2);
    }

    #[test]
    fn matching_case_ranks_above_other_case() {
        let text = "UserName userName\nuse";
        assert_eq!(
            word_completions("use", text, text.len(), &[], 10),
            ["userName", "UserName"]
        );
        assert_eq!(
            word_completions("", "alpha beta\n", 11, &[], 10),
            ["beta", "alpha"]
        );
    }
}
