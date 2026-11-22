//! Project tree selection: plain, ⌘ and ⇧ clicks, and arrow-key moves of the
//! tree's own cursor, combined into one multi-selection (`crate::tree_selection`).

use gpui_kit::component::tree::TreeState;
use gpui_kit::*;

use super::Workspace;

impl Workspace {
    /// Handles a click on tree row `ix` (`rel` relative to the root). Returns `true` when
    /// the click only changed the selection, so the tree must not also open or toggle it.
    pub(super) fn click_tree_entry(
        &mut self,
        ix: usize,
        rel: &str,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        let selection_only = modifiers.platform || modifiers.shift;
        if modifiers.platform {
            self.tree_selection.toggle(rel);
        } else if modifiers.shift {
            let visible = self.visible_tree_paths(cx);
            self.tree_selection.extend_to(rel, &visible);
        } else {
            self.tree_selection.select_only(rel);
        }
        self.tree_cursor = Some(rel.to_string());
        self.root_selected = false;
        if selection_only {
            // The tree selects the row itself only on clicks it handles; a row ⌘-clicked
            // out of the selection must not stay highlighted as the tree's cursor.
            let cursor = self.tree_selection.contains(rel).then_some(ix);
            self.tree
                .update(cx, |tree, cx| tree.set_selected_index(cursor, cx));
        }
        cx.notify();
        selection_only
    }

    /// Arrow keys move the tree's cursor; a row we did not put it on starts a new selection.
    pub(super) fn follow_tree_cursor(&mut self, tree: &Entity<TreeState>, cx: &mut Context<Self>) {
        let Some(id) = tree.read(cx).selected_item().map(|item| item.id.to_string()) else {
            return;
        };
        if self.tree_cursor.as_deref() == Some(id.as_str()) {
            return;
        }
        self.tree_selection.select_only(&id);
        self.tree_cursor = Some(id);
        self.root_selected = false;
        cx.notify();
    }

    fn visible_tree_paths(&self, cx: &App) -> Vec<String> {
        let tree = self.tree.read(cx);
        (0..)
            .map_while(|ix| tree.entry(ix))
            .map(|entry| entry.item().id.to_string())
            .collect()
    }
}

impl Workspace {
    /// The root row is highlighted only while no tree row is selected.
    pub(super) fn root_row_selected(&self) -> bool {
        self.root_selected && self.tree_selection.paths.is_empty()
    }

    /// A click on the root row selects it, so ⌘V pastes into the project root.
    pub(super) fn select_root_row(&mut self, cx: &mut Context<Self>) {
        self.tree_selection.set(Vec::new());
        self.tree_cursor = None;
        self.root_selected = true;
        self.tree.update(cx, |tree, cx| tree.set_selected_index(None, cx));
        cx.notify();
    }
}
