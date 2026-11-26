//! The remote folder tree: folders load when expanded, rows are flattened for the panel,
//! and the path bar moves the tree's root.

use std::collections::{HashMap, HashSet};

use gpui_kit::*;
use remote::Entry;
use remote::listing::{self, SortBy};

use super::{DirState, Workspace};

/// One visible row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Row<'a> {
    Entry {
        entry: &'a Entry,
        depth: usize,
    },
    /// A folder still loading or that failed, shown under its parent.
    Note {
        text: String,
        depth: usize,
    },
}

/// Flattens `root` and its expanded folders. The filter applies to the root folder only
/// (it filters "the current folder"); hidden files follow `show_hidden` everywhere.
pub(super) fn visible_rows<'a>(
    dirs: &'a HashMap<String, DirState>,
    expanded: &HashSet<String>,
    root: &str,
    show_hidden: bool,
    filter: &str,
) -> Vec<Row<'a>> {
    let mut rows = Vec::new();
    push_dir(dirs, expanded, root, 0, show_hidden, filter, &mut rows);
    rows
}

fn push_dir<'a>(
    dirs: &'a HashMap<String, DirState>,
    expanded: &HashSet<String>,
    dir: &str,
    depth: usize,
    show_hidden: bool,
    filter: &str,
    rows: &mut Vec<Row<'a>>,
) {
    match dirs.get(dir) {
        None | Some(DirState::Loading) => rows.push(Row::Note {
            text: "Loading…".into(),
            depth,
        }),
        Some(DirState::Failed(err)) => rows.push(Row::Note {
            text: err.clone(),
            depth,
        }),
        Some(DirState::Loaded(entries)) => {
            let shown = listing::visible(entries, show_hidden, if depth == 0 { filter } else { "" });
            if shown.is_empty() && depth == 0 {
                rows.push(Row::Note {
                    text: if filter.trim().is_empty() {
                        "Empty folder".into()
                    } else {
                        "Nothing matches the filter".into()
                    },
                    depth,
                });
            }
            for entry in shown {
                rows.push(Row::Entry { entry, depth });
                if entry.is_dir() && expanded.contains(&entry.path) {
                    push_dir(dirs, expanded, &entry.path, depth + 1, show_hidden, "", rows);
                }
            }
        }
    }
}

impl Workspace {
    /// Lists `dir` in the background (keeping the old listing visible meanwhile).
    pub(super) fn load_remote_dir(
        &mut self,
        site: &str,
        dir: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.remote.as_mut() else { return };
        let sort = state.sort;
        if let Some(conn) = state.conns.get_mut(site) {
            conn.dirs.entry(dir.to_string()).or_insert(DirState::Loading);
        }
        cx.notify();
        let (site_id, path) = (site.to_string(), dir.to_string());
        let listed = path.clone();
        self.run_remote_result(
            site,
            move |conn| {
                let mut entries = conn.run(|fs| fs.list(&listed))?;
                listing::sort_entries(&mut entries, sort);
                Ok(entries)
            },
            move |ws, result, window, cx| {
                let state = match result {
                    Ok(entries) => DirState::Loaded(entries),
                    Err(err) => {
                        // A dropped link is worth a notification; a folder we may not read
                        // is shown in place (and kept in the error journal).
                        if err.is_connection_lost() {
                            ws.remote_failed(&site_id, "Listing a folder", &err, window, cx);
                        } else {
                            super::journal(&err, &format!("listing {path}"));
                        }
                        DirState::Failed(err.user_message())
                    }
                };
                if let Some(conn) = ws.remote.as_mut().and_then(|s| s.conns.get_mut(&site_id)) {
                    conn.dirs.insert(path, state);
                }
                cx.notify();
            },
            window,
            cx,
        );
    }

    pub(super) fn toggle_remote_dir(
        &mut self,
        site: &str,
        dir: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(conn) = self.remote.as_mut().and_then(|s| s.conns.get_mut(site)) else {
            return;
        };
        if !conn.expanded.remove(dir) {
            conn.expanded.insert(dir.to_string());
            if !matches!(conn.dirs.get(dir), Some(DirState::Loaded(_))) {
                self.load_remote_dir(site, dir, window, cx);
            }
        }
        cx.notify();
    }

    /// Reloads the root and every expanded folder of `site`.
    pub(super) fn refresh_remote(&mut self, site: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conn) = self.remote.as_ref().and_then(|s| s.conns.get(site)) else {
            return;
        };
        let mut dirs: Vec<String> = conn.expanded.iter().cloned().collect();
        dirs.push(conn.root.clone());
        for dir in dirs {
            self.load_remote_dir(site, &dir, window, cx);
        }
    }

    /// Reloads `dir` if it is on screen (after a change inside it).
    pub(super) fn refresh_remote_dir(
        &mut self,
        site: &str,
        dir: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self
            .remote
            .as_ref()
            .and_then(|s| s.conns.get(site))
            .is_some_and(|c| c.root == dir || c.expanded.contains(dir) || c.dirs.contains_key(dir));
        if shown {
            self.load_remote_dir(site, dir, window, cx);
        }
    }

    /// Makes `dir` the tree's root.
    pub(super) fn set_remote_root(
        &mut self,
        site: &str,
        dir: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.remote.as_mut() else { return };
        let Some(conn) = state.conns.get_mut(site) else {
            return;
        };
        conn.root = dir.to_string();
        state.selected = None;
        let input = state.path_input.clone();
        input.update(cx, |input, cx| input.set_value(dir.to_string(), window, cx));
        state
            .filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.load_remote_dir(site, dir, window, cx);
    }

    /// Enter in the path bar: a folder becomes the root, a file opens.
    pub(super) fn go_to_typed_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.remote.as_ref() else { return };
        let Some(site) = state.active.clone() else { return };
        let base = state
            .conns
            .get(&site)
            .map_or_else(|| "/".into(), |c| c.root.clone());
        let typed = state.path_input.read(cx).value().to_string();
        let target = remote::path::normalize(&base, &typed);
        let site_id = site.clone();
        self.run_remote(
            &site,
            "Going to a path",
            {
                let target = target.clone();
                move |conn| conn.run(|fs| fs.stat(&target))
            },
            move |ws, entry, window, cx| {
                if entry.is_dir() {
                    ws.set_remote_root(&site_id, &target, window, cx);
                } else {
                    ws.set_remote_root(&site_id, &remote::path::parent(&target), window, cx);
                    ws.open_remote_file(&site_id, &target, window, cx);
                }
            },
            window,
            cx,
        );
    }

    pub(super) fn set_remote_sort(&mut self, sort: SortBy, cx: &mut Context<Self>) {
        let Some(state) = self.remote.as_mut() else { return };
        state.sort = sort;
        for conn in state.conns.values_mut() {
            for dir in conn.dirs.values_mut() {
                if let DirState::Loaded(entries) = dir {
                    listing::sort_entries(entries, sort);
                }
            }
        }
        cx.notify();
    }

    /// The listed entry at `path` on `site`, if its folder is loaded.
    pub(super) fn remote_entry(&self, site: &str, path: &str) -> Option<Entry> {
        let conn = self.remote.as_ref()?.conns.get(site)?;
        match conn.dirs.get(&remote::path::parent(path))? {
            DirState::Loaded(entries) => entries.iter().find(|e| e.path == path).cloned(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use remote::{Entry, EntryKind};

    use super::{DirState, Row, visible_rows};

    fn entry(path: &str, dir: bool) -> Entry {
        Entry {
            name: remote::path::file_name(path).into(),
            path: path.into(),
            kind: if dir { EntryKind::Dir } else { EntryKind::File },
            symlink: false,
            size: 1,
            modified: None,
            permissions: None,
            owner: None,
            group: None,
        }
    }

    fn names(rows: &[Row<'_>]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Entry { entry, depth } => format!("{depth}:{}", entry.name),
                Row::Note { text, depth } => format!("{depth}:({text})"),
            })
            .collect()
    }

    #[test]
    fn flattens_expanded_folders() {
        let mut dirs = HashMap::new();
        dirs.insert(
            "/srv".to_string(),
            DirState::Loaded(vec![
                entry("/srv/app", true),
                entry("/srv/logs", true),
                entry("/srv/.env", false),
                entry("/srv/readme.md", false),
            ]),
        );
        dirs.insert(
            "/srv/app".to_string(),
            DirState::Loaded(vec![entry("/srv/app/main.rb", false)]),
        );
        let expanded: HashSet<String> = ["/srv/app".to_string(), "/srv/logs".to_string()].into();
        let rows = visible_rows(&dirs, &expanded, "/srv", false, "");
        assert_eq!(
            names(&rows),
            ["0:app", "1:main.rb", "0:logs", "1:(Loading…)", "0:readme.md"]
        );
        let rows = visible_rows(&dirs, &expanded, "/srv", true, "env");
        assert_eq!(names(&rows), ["0:.env"]);
        let rows = visible_rows(&dirs, &expanded, "/srv", false, "zzz");
        assert_eq!(names(&rows), ["0:(Nothing matches the filter)"]);
    }
}
