//! Enter, Tab and Backspace in the editor go through the workspace first, so they can act
//! as in JetBrains IDEs (see `language_rules`); anything else is the editor's own.

use gpui_kit::component::input::{Backspace, Delete, EditorState, Enter, IndentInline};
use gpui_kit::*;

use super::Workspace;
use crate::indentation::IndentStyle;
use crate::language_rules;

impl Workspace {
    /// The active editor if it has the keyboard, with its language and indentation.
    fn typing_editor(&self, window: &Window, cx: &App) -> Option<(Entity<EditorState>, String, IndentStyle)> {
        let tab = self.active()?;
        tab.state.read(cx).focus_handle(cx).is_focused(window).then(|| {
            let indent = self.indent_style(&tab.path, &tab.language, cx);
            (tab.state.clone(), tab.language.clone(), indent)
        })
    }

    pub(super) fn smart_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handled = self
            .typing_editor(window, cx)
            .is_some_and(|(state, language, indent)| {
                state.update(cx, |state, cx| {
                    language_rules::smart_enter(state, &language, indent, window, cx)
                })
            });
        if !handled {
            window.dispatch_action(
                Box::new(Enter {
                    secondary: false,
                    shift: false,
                }),
                cx,
            );
        }
    }

    /// Tab accepts the open completion item (as Enter does); otherwise it indents.
    pub(super) fn smart_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let menu_open = self.typing_editor(window, cx).is_some_and(|(state, ..)| {
            let state = state.read(cx);
            state.completion_menu_state().open
        });
        // Dispatched, not called: the editor handles them after this workspace update ends.
        let action: Box<dyn Action> = if menu_open {
            Box::new(Enter {
                secondary: false,
                shift: false,
            })
        } else {
            Box::new(IndentInline)
        };
        window.dispatch_action(action, cx);
    }

    pub(super) fn smart_backspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (forward, backward) = self
            .typing_editor(window, cx)
            .map_or((0, 1), |(state, _, indent)| {
                language_rules::backspace_keys(state.read(cx), indent)
            });
        for _ in 0..forward {
            window.dispatch_action(Box::new(Delete), cx);
        }
        for _ in 0..backward {
            window.dispatch_action(Box::new(Backspace), cx);
        }
    }
}
