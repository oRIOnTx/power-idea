//! Multi-selection in the project tree (⌘+click toggles, ⇧+click selects a range),
//! kept as paths relative to the project root so it survives tree rebuilds.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeSelection {
    /// Selected paths in the order they were selected.
    pub paths: Vec<String>,
    /// Where a ⇧+click range starts.
    anchor: Option<String>,
}

impl TreeSelection {
    pub fn contains(&self, path: &str) -> bool {
        self.paths.iter().any(|p| p == path)
    }

    pub fn select_only(&mut self, path: &str) {
        self.paths = vec![path.to_string()];
        self.anchor = Some(path.to_string());
    }

    /// Selects `paths` (e.g. just pasted ones); the first is the range anchor.
    pub fn set(&mut self, paths: Vec<String>) {
        self.anchor = paths.first().cloned();
        self.paths = paths;
    }

    /// ⌘+click: adds or removes `path`; it becomes the range anchor.
    pub fn toggle(&mut self, path: &str) {
        if self.contains(path) {
            self.paths.retain(|p| p != path);
        } else {
            self.paths.push(path.to_string());
        }
        self.anchor = Some(path.to_string());
    }

    /// ⇧+click: selects the visible rows from the anchor to `path`.
    pub fn extend_to(&mut self, path: &str, visible: &[String]) {
        let target = visible.iter().position(|p| p == path);
        let anchor = self
            .anchor
            .as_ref()
            .and_then(|anchor| visible.iter().position(|p| p == anchor));
        let (Some(target), Some(anchor)) = (target, anchor) else {
            self.select_only(path);
            return;
        };
        let range = if anchor <= target {
            anchor..=target
        } else {
            target..=anchor
        };
        self.paths = visible.get(range).map_or_else(Vec::new, <[String]>::to_vec);
    }

    /// Drops paths that no longer exist.
    pub fn retain(&mut self, exists: impl Fn(&str) -> bool) {
        self.paths.retain(|p| exists(p));
        if self.anchor.as_deref().is_some_and(|a| !exists(a)) {
            self.anchor = None;
        }
    }
}

/// `paths` without the ones inside another selected folder (copying the folder copies them).
pub fn outermost(paths: &[String]) -> Vec<String> {
    let inside = |path: &str, dir: &str| path.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'));
    let mut result: Vec<String> = Vec::new();
    for path in paths {
        if result.contains(path) || paths.iter().any(|other| inside(path, other)) {
            continue;
        }
        result.push(path.clone());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{TreeSelection, outermost};

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn toggles_and_extends_ranges() {
        let visible = strings(&["a", "b", "c", "d", "e"]);
        let mut selection = TreeSelection::default();
        selection.select_only("b");
        selection.extend_to("d", &visible);
        assert_eq!(selection.paths, strings(&["b", "c", "d"]));
        // The anchor stays, so a second ⇧+click re-ranges from it.
        selection.extend_to("a", &visible);
        assert_eq!(selection.paths, strings(&["a", "b"]));

        selection.toggle("e");
        assert_eq!(selection.paths, strings(&["a", "b", "e"]));
        selection.toggle("a");
        assert_eq!(selection.paths, strings(&["b", "e"]));
        // ⌘+click moved the anchor to "a".
        selection.extend_to("c", &visible);
        assert_eq!(selection.paths, strings(&["a", "b", "c"]));
    }

    #[test]
    fn extends_from_nothing_as_a_single_click() {
        let mut selection = TreeSelection::default();
        selection.extend_to("b", &strings(&["a", "b"]));
        assert_eq!(selection.paths, strings(&["b"]));
    }

    #[test]
    fn retains_existing_paths() {
        let mut selection = TreeSelection::default();
        selection.select_only("gone");
        selection.toggle("kept");
        selection.toggle("gone");
        selection.toggle("gone");
        selection.retain(|p| p == "kept");
        assert_eq!(selection.paths, strings(&["kept"]));
        selection.extend_to("kept", &strings(&["kept"]));
        assert_eq!(selection.paths, strings(&["kept"]));
    }

    #[test]
    fn keeps_only_outermost_paths() {
        assert_eq!(
            outermost(&strings(&[
                "src/a.rs",
                "src",
                "srcx/b.rs",
                "src/ui/c.rs",
                "srcx/b.rs"
            ])),
            strings(&["src", "srcx/b.rs"])
        );
    }
}
