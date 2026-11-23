//! Rewrites LSP hover Markdown so it fits gpui-kit's hover popover, whose code
//! blocks drop their padding: the language label overlaps the first code line.

use lsp::lsp_types::{Hover, HoverContents, MarkedString, MarkupContent, MarkupKind};

/// Returns `hover` with its Markdown made popover-friendly (see [`tidy_markdown`]).
pub(super) fn tidy(mut hover: Hover) -> Hover {
    hover.contents = match hover.contents {
        HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }) => HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: tidy_markdown(&value),
        }),
        HoverContents::Scalar(MarkedString::String(value)) => {
            HoverContents::Scalar(MarkedString::String(tidy_markdown(&value)))
        }
        HoverContents::Array(items) => HoverContents::Array(
            items
                .into_iter()
                .map(|item| match item {
                    MarkedString::String(value) => MarkedString::String(tidy_markdown(&value)),
                    other @ MarkedString::LanguageString(_) => other,
                })
                .collect(),
        ),
        other => other,
    };
    hover
}

/// A one-line fenced block becomes inline code; longer blocks lose their
/// language tag so no label is drawn over the code.
pub(super) fn tidy_markdown(markdown: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut lines = markdown.lines();
    while let Some(line) = lines.next() {
        let Some((indent, fence)) = opening_fence(line) else {
            out.push(line.to_string());
            continue;
        };
        let mut body = Vec::new();
        let mut closed = false;
        for inner in lines.by_ref() {
            if is_closing_fence(inner, &fence) {
                closed = true;
                break;
            }
            body.push(inner);
        }
        let code: Vec<&str> = body.iter().copied().filter(|l| !l.trim().is_empty()).collect();
        if closed && let [only] = code.as_slice() {
            out.push(format!("{indent}{}", inline_code(only.trim())));
            continue;
        }
        out.push(format!("{indent}{fence}"));
        out.extend(body.iter().map(ToString::to_string));
        if closed {
            out.push(format!("{indent}{fence}"));
        }
    }
    out.join("\n")
}

/// `(indent, fence)` of a line opening a fenced code block.
fn opening_fence(line: &str) -> Option<(&str, String)> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = trimmed.chars().take_while(|c| *c == marker).count();
    if len < 3 || indent.len() > 3 {
        return None;
    }
    Some((indent, marker.to_string().repeat(len)))
}

fn is_closing_fence(line: &str, fence: &str) -> bool {
    let trimmed = line.trim();
    let marker = fence.chars().next().unwrap_or('`');
    trimmed.len() >= fence.len() && trimmed.chars().all(|c| c == marker)
}

/// Wraps `code` in enough backticks that its own backticks stay literal.
fn inline_code(code: &str) -> String {
    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let ticks = "`".repeat(longest + 1);
    let pad = if code.starts_with('`') || code.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{code}{pad}{ticks}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_block_becomes_inline_code() {
        let md = "```ruby\nbuild_from_advert()\n```\n\n**Definitions**: [a.rb](file:///a.rb)";
        assert_eq!(
            tidy_markdown(md),
            "`build_from_advert()`\n\n**Definitions**: [a.rb](file:///a.rb)"
        );
    }

    #[test]
    fn multi_line_block_loses_language_tag() {
        let md = "```rust\nfn a() {}\nfn b() {}\n```\ntext";
        assert_eq!(tidy_markdown(md), "```\nfn a() {}\nfn b() {}\n```\ntext");
    }

    #[test]
    fn backticks_inside_code_stay_literal() {
        assert_eq!(tidy_markdown("```sh\necho `pwd`\n```"), "`` echo `pwd` ``");
    }

    #[test]
    fn unclosed_block_is_kept() {
        assert_eq!(tidy_markdown("```ruby\nfoo"), "```\nfoo");
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(tidy_markdown("just `code` here"), "just `code` here");
    }
}
