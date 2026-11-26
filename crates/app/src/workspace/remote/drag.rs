//! Drag and drop between the project tree, Finder and the remote tree: dropping local
//! files on a remote folder uploads them, dropping a remote row on a project folder
//! downloads it there.

use std::path::PathBuf;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use super::Workspace;

/// A project tree row being dragged (path relative to the project root).
#[derive(Clone, Debug)]
pub(in crate::workspace) struct ProjectDrag {
    pub(in crate::workspace) rel: String,
}

/// A remote tree row being dragged.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct RemoteDrag {
    pub(in crate::workspace) site: String,
    pub(in crate::workspace) path: String,
}

/// The label that follows the pointer while dragging.
pub(in crate::workspace) struct DragLabel(pub(in crate::workspace) SharedString);

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_0p5()
            .rounded_md()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .text_color(theme.popover_foreground)
            .child(self.0.clone())
    }
}

impl Workspace {
    /// Local files dropped on the remote folder `dir` (from Finder or the project tree).
    pub(super) fn drop_local_on_remote(
        &mut self,
        site: &str,
        dir: &str,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for local in paths {
            self.queue_upload(site, local, dir, window, cx);
        }
    }

    /// A remote row dropped on the project folder `rel` ("" = the root).
    pub(in crate::workspace) fn drop_remote_on_project(
        &mut self,
        drag: &RemoteDrag,
        rel: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folder = if rel.is_empty() {
            self.root.clone()
        } else {
            let path = self.root.join(rel);
            if path.is_dir() {
                path
            } else {
                path.parent()
                    .map_or_else(|| self.root.clone(), std::path::Path::to_path_buf)
            }
        };
        self.queue_download(&drag.site, &drag.path, folder, window, cx);
    }
}
