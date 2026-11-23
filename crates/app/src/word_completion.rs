//! Completion that works in every language: words already written in the file and in
//! other open files, listed under the language server's items (or alone without one).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use anyhow::Result;
use gpui_kit::base::input::Rope;
use gpui_kit::component::input::{CompletionProvider, EditorState, RopeExt as _};
use gpui_kit::*;
use lsp::lsp_types::{self, CompletionItemKind, CompletionResponse, CompletionTriggerKind};

const MAX_WORDS: usize = 30;
/// Typing starts suggesting words from this many characters; ⌃Space suggests at once.
const MIN_PREFIX: usize = 2;
/// Larger open files are left out of the word list: reading them on every key costs too much.
const MAX_OTHER_BYTES: usize = 2 << 20;

/// The project's open editors, as sources of words.
pub type OpenBuffers = Rc<RefCell<Vec<WeakEntity<EditorState>>>>;

pub struct WordCompletion {
    /// The language server's provider, whose items come first.
    inner: Option<Rc<dyn CompletionProvider>>,
    buffers: OpenBuffers,
    /// Suggest words while typing; prose only on ⌃Space.
    auto_popup: bool,
    /// The editor this provider belongs to: it is being updated while asking, so it is
    /// read through the `text` argument instead.
    own: EntityId,
}

impl WordCompletion {
    /// Puts word completion on `state` (an editor for `language`), wrapping the provider
    /// it already has.
    pub fn install(state: &mut EditorState, language: &str, own: EntityId, buffers: OpenBuffers) {
        let lsp = state.lsp_mut();
        let inner = lsp.completion_provider.take();
        lsp.completion_provider = Some(Rc::new(Self {
            inner,
            buffers,
            auto_popup: !is_prose(language),
            own,
        }));
    }

    fn other_texts(&self, cx: &App) -> Vec<Rope> {
        let mut buffers = self.buffers.borrow_mut();
        buffers.retain(|buffer| buffer.upgrade().is_some());
        buffers
            .iter()
            .filter(|buffer| buffer.entity_id() != self.own)
            .filter_map(WeakEntity::upgrade)
            .map(|buffer| buffer.read(cx).text().clone())
            .filter(|text| text.len() <= MAX_OTHER_BYTES)
            .collect()
    }
}

impl CompletionProvider for WordCompletion {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: lsp_types::CompletionContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let invoked = trigger.trigger_kind == CompletionTriggerKind::INVOKED;
        let auto_popup = self.auto_popup;
        let server = self
            .inner
            .as_ref()
            .map(|inner| inner.completions(text, offset, trigger, window, cx));
        let (text, others) = (text.clone(), self.other_texts(cx));
        cx.background_spawn(async move {
            let mut items = match server {
                Some(task) => match task.await {
                    Ok(CompletionResponse::Array(items)) => items,
                    Ok(CompletionResponse::List(list)) => list.items,
                    Err(_) => Vec::new(),
                },
                None => Vec::new(),
            };
            let current = text.to_string();
            let prefix = search::word_before(&current, offset);
            if invoked || (auto_popup && prefix.chars().count() >= MIN_PREFIX) {
                let others: Vec<String> = others.iter().map(Rope::to_string).collect();
                let others: Vec<&str> = others.iter().map(String::as_str).collect();
                let words = search::word_completions(prefix, &current, offset, &others, MAX_WORDS);
                let range = lsp_types::Range::new(
                    text.offset_to_position(offset - prefix.len()),
                    text.offset_to_position(offset),
                );
                items.extend(word_items(&items, words, range));
            }
            // Asked again on every key, so the list follows the typed prefix.
            Ok(CompletionResponse::List(lsp_types::CompletionList {
                is_incomplete: true,
                items,
            }))
        })
    }

    fn is_completion_trigger(&self, offset: usize, new_text: &str, cx: &mut App) -> bool {
        (self.auto_popup && new_text.chars().any(search::is_word_char))
            || self
                .inner
                .as_ref()
                .is_some_and(|inner| inner.is_completion_trigger(offset, new_text, cx))
    }
}

/// Prose, where a list popping up on every word would get in the way.
fn is_prose(language: &str) -> bool {
    matches!(language, "markdown" | "md" | "txt" | "text" | "plaintext" | "")
}

/// Items for `words` the server didn't already offer, replacing the typed prefix.
fn word_items(
    server: &[lsp_types::CompletionItem],
    words: Vec<String>,
    range: lsp_types::Range,
) -> Vec<lsp_types::CompletionItem> {
    let offered: HashSet<&str> = server
        .iter()
        .flat_map(|item| [Some(item.label.as_str()), item.insert_text.as_deref()])
        .flatten()
        .collect();
    words
        .into_iter()
        .filter(|word| !offered.contains(word.as_str()))
        .map(|word| lsp_types::CompletionItem {
            kind: Some(CompletionItemKind::TEXT),
            text_edit: Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
                range,
                new_text: word.clone(),
            })),
            label: word,
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use lsp::lsp_types::{CompletionItem, Position, Range};

    use super::word_items;

    #[test]
    fn words_the_server_offered_are_not_repeated() {
        let server = [
            CompletionItem {
                label: "total_price".into(),
                ..Default::default()
            },
            CompletionItem {
                label: "total(a, b)".into(),
                insert_text: Some("total".into()),
                ..Default::default()
            },
        ];
        let range = Range::new(Position::new(0, 0), Position::new(0, 3));
        let words = ["total_price", "total", "total_tax"].map(String::from).to_vec();
        let labels: Vec<String> = word_items(&server, words, range)
            .into_iter()
            .map(|item| item.label)
            .collect();
        assert_eq!(labels, ["total_tax"]);
    }
}
