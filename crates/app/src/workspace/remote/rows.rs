//! One row of the remote tree: icon, name, size, tooltip, click / double-click, context
//! menu, and drag and drop (rows drag out; folders accept local files).

use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use remote::listing::{format_age, format_mode, format_size, format_utc};
use remote::{Entry, Site};

use super::drag::{DragLabel, ProjectDrag, RemoteDrag};
use super::{RemoteState, SiteConn, Workspace};
use crate::{file_icons, style};

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_remote_row(
        &self,
        ix: usize,
        site: &Site,
        conn: &SiteConn,
        entry: &Entry,
        depth: usize,
        state: &RemoteState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let is_dir = entry.is_dir();
        let expanded = is_dir && conn.expanded.contains(&entry.path);
        let selected = state.selected.as_deref() == Some(entry.path.as_str());
        let chevron = div().flex_none().w(style::TREE_CHEVRON).when(is_dir, |slot| {
            slot.child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .with_size(style::TREE_CHEVRON)
                .text_color(theme.muted_foreground),
            )
        });
        let icon = if is_dir {
            Icon::new(if expanded {
                IconName::FolderOpen
            } else {
                IconName::Folder
            })
            .text_color(file_icons::folder_tint(&entry.name).color(cx))
        } else {
            file_icons::icon_for_file(&entry.name, cx)
        };
        let tooltip = entry_tooltip(entry);
        let (site_id, target) = (site.id.clone(), entry.path.clone());
        let menu_site = site.clone();
        let menu_entry = entry.clone();
        let workspace = cx.entity().downgrade();
        let drag = RemoteDrag {
            site: site.id.clone(),
            path: entry.path.clone(),
        };
        let size = (!is_dir).then(|| format_size(entry.size));
        let row = h_flex()
            .id(("remote-row", ix))
            .h(px(24.))
            .pl(px(8. + 16. * depth as f32))
            .pr_2()
            .gap_1p5()
            .text_sm()
            .cursor_pointer()
            .when(selected, |row| row.bg(theme.list_active))
            .hover(|s| s.bg(theme.list_hover))
            .child(chevron)
            .child(div().flex_none().child(icon.with_size(style::TREE_ICON)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .when(entry.is_hidden(), |name| name.text_color(theme.muted_foreground))
                    .when(entry.symlink, Styled::italic)
                    .child(entry.name.clone()),
            )
            .children(size.map(|s| {
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(s)
            }))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(cx.listener(move |ws, event: &ClickEvent, window, cx| {
                if let Some(state) = ws.remote.as_mut() {
                    state.selected = Some(target.clone());
                }
                if is_dir {
                    ws.toggle_remote_dir(&site_id, &target, window, cx);
                } else if event.click_count() >= 2 {
                    ws.open_remote_file(&site_id, &target, window, cx);
                }
                cx.notify();
            }))
            .on_drag(drag, |drag, _, _, cx| {
                cx.new(|_| DragLabel(remote::path::file_name(&drag.path).to_string().into()))
            });
        // Folders accept local files and project rows: they are uploaded into the folder.
        let row = if is_dir {
            let (finder_site, finder_dir) = (site.id.clone(), entry.path.clone());
            let (project_site, project_dir) = (site.id.clone(), entry.path.clone());
            row.on_drop(cx.listener(move |ws, paths: &ExternalPaths, window, cx| {
                cx.stop_propagation();
                ws.drop_local_on_remote(&finder_site, &finder_dir, paths.paths().to_vec(), window, cx);
            }))
            .on_drop(cx.listener(move |ws, drag: &ProjectDrag, window, cx| {
                cx.stop_propagation();
                let local = ws.root.join(&drag.rel);
                ws.drop_local_on_remote(&project_site, &project_dir, vec![local], window, cx);
            }))
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().drop_target))
            .drag_over::<ProjectDrag>(|style, _, _, cx| style.bg(cx.theme().drop_target))
        } else {
            row
        };
        row.context_menu(move |menu, _, cx| match workspace.upgrade() {
            Some(ws) => ws.read(cx).remote_row_menu(menu, &menu_site, &menu_entry),
            None => menu,
        })
        .into_any_element()
    }
}

/// Size, modification date, permissions and owner, for a row's tooltip.
fn entry_tooltip(entry: &Entry) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut lines = vec![entry.path.clone()];
    if !entry.is_dir() {
        lines.push(format!(
            "Size: {} ({} bytes)",
            format_size(entry.size),
            entry.size
        ));
    }
    if let Some(modified) = entry.modified {
        lines.push(format!(
            "Modified: {} ({})",
            format_utc(modified),
            format_age(modified, now)
        ));
    }
    if let Some(mode) = entry.permissions {
        lines.push(format!("Permissions: {} ({:03o})", format_mode(mode), mode));
    }
    match (&entry.owner, &entry.group) {
        (Some(owner), Some(group)) => lines.push(format!("Owner: {owner}:{group}")),
        (Some(owner), None) => lines.push(format!("Owner: {owner}")),
        _ => {}
    }
    if entry.symlink {
        lines.push("Symbolic link".into());
    }
    lines.join("\n")
}
