//! Editor area: opening files, tabs, saving and JetBrains line operations.

use std::fs;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Position, RopeExt as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{MAX_RECENT_FILES, Workspace};
use crate::actions::*;
use crate::config::{AppConfig, LanguageServerConfig};
use crate::editor::{self, CenterTab, EditorTab, LineEdit};
use crate::preview::{self, Preview};
use crate::{file_icons, style};

impl Workspace {
    /// The active tab if it is a text editor.
    pub(super) fn active(&self) -> Option<&EditorTab> {
        self.active_tab
            .and_then(|ix| self.tabs.get(ix))
            .and_then(CenterTab::editor)
    }

    /// Opens `rel` (relative to the project root) and optionally moves the
    /// cursor to a 0-based (line, column).
    pub fn open_file(
        &mut self,
        rel: &str,
        position: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.root.join(rel);
        if path.is_dir() {
            return;
        }
        if preview::is_image(&path) {
            return self.open_image(rel, path, window, cx);
        }

        let ix = match self
            .tabs
            .iter()
            .position(|tab| tab.editor().is_some_and(|tab| tab.rel == rel))
        {
            Some(ix) => ix,
            None => {
                let text = match fs::read(&path) {
                    Ok(bytes) if bytes.contains(&0) => {
                        self.notify_error(format!("{rel} looks like a binary file"), window, cx);
                        return;
                    }
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(err) => {
                        self.notify_error(format!("Cannot open {rel}: {err}"), window, cx);
                        return;
                    }
                };
                let language = editor::language_for(&path);
                let tab_size = self
                    .project
                    .tab_size
                    .unwrap_or(cx.global::<AppConfig>().0.tab_size);
                let state = editor::new_editor_state(text.clone(), &language, tab_size, window, cx);
                let subscriptions = vec![
                    cx.subscribe(&state, |this, state, event: &InputEvent, cx| {
                        if let InputEvent::Change = event {
                            this.lsp_did_change(&state, cx);
                        }
                        cx.notify();
                    }),
                    cx.observe(&state, |_, _, cx| cx.notify()),
                ];
                let preview = preview::preview_kind(&path).map(Preview::new);
                self.tabs.push(CenterTab::Editor(EditorTab {
                    path,
                    rel: rel.to_string(),
                    state,
                    language,
                    saved_text: text.into(),
                    lsp: None,
                    preview,
                    breakpoint_marks: None,
                    execution_mark: None,
                    _subscriptions: subscriptions,
                }));
                let ix = self.tabs.len() - 1;
                self.attach_language_server(ix, cx);
                let path = self.root.join(rel);
                self.refresh_breakpoint_marks(&path, cx);
                ix
            }
        };

        self.record_location_if_leaving(rel, position.is_some(), cx);
        self.activate_tab(ix, window, cx);
        if let Some((line, column)) = position {
            self.goto_line(line as u32, column as u32, window, cx);
        }
    }

    pub(super) fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        self.active_tab = Some(ix);
        match tab {
            CenterTab::Editor(tab) => {
                let rel = tab.rel.clone();
                tab.state.update(cx, |state, cx| state.focus(window, cx));
                self.recent_files.retain(|r| *r != rel);
                self.recent_files.insert(0, rel);
                self.recent_files.truncate(MAX_RECENT_FILES);
            }
            CenterTab::Diff(tab) => tab.state.read(cx).focus_handle(cx).focus(window, cx),
            CenterTab::Settings | CenterTab::Conflicts | CenterTab::Image(_) => {
                self.focus_handle.focus(window, cx);
            }
            CenterTab::Console { console, .. } => console.update(cx, |console, cx| console.focus(window, cx)),
        }
        cx.notify();
    }

    pub fn goto_line(&mut self, line: u32, column: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active() else { return };
        tab.state.update(cx, |state, cx| {
            let last_line = state.text().lines_len().saturating_sub(1) as u32;
            state.set_cursor_position(Position::new(line.min(last_line), column), window, cx);
        });
    }

    /// Closes a tab without asking; see `unsaved::close_tab` for the guarded version.
    pub(super) fn close_tab_now(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        if let CenterTab::Editor(tab) = self.tabs.remove(ix) {
            Self::lsp_did_close(&tab);
        }
        self.active_tab = match self.active_tab {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > ix => Some(active - 1),
            Some(active) => Some(active.min(self.tabs.len() - 1)),
            None => None,
        };
        match self.active_tab {
            Some(active) => self.activate_tab(active, window, cx),
            None => self.focus_handle.focus(window, cx),
        }
        cx.notify();
    }

    pub(super) fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active() {
            Some(tab) => tab.state.update(cx, |state, cx| state.focus(window, cx)),
            None => self.focus_handle.focus(window, cx),
        }
    }

    pub(super) fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active_tab {
            self.save_tab(ix, window, cx);
        }
    }

    /// Writes an editor tab to disk; returns false (after notifying) on failure.
    pub(super) fn save_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(CenterTab::Editor(tab)) = self.tabs.get_mut(ix) else {
            return true;
        };
        let text = tab.state.read(cx).value();
        match fs::write(&tab.path, text.as_bytes()) {
            Ok(()) => {
                tab.saved_text = text;
                Self::lsp_did_save(tab);
                self.refresh_git(cx);
                cx.notify();
                true
            }
            Err(err) => {
                let message = format!("Cannot save {}: {err}", tab.rel);
                self.notify_error(message, window, cx);
                false
            }
        }
    }

    /// Applies a pure line operation to the active editor.
    pub(super) fn edit_lines(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        op: impl FnOnce(&str, u32, &str) -> Option<LineEdit>,
    ) {
        let Some(tab) = self.active() else { return };
        let language = tab.language.clone();
        tab.state.update(cx, |state, cx| {
            let cursor = state.cursor_position();
            let text = state.value();
            if let Some(edit) = op(&text, cursor.line, &language) {
                // `replace_all` scrolls to the top; keep the view where it was (⌘D, ⌘/, …).
                let scroll = state.scroll_offset();
                state.replace_all(edit.text, window, cx);
                state.set_cursor_position(Position::new(edit.cursor_line, cursor.character), window, cx);
                state.set_scroll_offset(scroll, cx);
            }
        });
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ix = match self.tabs.iter().position(|t| matches!(t, CenterTab::Settings)) {
            Some(ix) => ix,
            None => {
                self.tabs.push(CenterTab::Settings);
                self.tabs.len() - 1
            }
        };
        self.activate_tab(ix, window, cx);
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let title = if tab.is_dirty(cx) {
                format!("● {}", tab.title())
            } else {
                tab.title()
            };
            // The underline variant has no inner padding, so the label brings its own.
            let icon = tab
                .file_rel()
                .map(|_| file_icons::icon_for_file(&tab.title(), cx).with_size(px(14.)));
            let label = h_flex()
                .id(("tab-label", ix))
                .gap_1p5()
                .pl_3()
                .pr_1()
                .children(icon)
                .child(title);
            let label = match tab.file_rel().map(str::to_string) {
                Some(rel) => {
                    let workspace = cx.entity().downgrade();
                    label
                        .context_menu(move |menu, _, cx| match workspace.upgrade() {
                            Some(ws) => ws.read(cx).tab_context_menu(menu, &rel),
                            None => menu,
                        })
                        .into_any_element()
                }
                None => label.into_any_element(),
            };
            Tab::new().child(label).suffix(div().pr_1p5().child(
                style::icon_button(("close-tab", ix), IconName::Close, "Close (⌘W)").on_click(cx.listener(
                    move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab(ix, window, cx);
                    },
                )),
            ))
        });
        TabBar::new("editor-tabs")
            .underline()
            .pl_1()
            .children(tabs)
            .selected_index(self.active_tab.unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, window, cx| this.activate_tab(*ix, window, cx)))
    }

    pub(super) fn render_empty_state(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let hints: [(&str, Box<dyn Action>); 6] = [
            ("Search Everywhere", Box::new(SearchEverywhere)),
            ("Go to File", Box::new(GotoFile)),
            ("Recent Files", Box::new(RecentFiles)),
            ("Find in Files", Box::new(FindInFiles)),
            ("Find Action", Box::new(FindAction)),
            ("Open Project", Box::new(OpenProject)),
        ];
        let wordmark = v_flex()
            .items_center()
            .gap_3()
            .mb_6()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(style::signal_dot(theme.primary))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::LIGHT)
                            .text_color(theme.foreground)
                            .child("P O W E R   I D E A"),
                    ),
            )
            .child(style::fade_line(theme.primary, px(260.)));
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(wordmark)
            .children(hints.into_iter().map(|(label, action)| {
                let keys = if action.partial_eq(&SearchEverywhere) {
                    div().child("Double ⇧").into_any_element()
                } else {
                    Kbd::binding_for_action(action.as_ref(), Some(WORKSPACE_CONTEXT), window)
                        .map_or_else(|| div().into_any_element(), IntoElement::into_any_element)
                };
                h_flex().w(px(300.)).justify_between().child(label).child(keys)
            }))
    }

    /// Language server configured for `tab`'s language but not installed, if any.
    fn missing_language_server(&self, tab: &EditorTab, cx: &App) -> Option<LanguageServerConfig> {
        let language = editor::canonical_language(&tab.language);
        if self.dismissed_language_hints.contains(language) {
            return None;
        }
        let config = &cx.global::<AppConfig>().0;
        let server = config
            .language_servers
            .values()
            .find(|s| s.enabled && s.languages.iter().any(|l| l == language))?;
        crate::settings::find_in_path(&server.command)
            .is_none()
            .then(|| server.clone())
    }

    /// JetBrains-style banner suggesting the language server for this file type.
    pub(super) fn render_language_hint(&self, tab: &EditorTab, cx: &mut Context<Self>) -> Option<AnyElement> {
        let server = self.missing_language_server(tab, cx)?;
        let theme = cx.theme();
        let language = editor::canonical_language(&tab.language).to_string();
        let extension = tab
            .path
            .extension()
            .map_or_else(|| tab.title(), |e| format!("*.{}", e.to_string_lossy()));
        let hint = server.install_hint.clone();

        Some(
            h_flex()
                .px_3()
                .py_1p5()
                .gap_2()
                .text_sm()
                .bg(theme.primary.opacity(0.07))
                .border_b_1()
                .border_color(theme.primary.opacity(0.25))
                .child(style::signal_dot(theme.primary))
                .child(format!(
                    "Code insight for {extension} files needs {}.",
                    server.command
                ))
                .when(!hint.is_empty(), |el| {
                    el.child(
                        div()
                            .px_1p5()
                            .rounded_sm()
                            .bg(theme.muted)
                            .text_xs()
                            .font_family(theme.mono_font_family.clone())
                            .child(hint.clone()),
                    )
                })
                .child(div().flex_1())
                .when(!hint.is_empty(), |el| {
                    el.child(
                        Button::new("copy-install")
                            .small()
                            .label("Copy install command")
                            .on_click(move |_, window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(hint.clone()));
                                window.push_notification(
                                    Notification::success(
                                        "Run it in the terminal (⌥F12), then reopen the file.",
                                    ),
                                    cx,
                                );
                            }),
                    )
                })
                .child(
                    Button::new("language-settings")
                        .ghost()
                        .small()
                        .label("Settings")
                        .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
                )
                .child(
                    style::icon_button("dismiss-language-hint", IconName::Close, "Dismiss").on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.dismissed_language_hints.insert(language.clone());
                            cx.notify();
                        }),
                    ),
                )
                .into_any_element(),
        )
    }
}
