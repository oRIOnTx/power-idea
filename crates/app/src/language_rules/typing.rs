//! Typing as in JetBrains IDEs: Enter on a new block's first line closes the block in one
//! undo step, closing keywords (`end`, `else`, `fi`, …) line up with their opener as they
//! are typed, and Backspace in indentation removes a whole level or a blank line.

use std::cell::RefCell;
use std::ops::Range;

use gpui_kit::component::input::{EditorState, InputEditsEvent, RopeExt as _};
use gpui_kit::*;

use super::{block_closer, block_language, closing_insert, indent_of};
use crate::indentation::IndentStyle;

/// Keywords that end or continue a block, lined up with its opener when typed.
fn outdent_keywords(language: &str) -> &'static [&'static str] {
    match language {
        "ruby" => &["end", "else", "elsif", "when", "rescue", "ensure"],
        "lua" => &["end", "else", "elseif"],
        "elixir" => &["end", "else", "rescue", "catch", "after"],
        "sh" => &["fi", "done", "esac", "else", "elif"],
        "python" => &["elif", "else", "except", "finally"],
        _ => &[],
    }
}

/// Python: the statements a continuation keyword can follow at the same indentation.
fn python_openers(keyword: &str) -> &'static [&'static str] {
    match keyword {
        "elif" => &["if", "elif"],
        "else" => &["if", "elif", "for", "while", "try", "except"],
        "except" => &["try", "except"],
        "finally" => &["try", "except", "else"],
        _ => &[],
    }
}

/// Python has no closing keyword, so the open block is the nearest statement above that
/// the keyword can continue and that is not deeper than the typed line (`indent`).
/// The keyword never moves right.
fn python_opener_indent<'a>(keyword: &str, indent: &str, before: &'a str) -> Option<&'a str> {
    let openers = python_openers(keyword);
    let mut limit = indent.len() + 1;
    for line in before.lines().rev() {
        let code = line.trim_start();
        if code.is_empty() || code.starts_with('#') {
            continue;
        }
        let line_indent = indent_of(line);
        if line_indent.len() >= limit {
            continue;
        }
        if openers.contains(&first_word(code)) {
            return Some(line_indent);
        }
        if line_indent.is_empty() {
            return None;
        }
        limit = line_indent.len();
    }
    None
}

fn first_word(code: &str) -> &str {
    code.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("")
}

/// Indentation of the still-open block opener above (`before` ends where the typed line
/// starts), skipping nested blocks that are already closed.
fn opener_indent<'a>(language: &str, before: &'a str) -> Option<&'a str> {
    let mut depth = 0usize;
    for line in before.lines().rev() {
        let code = line.trim_start();
        if code.is_empty() {
            continue;
        }
        if matches!(first_word(code), "end" | "fi" | "done" | "esac") {
            depth += 1;
        } else if block_closer(language, line).is_some() {
            if depth == 0 {
                return Some(indent_of(line));
            }
            depth -= 1;
        }
    }
    None
}

/// Enter at the end of a new block's first line: what replaces `caret..end of line` (a
/// body line one level in, then the closing keyword) and where the caret goes.
fn smart_enter_edit(
    language: &str,
    text: &str,
    caret: usize,
    unit: &str,
) -> Option<(Range<usize>, String, usize)> {
    let line_start = text[..caret].rfind('\n').map_or(0, |ix| ix + 1);
    let line_end = text[caret..].find('\n').map_or(text.len(), |ix| caret + ix);
    if !text[caret..line_end].trim().is_empty() {
        return None;
    }
    let after = text.get(line_end + 1..).unwrap_or("");
    let closing = closing_insert(language, &text[..caret], after)?;
    let body = format!("\n{}{unit}", indent_of(&text[line_start..caret]));
    let caret_after = caret + body.len();
    Some((caret..line_end, body + &closing, caret_after))
}

/// Python: Enter on a blank, indented line leaves the block. The blank line loses its
/// whitespace and the new line starts one level out, so two Enters after a body end it.
fn python_blank_enter_edit(text: &str, caret: usize, unit: &str) -> Option<(Range<usize>, String, usize)> {
    let line_start = text[..caret].rfind('\n').map_or(0, |ix| ix + 1);
    let line_end = text[caret..].find('\n').map_or(text.len(), |ix| caret + ix);
    let line = &text[line_start..line_end];
    if line.is_empty() || !line.chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }
    let indent = &text[line_start..caret];
    let outer = indent
        .strip_suffix(unit)
        .or_else(|| indent.strip_suffix('\t'))
        .unwrap_or_else(|| indent.trim_end_matches(' '));
    let insert = format!("\n{outer}");
    Some((line_start..line_end, insert.clone(), line_start + insert.len()))
}

/// Backspace in `line` with the caret at byte `column`: on a blank line, the characters to
/// delete forward and backward to remove the line (the caret ends on the line above); in
/// indentation before code, backward to the previous indent stop. `None` is a plain Backspace.
fn backspace_counts(
    line: &str,
    column: usize,
    first_line: bool,
    indent: IndentStyle,
) -> Option<(usize, usize)> {
    let line = line.trim_end_matches(['\n', '\r']);
    let before = line.get(..column)?;
    if before.is_empty() || !before.chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }
    let rest = &line[column..];
    if rest.trim().is_empty() {
        return (!first_line).then(|| (rest.chars().count(), before.chars().count() + 1));
    }
    if indent.hard_tabs || before.contains('\t') || indent.size == 0 {
        return None;
    }
    let target = (column - 1) / indent.size * indent.size;
    Some((0, column - target))
}

/// Handles Enter in a block language; `false` leaves it to the editor.
pub fn smart_enter(
    state: &mut EditorState,
    editor_language: &str,
    indent: IndentStyle,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) -> bool {
    let Some(language) = block_language(editor_language) else {
        return false;
    };
    let selection = state.selected_range();
    if !selection.is_empty() || state.completion_menu_state().open || state.code_action_menu_state().open {
        return false;
    }
    let unit = if indent.hard_tabs {
        "\t".to_string()
    } else {
        " ".repeat(indent.size)
    };
    let text = state.text().to_string();
    let edit = if language == "python" {
        python_blank_enter_edit(&text, selection.start, &unit)
    } else {
        smart_enter_edit(language, &text, selection.start, &unit)
    };
    let Some((range, insert, caret)) = edit else {
        return false;
    };
    // One replacement, so a single ⌘Z takes back the line break and the closing keyword.
    state.set_selected_range(range, cx);
    state.replace(insert, window, cx);
    state.set_selected_range(caret..caret, cx);
    true
}

/// How many Delete and Backspace presses Backspace stands for at the caret. The editor
/// carries them out, so every caret of a multi-caret edit gets the same treatment.
pub fn backspace_keys(state: &EditorState, indent: IndentStyle) -> (usize, usize) {
    let caret = state.cursor();
    if !state.selected_range().is_empty() {
        return (0, 1);
    }
    let text = state.text();
    let point = text.offset_to_point(caret);
    let line = text.slice_line(point.row).to_string();
    let column = caret - text.line_start_offset(point.row);
    backspace_counts(&line, column, point.row == 0, indent).unwrap_or((0, 1))
}

/// A keyword moved to its opener's indentation, remembered so that the line gets its
/// indentation back if typing goes on into another word (`end` → `endpoint`).
pub struct PendingDedent {
    line_start: usize,
    original: String,
}

/// After a typed character: lines up a just-completed closing keyword with its opener,
/// or undoes that when the word grows into something else.
pub fn reindent_after_typing(
    state: &mut EditorState,
    editor_language: &str,
    event: &InputEditsEvent,
    pending: &RefCell<Option<PendingDedent>>,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) {
    let Some(language) = block_language(editor_language) else {
        return;
    };
    let [(edit, _)] = event.edits() else { return };
    let typed = edit.old_end_byte == edit.start_byte && state.cursor() == edit.new_end_byte;
    if !typed || edit.new_end_byte - edit.start_byte > 4 {
        return;
    }
    let text = state.text();
    let caret = state.cursor();
    let point = text.offset_to_point(caret);
    let line_start = text.line_start_offset(point.row);
    let line = text.slice_line(point.row).to_string();
    let column = caret - line_start;
    let (before, rest) = line.split_at(column.min(line.len()));
    if !rest.trim().is_empty() {
        return;
    }
    let indent = indent_of(before).to_string();
    let word = before.trim_start();
    let keywords = outdent_keywords(language);
    let target = if keywords.contains(&word) {
        let above = text.slice(..line_start).to_string();
        let opener = if language == "python" {
            python_opener_indent(word, &indent, &above)
        } else {
            opener_indent(language, &above)
        };
        opener
            .filter(|target| *target != indent)
            .map(str::to_string)
            .inspect(|_| {
                pending.replace(Some(PendingDedent {
                    line_start,
                    original: indent.clone(),
                }));
            })
    } else {
        // `else:` / `elif x` keep the keyword; `endpoint` / `elsewhere` do not.
        let grown = !keywords.iter().any(|k| k.starts_with(first_word(word)));
        match pending.borrow().as_ref() {
            Some(p) if p.line_start == line_start && grown && p.original != indent => {
                Some(p.original.clone())
            }
            _ => None,
        }
    };
    let Some(target) = target else { return };
    state.set_selected_range(line_start..line_start + indent.len(), cx);
    state.replace(target.clone(), window, cx);
    let caret = caret + target.len() - indent.len();
    state.set_selected_range(caret..caret, cx);
}

#[cfg(test)]
mod tests {
    use super::{
        backspace_counts, opener_indent, python_blank_enter_edit, python_opener_indent, smart_enter_edit,
    };
    use crate::indentation::IndentStyle;

    const TWO: IndentStyle = IndentStyle {
        size: 2,
        hard_tabs: false,
    };

    #[test]
    fn enter_closes_a_new_block_in_one_edit() {
        let text = "class A\n  def go\nend\n";
        let caret = text.find("go").unwrap() + 2;
        let (range, insert, after) = smart_enter_edit("ruby", text, caret, "  ").unwrap();
        assert_eq!(range, caret..caret);
        assert_eq!(insert, "\n    \n  end");
        assert_eq!(after, caret + "\n    ".len());
        let existing = "def go\n  body\nend\n";
        assert_eq!(smart_enter_edit("ruby", existing, 6, "  "), None);
        assert_eq!(
            smart_enter_edit("ruby", "def go(a, b)", 6, "  "),
            None,
            "caret mid-line"
        );
    }

    #[test]
    fn typed_keywords_find_the_open_block() {
        let text = "class A\n  def go\n    if x\n      y\n    end\n    z\n";
        assert_eq!(opener_indent("ruby", text), Some("  "), "end closes def");
        assert_eq!(opener_indent("ruby", "if x\n  a\n"), Some(""));
        assert_eq!(opener_indent("ruby", "case x\nwhen 1\n  a\n"), Some(""));
        assert_eq!(
            opener_indent("sh", "for f in *; do\n  if a; then\n    b\n  fi\n"),
            Some("")
        );
        assert_eq!(opener_indent("ruby", "puts 1\n"), None);
    }

    #[test]
    fn python_keywords_find_their_statement() {
        let text = "def go():\n    if a:\n        x\n";
        assert_eq!(python_opener_indent("elif", "        ", text), Some("    "));
        assert_eq!(python_opener_indent("else", "        ", text), Some("    "));
        let nested = "if a:\n    if b:\n        c\n";
        assert_eq!(
            python_opener_indent("else", "", nested),
            Some(""),
            "never moves right"
        );
        assert_eq!(python_opener_indent("else", "        ", nested), Some("    "));
        let tried = "try:\n    x\nexcept E:\n    y\n    # note\n";
        assert_eq!(python_opener_indent("finally", "    ", tried), Some(""));
        assert_eq!(python_opener_indent("except", "    ", "x = 1\n    y\n"), None);
        assert_eq!(
            python_opener_indent("else", "    ", "for i in r:\n    y\n"),
            Some("")
        );
    }

    #[test]
    fn python_enter_on_a_blank_line_leaves_the_block() {
        let text = "if a:\n    x\n    ";
        let caret = text.len();
        let (range, insert, after) = python_blank_enter_edit(text, caret, "    ").unwrap();
        assert_eq!(range, caret - 4..caret);
        assert_eq!(insert, "\n");
        assert_eq!(after, caret - 4 + 1);
        let nested = "def f():\n    if a:\n        x\n        \ny = 1\n";
        let caret = nested.find("x\n").unwrap() + 2 + 8;
        let (_, insert, _) = python_blank_enter_edit(nested, caret, "    ").unwrap();
        assert_eq!(insert, "\n    ");
        assert_eq!(
            python_blank_enter_edit("if a:\n    x", 11, "    "),
            None,
            "after code"
        );
        assert_eq!(python_blank_enter_edit("x\n", 2, "    "), None, "empty line");
    }

    #[test]
    fn backspace_in_indentation_goes_a_level_back() {
        assert_eq!(backspace_counts("    foo\n", 4, false, TWO), Some((0, 2)));
        assert_eq!(backspace_counts("     foo\n", 5, false, TWO), Some((0, 1)));
        assert_eq!(
            backspace_counts("    \n", 4, false, TWO),
            Some((0, 5)),
            "blank line goes"
        );
        assert_eq!(backspace_counts("    \n", 2, false, TWO), Some((2, 3)));
        assert_eq!(backspace_counts("    \n", 4, true, TWO), None, "first line");
        assert_eq!(backspace_counts("  foo\n", 5, false, TWO), None, "after code");
        assert_eq!(backspace_counts("\tfoo", 1, false, TWO), None);
    }
}
