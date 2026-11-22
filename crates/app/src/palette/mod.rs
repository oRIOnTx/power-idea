//! Modal picker used by Search Everywhere, Go to File, Recent Files,
//! Find in Files, Find Action, Go to Line, Recent Projects and the
//! code navigation pickers (usages, file structure, symbols).

mod locations;

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use search::{FileIndex, TextMatch, TextSearchOptions, find_in_files, fuzzy_filter};
use vcs::Branch;

use crate::actions::{self, PALETTE_CONTEXT, SelectNext, SelectPrev, WORKSPACE_CONTEXT};
use crate::workspace::Workspace;
pub use locations::{LocationItem, SymbolSearch};

const MAX_RESULTS: usize = 200;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    SearchEverywhere,
    GotoFile,
    RecentFiles,
    FindInFiles,
    FindAction,
    GotoLine,
    RecentProjects,
    Branches,
    Usages,
    FileStructure,
    GotoSymbol,
    GotoClass,
}

impl PaletteMode {
    fn placeholder(self) -> &'static str {
        match self {
            Self::SearchEverywhere => "Search everywhere: files and actions",
            Self::GotoFile => "Go to file",
            Self::RecentFiles => "Recent files",
            Self::FindInFiles => "Find in files",
            Self::FindAction => "Find action",
            Self::GotoLine => "Go to line[:column]",
            Self::RecentProjects => "Recent projects",
            Self::Branches => "Checkout branch, or type a name to create one",
            Self::Usages => "Usages",
            Self::FileStructure => "File structure",
            Self::GotoSymbol => "Go to symbol",
            Self::GotoClass => "Go to class",
        }
    }
}

/// Snapshot of workspace state the palette searches over.
pub struct PaletteSource {
    pub workspace: WeakEntity<Workspace>,
    pub root: PathBuf,
    pub index: Arc<FileIndex>,
    pub recent_files: Vec<String>,
    pub recent_projects: Vec<PathBuf>,
    pub branches: Vec<Branch>,
    /// Project paths excluded from search.
    pub exclude: Vec<String>,
    /// Fixed results of Usages and File Structure.
    pub locations: Vec<LocationItem>,
    /// Live symbol lookup for Go to Symbol / Class.
    pub symbol_search: Option<SymbolSearch>,
}

enum PaletteItem {
    File(String),
    Text(TextMatch),
    Action {
        label: &'static str,
        action: Box<dyn Action>,
    },
    Project(PathBuf),
    Branch(Branch),
    NewBranch(String),
    Location(LocationItem),
}

pub struct Palette {
    mode: PaletteMode,
    source: PaletteSource,
    input: Entity<InputState>,
    items: Vec<PaletteItem>,
    selected: usize,
    scroll: UniformListScrollHandle,
    search_task: Task<()>,
    focused_once: bool,
    _subscriptions: Vec<Subscription>,
}

impl Palette {
    pub fn new(
        mode: PaletteMode,
        source: PaletteSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(mode.placeholder()));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.update_query(cx),
                    InputEvent::PressEnter { .. } => this.confirm(window, cx),
                    _ => {}
                },
            );
        let mut palette = Self {
            mode,
            source,
            input,
            items: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            search_task: Task::ready(()),
            focused_once: false,
            _subscriptions: vec![subscription],
        };
        palette.update_query(cx);
        palette
    }

    /// Opens the palette as a modal dialog in `window`.
    pub fn open(mode: PaletteMode, source: PaletteSource, window: &mut Window, cx: &mut App) {
        let palette = cx.new(|cx| Palette::new(mode, source, window, cx));
        window.open_dialog(cx, move |dialog, _, cx| {
            let accent = cx.theme().primary;
            dialog
                .w(px(720.))
                .margin_top(px(72.))
                .p_0()
                .border_1()
                .border_color(accent.opacity(0.35))
                .shadow(crate::style::glow(accent, 1.6))
                .close_button(false)
                .child(palette.clone())
        });
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_string()
    }

    fn update_query(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.selected = 0;
        match self.mode {
            PaletteMode::GotoFile => self.items = self.files(&query),
            PaletteMode::RecentFiles => {
                let recent = self.source.recent_files.clone();
                self.items = fuzzy_filter(&query, recent, |p| p.as_str())
                    .into_iter()
                    .map(PaletteItem::File)
                    .collect();
            }
            PaletteMode::FindAction => self.items = actions_matching(&query),
            PaletteMode::SearchEverywhere => {
                let mut items = actions_matching(&query);
                items.truncate(if query.is_empty() { 0 } else { 8 });
                let mut files = self.files(&query);
                files.append(&mut items);
                self.items = files;
            }
            PaletteMode::RecentProjects => {
                let projects = self.source.recent_projects.clone();
                self.items = fuzzy_filter(&query, projects, |p| p.to_str().unwrap_or_default())
                    .into_iter()
                    .map(PaletteItem::Project)
                    .collect();
            }
            PaletteMode::GotoLine => self.items.clear(),
            PaletteMode::Branches => {
                let branches = self.source.branches.clone();
                let exact = branches.iter().any(|b| b.name == query);
                let mut items: Vec<PaletteItem> = fuzzy_filter(&query, branches, |b| b.name.as_str())
                    .into_iter()
                    .map(PaletteItem::Branch)
                    .collect();
                if !query.is_empty() && !exact && !query.contains(char::is_whitespace) {
                    items.push(PaletteItem::NewBranch(query));
                }
                self.items = items;
            }
            PaletteMode::Usages | PaletteMode::FileStructure => {
                let locations = self.source.locations.clone();
                self.items = fuzzy_filter(&query, locations, |l| l.label.as_str())
                    .into_iter()
                    .map(PaletteItem::Location)
                    .collect();
            }
            PaletteMode::GotoSymbol | PaletteMode::GotoClass => {
                let Some(search) = self.source.symbol_search.clone() else {
                    return;
                };
                if query.is_empty() {
                    self.items.clear();
                    self.search_task = Task::ready(());
                    return cx.notify();
                }
                self.search_task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SEARCH_DEBOUNCE).await;
                    let found = cx.background_spawn(async move { search(&query) }).await;
                    this.update(cx, |this, cx| {
                        this.items = found.into_iter().map(PaletteItem::Location).collect();
                        this.selected = 0;
                        cx.notify();
                    })
                    .ok();
                });
            }
            PaletteMode::FindInFiles => {
                let root = self.source.root.clone();
                let exclude = self.source.exclude.clone();
                self.search_task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SEARCH_DEBOUNCE).await;
                    let matches = cx
                        .background_spawn(async move {
                            let options = TextSearchOptions {
                                limit: MAX_RESULTS,
                                ..Default::default()
                            };
                            find_in_files(&root, &query, options, &exclude)
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        this.items = matches.into_iter().map(PaletteItem::Text).collect();
                        this.selected = 0;
                        cx.notify();
                    })
                    .ok();
                });
            }
        }
        cx.notify();
    }

    fn files(&self, query: &str) -> Vec<PaletteItem> {
        self.source
            .index
            .fuzzy(query, MAX_RESULTS)
            .into_iter()
            .map(|m| PaletteItem::File(m.path))
            .collect()
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1) % self.items.len();
            self.scroll.scroll_to_item(self.selected, ScrollStrategy::Top);
            cx.notify();
        }
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        if !self.items.is_empty() {
            self.selected = self.selected.checked_sub(1).unwrap_or(self.items.len() - 1);
            self.scroll.scroll_to_item(self.selected, ScrollStrategy::Top);
            cx.notify();
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.source.workspace.clone();
        if self.mode == PaletteMode::GotoLine {
            let Some((line, column)) = parse_line_column(&self.query(cx)) else {
                return;
            };
            window.close_dialog(cx);
            workspace
                .update(cx, |ws, cx| ws.goto_line(line, column, window, cx))
                .ok();
            return;
        }

        let Some(item) = self.items.get(self.selected) else {
            return;
        };
        window.close_dialog(cx);
        match item {
            PaletteItem::File(path) => {
                let path = path.clone();
                workspace
                    .update(cx, |ws, cx| ws.open_file(&path, None, window, cx))
                    .ok();
            }
            PaletteItem::Text(m) => {
                let (path, pos) = (m.path.clone(), (m.line - 1, m.column - 1));
                workspace
                    .update(cx, |ws, cx| ws.open_file(&path, Some(pos), window, cx))
                    .ok();
            }
            PaletteItem::Action { action, .. } => {
                let action = action.boxed_clone();
                workspace
                    .update(cx, |ws, cx| ws.dispatch_deferred(action, window, cx))
                    .ok();
            }
            PaletteItem::Project(path) => crate::projects::open_project(path.clone(), cx),
            PaletteItem::Branch(branch) => {
                let branch = branch.clone();
                workspace
                    .update(cx, |ws, cx| ws.checkout(branch, window, cx))
                    .ok();
            }
            PaletteItem::NewBranch(name) => {
                let name = name.clone();
                workspace
                    .update(cx, |ws, cx| ws.create_branch(name, window, cx))
                    .ok();
            }
            PaletteItem::Location(item) => {
                let item = item.clone();
                workspace
                    .update(cx, |ws, cx| {
                        ws.jump_to(&item.path, item.line, item.column, window, cx);
                    })
                    .ok();
            }
        }
    }

    fn render_item(&self, ix: usize, window: &Window, cx: &App) -> ListItem {
        let theme = cx.theme();
        let row = match &self.items[ix] {
            PaletteItem::File(path) => {
                let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
                h_flex()
                    .gap_2()
                    .child(div().text_color(theme.foreground).child(name.to_string()))
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .text_xs()
                            .child(dir.to_string()),
                    )
            }
            PaletteItem::Text(m) => h_flex()
                .w_full()
                .gap_3()
                .justify_between()
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .text_ellipsis()
                        .font_family(theme.mono_font_family.clone())
                        .child(m.line_text.trim().to_string()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("{}:{}", m.path, m.line)),
                ),
            PaletteItem::Action { label, action } => h_flex()
                .w_full()
                .justify_between()
                .child(*label)
                .children(Kbd::binding_for_action(
                    action.as_ref(),
                    Some(WORKSPACE_CONTEXT),
                    window,
                )),
            PaletteItem::Project(path) => {
                let name = path
                    .file_name()
                    .map_or_default(|n| n.to_string_lossy().into_owned());
                h_flex().gap_2().child(name).child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(path.display().to_string()),
                )
            }
            PaletteItem::Branch(branch) => h_flex()
                .w_full()
                .gap_2()
                .child(if branch.is_current { "● " } else { "  " })
                .child(branch.name.clone())
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(if branch.is_remote { "remote" } else { "local" }),
                ),
            PaletteItem::NewBranch(name) => h_flex()
                .gap_2()
                .child(div().text_color(theme.primary).child("+ New branch"))
                .child(name.clone()),
            PaletteItem::Location(item) => {
                let show_place = self.mode != PaletteMode::FileStructure;
                locations::render_location(item, &self.source.root, show_place, cx)
            }
        };
        ListItem::new(ix)
            .selected(ix == self.selected)
            .py_1()
            .px_3()
            .child(row)
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
        let theme = cx.theme();
        let empty_hint = match self.mode {
            PaletteMode::GotoLine => Some("Type a line number and press Enter"),
            _ if self.items.is_empty() && !self.query(cx).is_empty() => Some("Nothing found"),
            _ => None,
        };

        v_flex()
            .key_context(PALETTE_CONTEXT)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .w_full()
            .child(
                div()
                    .p_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(Input::new(&self.input).appearance(false).cleanable(false)),
            )
            .child(
                uniform_list(
                    "palette-results",
                    self.items.len(),
                    cx.processor(|this, range: Range<usize>, window, cx| {
                        range
                            .map(|ix| {
                                this.render_item(ix, window, cx).on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.selected = ix;
                                        this.confirm(window, cx);
                                    },
                                ))
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll)
                .h(px(if self.items.is_empty() { 0. } else { 420. }))
                .py_1(),
            )
            .children(empty_hint.map(|hint| {
                div()
                    .p_3()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(hint)
            }))
    }
}

fn actions_matching(query: &str) -> Vec<PaletteItem> {
    fuzzy_filter(query, actions::palette_actions(), |(label, _)| label)
        .into_iter()
        .map(|(label, action)| PaletteItem::Action { label, action })
        .collect()
}

/// Parses `"42"` or `"42:7"` (1-based) into 0-based (line, column).
fn parse_line_column(query: &str) -> Option<(u32, u32)> {
    let mut parts = query.split(':');
    let line: u32 = parts.next()?.trim().parse().ok()?;
    let column: u32 = parts.next().and_then(|c| c.trim().parse().ok()).unwrap_or(1);
    Some((line.saturating_sub(1), column.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::parse_line_column;

    #[test]
    fn parses_line_and_column() {
        assert_eq!(parse_line_column("42"), Some((41, 0)));
        assert_eq!(parse_line_column("42:7"), Some((41, 6)));
        assert_eq!(parse_line_column("x"), None);
    }
}
