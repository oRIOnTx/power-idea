//! Replace in Files (⌘⇧R): find text across the project, preview every line
//! with the replacement applied, then replace all after a confirming click.
//! Open files are changed in their editors (left unsaved), others on disk.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use search::{TextMatch, TextSearchOptions, find_in_files, match_ranges, replace_all};

use super::Workspace;
use crate::editor::CenterTab;

const PREVIEW_LIMIT: usize = 2000;
const DEBOUNCE: Duration = Duration::from_millis(150);

pub struct ReplaceDialog {
    workspace: WeakEntity<Workspace>,
    root: PathBuf,
    exclude: Vec<String>,
    find: Entity<InputState>,
    replace: Entity<InputState>,
    case_sensitive: bool,
    whole_word: bool,
    /// Matching lines (first match per line from the search; all matches are replaced).
    matches: Vec<TextMatch>,
    searching: bool,
    /// The Replace All button was clicked once and now asks to confirm.
    confirming: bool,
    search_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl ReplaceDialog {
    fn new(
        workspace: WeakEntity<Workspace>,
        root: PathBuf,
        exclude: Vec<String>,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let find = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Find")
                .default_value(initial)
        });
        let replace = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
        let subscriptions = vec![
            cx.subscribe(&find, Self::on_input),
            cx.subscribe(&replace, Self::on_input),
        ];
        find.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            workspace,
            root,
            exclude,
            find,
            replace,
            case_sensitive: false,
            whole_word: false,
            matches: Vec::new(),
            searching: false,
            confirming: false,
            search_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.search(cx);
        this
    }

    fn on_input(&mut self, _: Entity<InputState>, event: &InputEvent, cx: &mut Context<Self>) {
        if let InputEvent::Change = event {
            self.confirming = false;
            self.search(cx);
        }
    }

    fn options(&self, limit: usize) -> TextSearchOptions {
        TextSearchOptions {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            limit,
        }
    }

    fn query(&self, cx: &App) -> String {
        self.find.read(cx).value().to_string()
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        if query.is_empty() {
            self.matches.clear();
            self.searching = false;
            return cx.notify();
        }
        let (root, exclude, options) = (
            self.root.clone(),
            self.exclude.clone(),
            self.options(PREVIEW_LIMIT),
        );
        self.searching = true;
        cx.notify();
        self.search_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let matches = cx
                .background_spawn(async move { find_in_files(&root, &query, options, &exclude) })
                .await;
            this.update(cx, |this, cx| {
                this.matches = matches;
                this.searching = false;
                cx.notify();
            })
            .ok();
        });
    }

    /// (occurrences, files) among the previewed lines.
    fn counts(&self, cx: &App) -> (usize, usize) {
        let query = self.query(cx);
        let occurrences = self
            .matches
            .iter()
            .map(|m| match_ranges(&m.line_text, &query, self.options(0)).len())
            .sum();
        let files = self
            .matches
            .iter()
            .map(|m| &m.path)
            .collect::<BTreeSet<_>>()
            .len();
        (occurrences, files)
    }

    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.confirming {
            self.confirming = true;
            return cx.notify();
        }
        let (query, replacement) = (self.query(cx), self.replace.read(cx).value().to_string());
        let (root, exclude, options) = (self.root.clone(), self.exclude.clone(), self.options(0));
        let workspace = self.workspace.clone();
        window.close_dialog(cx);
        // Search again without the preview limit, so every file is covered.
        cx.spawn_in(window, async move |_, cx| {
            let paths: BTreeSet<String> = cx
                .background_spawn({
                    let (root, query) = (root.clone(), query.clone());
                    async move {
                        find_in_files(&root, &query, options, &exclude)
                            .into_iter()
                            .map(|m| m.path)
                            .collect()
                    }
                })
                .await;
            workspace
                .update_in(cx, |ws, window, cx| {
                    ws.apply_replacement(paths, query, replacement, options, window, cx);
                })
                .ok();
        })
        .detach();
    }

    fn render_row(&self, ix: usize, query: &str, replacement: &str, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let Some(m) = self.matches.get(ix) else {
            return div().into_any_element();
        };
        let line = m.line_text.trim_start();
        // Show the line as it will read after replacing, replacements highlighted.
        let mut segments: Vec<AnyElement> = Vec::new();
        let mut cursor = 0;
        for range in match_ranges(line, query, self.options(0)) {
            segments.push(
                div()
                    .child(line[cursor..range.start].to_string())
                    .into_any_element(),
            );
            segments.push(
                div()
                    .rounded_sm()
                    .bg(theme.green.opacity(0.2))
                    .child(if replacement.is_empty() {
                        "∅".to_string()
                    } else {
                        replacement.to_string()
                    })
                    .into_any_element(),
            );
            cursor = range.end;
        }
        segments.push(div().child(line[cursor..].to_string()).into_any_element());
        let (path, line_no) = (m.path.clone(), m.line);
        let workspace = self.workspace.clone();
        h_flex()
            .id(("replace-row", ix))
            .h(px(24.))
            .px_2()
            .gap_3()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(theme.list_hover))
            .child(
                div()
                    .flex_none()
                    .w(px(220.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{path}:{line_no}")),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .font_family(theme.mono_font_family.clone())
                    .children(segments),
            )
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                workspace
                    .update(cx, |ws, cx| {
                        ws.open_file(&path, Some((line_no - 1, 0)), window, cx);
                    })
                    .ok();
            })
            .into_any_element()
    }
}

impl Render for ReplaceDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (query, replacement) = (self.query(cx), self.replace.read(cx).value().to_string());
        let (occurrences, files) = self.counts(cx);
        let rows: Vec<AnyElement> = (0..self.matches.len())
            .map(|ix| self.render_row(ix, &query, &replacement, cx))
            .collect();
        let theme = cx.theme();
        let summary = if self.searching {
            "Searching…".to_string()
        } else if query.is_empty() {
            "Type the text to find".to_string()
        } else {
            let more = if self.matches.len() >= PREVIEW_LIMIT {
                "+"
            } else {
                ""
            };
            format!("{occurrences}{more} matches in {files} files (open files are searched as saved)")
        };
        let label = if self.confirming {
            format!("Confirm: replace in {files} files")
        } else {
            "Replace All".to_string()
        };
        v_flex()
            .w_full()
            .gap_2()
            .child(Input::new(&self.find))
            .child(Input::new(&self.replace))
            .child(
                h_flex()
                    .gap_4()
                    .text_sm()
                    .child(
                        Checkbox::new("replace-case")
                            .label("Match case")
                            .checked(self.case_sensitive)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.case_sensitive = *checked;
                                this.confirming = false;
                                this.search(cx);
                            })),
                    )
                    .child(
                        Checkbox::new("replace-word")
                            .label("Words")
                            .checked(self.whole_word)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.whole_word = *checked;
                                this.confirming = false;
                                this.search(cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(theme.muted_foreground).child(summary)),
            )
            .child(
                div()
                    .id("replace-results")
                    .h(px(360.))
                    .overflow_y_scroll()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .children(rows),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        crate::style::dialog_button("replace-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        crate::style::dialog_button("replace-all")
                            .label(label)
                            .when_confirming(self.confirming)
                            .disabled(self.matches.is_empty() || self.searching)
                            .on_click(cx.listener(|this, _, window, cx| this.replace_all(window, cx))),
                    ),
            )
    }
}

/// Primary style normally, danger while asking to confirm.
trait ConfirmStyle {
    fn when_confirming(self, confirming: bool) -> Self;
}

impl ConfirmStyle for Button {
    fn when_confirming(self, confirming: bool) -> Self {
        if confirming { self.danger() } else { self.primary() }
    }
}

impl Workspace {
    /// ⌘⇧R; the editor's single-line selection pre-fills the search.
    pub(super) fn open_replace_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let initial = self
            .active()
            .map(|tab| tab.state.read(cx).selected_value().to_string())
            .filter(|s| !s.is_empty() && !s.contains('\n'))
            .unwrap_or_default();
        let (workspace, root, exclude) = (
            cx.entity().downgrade(),
            self.root.clone(),
            self.project.exclude.clone(),
        );
        let dialog = cx.new(|cx| ReplaceDialog::new(workspace, root, exclude, initial, window, cx));
        window.open_dialog(cx, move |d, _, _| {
            d.w(px(860.)).title("Replace in Files").child(dialog.clone())
        });
    }

    fn apply_replacement(
        &mut self,
        paths: BTreeSet<String>,
        query: String,
        replacement: String,
        options: TextSearchOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut on_disk = Vec::new();
        let (mut total, mut files) = (0, 0);
        for rel in paths {
            let open = self.tabs.iter().find_map(|tab| match tab {
                CenterTab::Editor(tab) if tab.rel == rel => Some(tab.state.clone()),
                _ => None,
            });
            match open {
                Some(state) => {
                    let (text, count) = replace_all(&state.read(cx).value(), &query, &replacement, options);
                    if count > 0 {
                        state.update(cx, |state, cx| {
                            let cursor = state.cursor_position();
                            state.set_value(text, window, cx);
                            state.set_cursor_position(cursor, window, cx);
                        });
                        total += count;
                        files += 1;
                    }
                }
                None => on_disk.push(self.root.join(&rel)),
            }
        }
        cx.spawn_in(window, async move |_, cx| {
            let written = cx
                .background_spawn(async move {
                    let (mut total, mut files, mut errors) = (0, 0, Vec::new());
                    for path in on_disk {
                        let result = std::fs::read_to_string(&path).and_then(|text| {
                            let (text, count) = replace_all(&text, &query, &replacement, options);
                            if count > 0 {
                                std::fs::write(&path, text)?;
                            }
                            Ok(count)
                        });
                        match result {
                            Ok(0) => {}
                            Ok(count) => {
                                total += count;
                                files += 1;
                            }
                            Err(err) => errors.push(format!("{}: {err}", path.display())),
                        }
                    }
                    (total, files, errors)
                })
                .await;
            cx.update(|window, cx| {
                let (disk_total, disk_files, errors) = written;
                let (total, files) = (total + disk_total, files + disk_files);
                let notification = if errors.is_empty() {
                    Notification::success(format!("Replaced {total} occurrences in {files} files"))
                } else {
                    Notification::error(errors.join("\n"))
                        .title(format!("Replaced {total}; some files failed"))
                };
                window.push_notification(notification, cx);
            })
            .ok();
        })
        .detach();
    }
}
