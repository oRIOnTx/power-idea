//! Pure helpers for navigation and refactoring results: flattening symbol
//! trees, applying text edits (UTF-16 positions) and splitting workspace edits.

use std::path::PathBuf;

use anyhow::{Result, bail};
use lsp_types::{
    DocumentChangeOperation, DocumentChanges, DocumentSymbolResponse, OneOf, Position, SymbolKind, TextEdit,
    Uri, WorkspaceEdit, WorkspaceSymbolResponse,
};

use crate::uri_to_path;

/// A symbol with where it is, ready for a picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolItem {
    pub name: String,
    /// Signature, type or container shown next to the name.
    pub detail: Option<String>,
    pub kind: SymbolKind,
    /// Nesting level in a document outline (0 for workspace results).
    pub depth: usize,
    /// File of a workspace symbol; `None` for document symbols.
    pub path: Option<PathBuf>,
    pub position: Position,
}

/// Short label for a symbol kind, as shown in JetBrains pickers.
pub fn kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::FILE => "file",
        SymbolKind::MODULE | SymbolKind::NAMESPACE | SymbolKind::PACKAGE => "module",
        SymbolKind::CLASS => "class",
        SymbolKind::METHOD => "method",
        SymbolKind::PROPERTY | SymbolKind::FIELD => "field",
        SymbolKind::CONSTRUCTOR => "constructor",
        SymbolKind::ENUM => "enum",
        SymbolKind::INTERFACE => "interface",
        SymbolKind::FUNCTION => "function",
        SymbolKind::VARIABLE => "variable",
        SymbolKind::CONSTANT => "constant",
        SymbolKind::STRUCT => "struct",
        SymbolKind::ENUM_MEMBER => "variant",
        SymbolKind::TYPE_PARAMETER => "type param",
        _ => "symbol",
    }
}

/// Kinds listed by "Go to Class".
pub fn is_type_like(kind: SymbolKind) -> bool {
    [
        SymbolKind::CLASS,
        SymbolKind::STRUCT,
        SymbolKind::INTERFACE,
        SymbolKind::ENUM,
        SymbolKind::MODULE,
        SymbolKind::NAMESPACE,
    ]
    .contains(&kind)
}

/// Document outline in source order, children after their parent.
pub fn document_symbols(response: Option<DocumentSymbolResponse>) -> Vec<SymbolItem> {
    fn walk(symbols: Vec<lsp_types::DocumentSymbol>, depth: usize, out: &mut Vec<SymbolItem>) {
        for symbol in symbols {
            out.push(SymbolItem {
                name: symbol.name,
                detail: symbol.detail.filter(|d| !d.is_empty()),
                kind: symbol.kind,
                depth,
                path: None,
                position: symbol.selection_range.start,
            });
            walk(symbol.children.unwrap_or_default(), depth + 1, out);
        }
    }
    let mut out = Vec::new();
    match response {
        None => {}
        Some(DocumentSymbolResponse::Nested(symbols)) => walk(symbols, 0, &mut out),
        Some(DocumentSymbolResponse::Flat(symbols)) => {
            out.extend(symbols.into_iter().map(|s| SymbolItem {
                name: s.name,
                detail: s.container_name,
                kind: s.kind,
                depth: 0,
                path: None,
                position: s.location.range.start,
            }));
            out.sort_by_key(|s| (s.position.line, s.position.character));
        }
    }
    out
}

/// Workspace symbol results that point into files.
pub fn workspace_symbols(response: Option<WorkspaceSymbolResponse>) -> Vec<SymbolItem> {
    let item = |name, container: Option<String>, kind, uri: &Uri, position| {
        Some(SymbolItem {
            name,
            detail: container.filter(|c| !c.is_empty()),
            kind,
            depth: 0,
            path: Some(uri_to_path(uri).ok()?),
            position,
        })
    };
    match response {
        None => Vec::new(),
        Some(WorkspaceSymbolResponse::Flat(symbols)) => symbols
            .into_iter()
            .filter_map(|s| {
                item(
                    s.name,
                    s.container_name,
                    s.kind,
                    &s.location.uri,
                    s.location.range.start,
                )
            })
            .collect(),
        Some(WorkspaceSymbolResponse::Nested(symbols)) => symbols
            .into_iter()
            .filter_map(|s| {
                let (uri, position) = match &s.location {
                    OneOf::Left(location) => (location.uri.clone(), location.range.start),
                    OneOf::Right(location) => (location.uri.clone(), Position::default()),
                };
                item(s.name, s.container_name, s.kind, &uri, position)
            })
            .collect(),
    }
}

/// Byte offset of an LSP position (UTF-16 columns); positions past the end clamp.
fn offset_of(text: &str, position: Position) -> usize {
    let mut line_start = 0;
    for _ in 0..position.line {
        match text[line_start..].find('\n') {
            Some(newline) => line_start += newline + 1,
            None => return text.len(),
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |n| line_start + n);
    let mut units = 0;
    for (ix, ch) in text[line_start..line_end].char_indices() {
        if units >= position.character as usize {
            return line_start + ix;
        }
        units += ch.len_utf16();
    }
    line_end
}

/// Applies non-overlapping edits (positions refer to the original text).
pub fn apply_text_edits(text: &str, edits: &[TextEdit]) -> Result<String> {
    let mut ranges: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            (
                offset_of(text, e.range.start),
                offset_of(text, e.range.end),
                e.new_text.as_str(),
            )
        })
        .collect();
    // Stable sort keeps the server's order for inserts at the same spot.
    ranges.sort_by_key(|&(start, end, _)| (start, end));
    if ranges
        .windows(2)
        .any(|pair| matches!(pair, [prev, next] if next.0 < prev.1))
    {
        bail!("language server returned overlapping edits");
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, new_text) in ranges {
        if start < cursor || end < start {
            bail!("language server returned an invalid edit range");
        }
        out.push_str(&text[cursor..start]);
        out.push_str(new_text);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    Ok(out)
}

/// Text edits per file of a rename; file creations, renames and deletions are rejected.
pub fn workspace_edit_files(edit: WorkspaceEdit) -> Result<Vec<(PathBuf, Vec<TextEdit>)>> {
    let mut files: Vec<(Uri, Vec<TextEdit>)> = Vec::new();
    let mut add = |uri: Uri, edits: Vec<TextEdit>| match files.iter_mut().find(|(u, _)| *u == uri) {
        Some((_, existing)) => existing.extend(edits),
        None => files.push((uri, edits)),
    };
    match edit.document_changes {
        Some(DocumentChanges::Edits(edits)) => {
            for doc in edits {
                add(
                    doc.text_document.uri,
                    doc.edits.into_iter().map(text_edit).collect(),
                );
            }
        }
        Some(DocumentChanges::Operations(ops)) => {
            for op in ops {
                match op {
                    DocumentChangeOperation::Edit(doc) => {
                        add(
                            doc.text_document.uri,
                            doc.edits.into_iter().map(text_edit).collect(),
                        );
                    }
                    DocumentChangeOperation::Op(_) => {
                        bail!("renames that create or move files are not supported")
                    }
                }
            }
        }
        None => {
            for (uri, edits) in edit.changes.unwrap_or_default() {
                add(uri, edits);
            }
        }
    }
    files
        .into_iter()
        .map(|(uri, edits)| Ok((uri_to_path(&uri)?, edits)))
        .collect()
}

fn text_edit(edit: OneOf<TextEdit, lsp_types::AnnotatedTextEdit>) -> TextEdit {
    match edit {
        OneOf::Left(edit) => edit,
        OneOf::Right(annotated) => annotated.text_edit,
    }
}

/// The identifier around a UTF-16 column of `line`, for prefilling Rename.
pub fn word_at(line: &str, character: u32) -> Option<String> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$' || c == '?' || c == '!';
    let offset = offset_of(line, Position::new(0, character));
    let start = line[..offset]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(offset, |(ix, _)| ix);
    let end = line[offset..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(line.len(), |(ix, _)| offset + ix);
    (start < end).then(|| line[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use lsp_types::{DocumentSymbol, Range};

    use super::*;

    fn edit(sl: u32, sc: u32, el: u32, ec: u32, text: &str) -> TextEdit {
        TextEdit {
            range: Range::new(Position::new(sl, sc), Position::new(el, ec)),
            new_text: text.into(),
        }
    }

    #[test]
    fn applies_edits_with_utf16_columns() {
        let text = "let ä = 1;\nprint(ä)\n";
        let edits = [edit(1, 6, 1, 7, "b"), edit(0, 4, 0, 5, "b")];
        assert_eq!(apply_text_edits(text, &edits).unwrap(), "let b = 1;\nprint(b)\n");
        // Emoji takes two UTF-16 units.
        assert_eq!(apply_text_edits("😀x", &[edit(0, 2, 0, 3, "y")]).unwrap(), "😀y");
        // Insert at end of file and past-the-end positions clamp.
        assert_eq!(apply_text_edits("a", &[edit(5, 0, 5, 0, "\n")]).unwrap(), "a\n");
        assert!(apply_text_edits("abcd", &[edit(0, 0, 0, 3, ""), edit(0, 1, 0, 2, "")]).is_err());
    }

    #[test]
    fn flattens_document_symbols() {
        #[allow(deprecated)]
        let symbol = |name: &str, line, children| DocumentSymbol {
            name: name.into(),
            detail: None,
            kind: SymbolKind::CLASS,
            tags: None,
            deprecated: None,
            range: Range::new(Position::new(line, 0), Position::new(line + 5, 0)),
            selection_range: Range::new(Position::new(line, 6), Position::new(line, 9)),
            children,
        };
        let tree = vec![
            symbol("A", 0, Some(vec![symbol("m", 1, None)])),
            symbol("B", 9, None),
        ];
        let flat = document_symbols(Some(DocumentSymbolResponse::Nested(tree)));
        let names: Vec<_> = flat.iter().map(|s| (s.name.as_str(), s.depth)).collect();
        assert_eq!(names, [("A", 0), ("m", 1), ("B", 0)]);
        assert_eq!(flat[1].position, Position::new(1, 6));
    }

    #[test]
    fn splits_workspace_edits_by_file() {
        let uri: Uri = "file:///p/a.rb".parse().unwrap();
        let edit = WorkspaceEdit {
            changes: Some(HashMap::from([(uri, vec![edit(0, 0, 0, 1, "x")])])),
            ..Default::default()
        };
        let files = workspace_edit_files(edit).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, PathBuf::from("/p/a.rb"));
    }

    #[test]
    fn finds_word_under_cursor() {
        assert_eq!(word_at("  user.full_name!", 9).as_deref(), Some("full_name!"));
        assert_eq!(word_at("foo(bar)", 4).as_deref(), Some("bar"));
        assert_eq!(word_at("foo(bar)", 3).as_deref(), Some("foo"));
        assert_eq!(word_at("a + b", 2), None);
    }
}
