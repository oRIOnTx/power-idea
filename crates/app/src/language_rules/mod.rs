//! Block structure of languages without braces: which lines indent the next one (for
//! the editor's Enter) and which keyword closes a block (`end`, `fi`, `done`, `esac`),
//! inserted after Enter on a block's first line as JetBrains does.

mod typing;

pub use typing::{backspace_keys, reindent_after_typing, smart_enter};

use std::sync::LazyLock;

use gpui_kit::component::input::language_config::LanguageConfig;
use gpui_kit::component::input::{IndentationRules, set_language_config};
use gpui_kit::*;
use regex::Regex;

/// Brackets indent in every language; these patterns are joined with block keywords.
const OPEN_BRACKET: &str = r"[\{\[\(]\s*$";
const CLOSE_BRACKET: &str = r"[\}\]\)]";

struct Rules {
    language: &'static str,
    /// The line before the caret indents the next one.
    increase: &'static str,
    /// The text after the caret starts at the outer level.
    decrease: &'static str,
}

const RULES: &[Rules] = &[
    Rules {
        language: "ruby",
        increase: r"^\s*(def|class|module|if|unless|while|until|case|begin|for|else|elsif|when|in|rescue|ensure)\b|\bdo(\s*\|[^|]*\|)?\s*$|=\s*(if|unless|case|begin)\b|\{\s*\|[^|]*\|\s*$",
        decrease: r"^\s*(end|else|elsif|when|in|rescue|ensure)\b",
    },
    Rules {
        language: "yaml",
        increase: r":\s*([|>][-+]?)?\s*$|^\s*-\s*$",
        decrease: r"^$a",
    },
    Rules {
        language: "lua",
        increase: r"^\s*(else|elseif|repeat)\b|\bfunction\b.*\)\s*$|\b(then|do)\s*$",
        decrease: r"^\s*(end|else|elseif|until)\b",
    },
    Rules {
        language: "elixir",
        increase: r"\bdo\s*$|->\s*$|^\s*(else|rescue|catch|after)\b",
        decrease: r"^\s*(end|else|rescue|catch|after)\b",
    },
    Rules {
        language: "bash",
        increase: r"\b(then|do)\s*$|^\s*(else|elif)\b|^\s*case\b.*\bin\s*$",
        decrease: r"^\s*(fi|done|esac|else|elif)\b",
    },
];

/// Registers the indentation rules with the editor; call once at startup.
pub fn register(cx: &mut App) {
    for rules in RULES {
        let increase = Regex::new(&format!("{}|{OPEN_BRACKET}", rules.increase));
        let decrease = Regex::new(&format!("{}|^\\s*{CLOSE_BRACKET}", rules.decrease));
        let (Ok(increase), Ok(decrease)) = (increase, decrease) else {
            eprintln!("power-idea: invalid indentation rules for {}", rules.language);
            continue;
        };
        let config = LanguageConfig::default().indentation_rules(IndentationRules::new(increase, decrease));
        set_language_config(rules.language, config, cx);
    }
}

struct Closer {
    languages: &'static [&'static str],
    opens: LazyLock<Regex>,
    keyword: &'static str,
}

macro_rules! closer {
    ($languages:expr, $pattern:expr, $keyword:expr) => {
        Closer {
            languages: $languages,
            opens: LazyLock::new(|| Regex::new($pattern).expect("valid block pattern")),
            keyword: $keyword,
        }
    };
}

static CLOSERS: [Closer; 7] = [
    // Ruby: statements opening a block (not `x if y` modifiers), assignments of them, `do`.
    closer!(
        &["ruby"],
        r"^\s*(def|class|module|if|unless|while|until|case|begin|for)\b|=\s*(if|unless|case|begin)\b|\bdo(\s*\|[^|]*\|)?\s*$",
        "end"
    ),
    closer!(
        &["lua"],
        r"^\s*(local\s+)?function\b|\bfunction\b.*\)\s*$|^\s*if\b.*\bthen\s*$|^\s*(for|while)\b.*\bdo\s*$|^\s*do\s*$",
        "end"
    ),
    closer!(&["elixir"], r"\bdo\s*$|\bfn\b.*->\s*$", "end"),
    closer!(&["sh", "bash"], r"^\s*if\b.*\bthen\s*$", "fi"),
    closer!(&["sh", "bash"], r"^\s*(for|while|until)\b.*\bdo\s*$", "done"),
    closer!(&["sh", "bash"], r"^\s*case\b.*\bin\s*$", "esac"),
    // `then` / `do` on their own line after `if` / `for`.
    closer!(&["sh", "bash"], r"^\s*then\s*$", "fi"),
];

/// Keywords that continue a block rather than follow it, per closing keyword.
fn continues(keyword: &str, line: &str) -> bool {
    let first = line
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or("");
    let words: &[&str] = match keyword {
        "end" => &[
            "else", "elsif", "elseif", "when", "in", "rescue", "ensure", "catch", "after",
        ],
        "fi" => &["else", "elif"],
        _ => &[],
    };
    first == keyword || words.contains(&first)
}

/// The keyword closing the block that `line` opens, if `language` closes blocks with
/// keywords. One-line blocks (`def x; end`, endless `def x = 1`) open nothing.
pub fn block_closer(language: &str, line: &str) -> Option<&'static str> {
    // reason: the pattern is a literal covered by the tests below; it cannot fail at runtime.
    #[allow(clippy::expect_used)]
    static CLOSED_INLINE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"[\s;](end|fi|done|esac)$|^\s*def\s+[\w.?!]+(\(.*\))?\s*=[^=~]")
            .expect("valid one-line block pattern")
    });
    let code = line.trim_end();
    if CLOSED_INLINE.is_match(code) {
        return None;
    }
    CLOSERS
        .iter()
        .find(|c| c.languages.contains(&language) && c.opens.is_match(code))
        .map(|c| c.keyword)
}

pub(crate) fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// After Enter on a block's first line: the text to insert below the caret line to close
/// the block (`\n` + the opener's indent + keyword), unless the block already has a body
/// or its closing line follows.
///
/// `before` is the document up to the end of the opener line; `after` is everything after
/// the (new, empty) caret line.
pub fn closing_insert(language: &str, before: &str, after: &str) -> Option<String> {
    let opener = before.rsplit('\n').next()?;
    let keyword = block_closer(language, opener)?;
    let indent = indent_of(opener);
    let next = after.lines().find(|line| !line.trim().is_empty());
    if let Some(next) = next {
        let next_indent = indent_of(next).len();
        let body_follows = next_indent > indent.len();
        let closed = next_indent == indent.len() && continues(keyword, next.trim_start());
        if body_follows || closed {
            return None;
        }
    }
    Some(format!("\n{indent}{keyword}"))
}

/// The block language of an editor language, for languages whose blocks end in keywords.
/// ERB is Ruby inside HTML, where a closing `end` needs its own `<% %>`.
pub fn block_language(editor_language: &str) -> Option<&'static str> {
    match editor_language {
        "rb" | "ruby" => Some("ruby"),
        "lua" => Some("lua"),
        "ex" | "exs" | "elixir" => Some("elixir"),
        "sh" | "bash" => Some("sh"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{RULES, block_closer, closing_insert};

    #[test]
    fn rules_compile() {
        for rules in RULES {
            assert!(regex::Regex::new(rules.increase).is_ok(), "{}", rules.language);
            assert!(regex::Regex::new(rules.decrease).is_ok(), "{}", rules.language);
        }
    }

    #[test]
    fn finds_block_openers() {
        let ruby = |line| block_closer("ruby", line);
        assert_eq!(ruby("  def total"), Some("end"));
        assert_eq!(ruby("class User < ApplicationRecord"), Some("end"));
        assert_eq!(ruby("items.each do |item|"), Some("end"));
        assert_eq!(ruby("value = if ready"), Some("end"));
        assert_eq!(ruby("return if done"), None, "modifier if");
        assert_eq!(ruby("def name = @name"), None, "endless def");
        assert_eq!(ruby("def x; end"), None);
        assert_eq!(ruby("def append"), Some("end"));
        assert_eq!(ruby("def ==(other)"), Some("end"));
        assert_eq!(ruby("puts 1"), None);
        assert_eq!(block_closer("lua", "local function go(x)"), Some("end"));
        assert_eq!(block_closer("lua", "if x then"), Some("end"));
        assert_eq!(block_closer("elixir", "defmodule App do"), Some("end"));
        assert_eq!(block_closer("sh", "if [ -f x ]; then"), Some("fi"));
        assert_eq!(block_closer("sh", "for f in *.rs; do"), Some("done"));
        assert_eq!(block_closer("sh", "case \"$1\" in"), Some("esac"));
        assert_eq!(block_closer("rust", "fn main() {"), None);
    }

    #[test]
    fn closes_a_new_block_but_not_an_existing_one() {
        let ruby = |before, after| closing_insert("ruby", before, after);
        assert_eq!(ruby("class A\n  def go", "end\n").as_deref(), Some("\n  end"));
        assert_eq!(
            ruby("  def go", "\n  def other\n  end\n").as_deref(),
            Some("\n  end")
        );
        assert_eq!(ruby("  def go", "").as_deref(), Some("\n  end"));
        assert_eq!(
            ruby("  def go", "    body\n  end\n"),
            None,
            "splitting an existing method"
        );
        assert_eq!(ruby("  def go", "  end\n"), None, "already closed");
        assert_eq!(ruby("  if ok", "  else\n"), None);
        assert_eq!(ruby("  puts 1", ""), None);
        assert_eq!(closing_insert("sh", "if true; then", "").as_deref(), Some("\nfi"));
    }
}
