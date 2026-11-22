//! Welcome screen, JetBrains style: shown when no project is open (on start
//! without a recent project, or after closing the last project window).
//! Lists recent projects and offers New Project / Open.

mod new_project;
mod paths;

pub use new_project::open_new_project_dialog;

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use search::fuzzy_filter;

use crate::config::AppConfig;
use crate::style;
use paths::tilde;

/// The open Welcome window, if any.
#[derive(Default)]
struct WelcomeWindow(Option<AnyWindowHandle>);

impl Global for WelcomeWindow {}

/// Shows the Welcome window, or brings the open one to front.
pub fn show(cx: &mut App) {
    if let Some(handle) = cx.try_global::<WelcomeWindow>().and_then(|w| w.0)
        && handle.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(880.), px(580.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(640.), px(420.))),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("Welcome to Power IDEA");
        cx.new(|cx| Welcome::new(window, cx))
    });
    match opened {
        Ok((handle, _)) => cx.set_global(WelcomeWindow(Some(handle))),
        Err(err) => eprintln!("failed to open the Welcome window: {err:#}"),
    }
}

/// Closes the Welcome window once a project window is open.
pub fn close(cx: &mut App) {
    let Some(handle) = cx.try_global::<WelcomeWindow>().and_then(|w| w.0) else {
        return;
    };
    cx.set_global(WelcomeWindow(None));
    handle.update(cx, |_, window, _| window.remove_window()).ok();
}

/// Forgets the Welcome window if `id` is it; returns whether it was.
pub fn forget_closed(id: WindowId, cx: &mut App) -> bool {
    let is_welcome = cx
        .try_global::<WelcomeWindow>()
        .and_then(|w| w.0)
        .is_some_and(|handle| handle.window_id() == id);
    if is_welcome {
        cx.set_global(WelcomeWindow(None));
    }
    is_welcome
}

/// Stable avatar color slot for a project name.
fn avatar_slot(name: &str, slots: usize) -> usize {
    name.bytes()
        .fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(b as usize))
        % slots
}

/// Up to two letters for the project avatar: `power-idea` → `PI`.
fn initials(name: &str) -> String {
    let mut words = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty());
    let first = words.next().and_then(|w| w.chars().next());
    let second = words.next().and_then(|w| w.chars().next());
    first.into_iter().chain(second).collect::<String>().to_uppercase()
}

pub struct Welcome {
    search: Entity<InputState>,
    _subscription: Subscription,
}

impl Welcome {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search projects"));
        let subscription = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        });
        search.update(cx, |input, cx| input.focus(window, cx));
        Self {
            search,
            _subscription: subscription,
        }
    }

    fn open(path: PathBuf, window: &mut Window, cx: &mut App) {
        if !path.is_dir() {
            let text = format!("{} no longer exists", path.display());
            return window.push_notification(Notification::error(text), cx);
        }
        crate::projects::open_project(path, cx);
    }

    fn forget(path: &PathBuf, cx: &mut App) {
        cx.global_mut::<AppConfig>()
            .0
            .recent_projects
            .retain(|p| p != path);
        crate::save_config(cx);
        cx.refresh_windows();
    }

    fn render_project(&self, ix: usize, path: PathBuf, home: &std::path::Path, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let name = path
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let palette = [
            theme.blue,
            theme.green,
            theme.magenta,
            theme.yellow,
            theme.cyan,
            theme.primary,
        ];
        let color = palette[avatar_slot(&name, palette.len())];
        let missing = !path.is_dir();
        let (open_path, forget_path) = (path.clone(), path.clone());
        h_flex()
            .id(("recent", ix))
            .group("recent")
            .px_3()
            .py_2()
            .gap_3()
            .rounded(theme.radius_lg)
            .cursor_pointer()
            .hover(|s| s.bg(theme.list_hover))
            .child(
                div()
                    .flex_none()
                    .size(px(32.))
                    .rounded(theme.radius)
                    .bg(color.opacity(0.18))
                    .text_color(color)
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(initials(&name)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(name))
                    .child(
                        div()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(if missing {
                                theme.danger
                            } else {
                                theme.muted_foreground
                            })
                            .child(tilde(&path, home))
                            .when(missing, |el| el.child(" · not found")),
                    ),
            )
            .child(div().invisible().group_hover("recent", |s| s.visible()).child(
                style::icon_button(("forget", ix), IconName::Close, "Remove from Recent Projects").on_click(
                    move |_, _, cx| {
                        cx.stop_propagation();
                        Self::forget(&forget_path, cx);
                    },
                ),
            ))
            .on_click(move |_, window, cx| Self::open(open_path.clone(), window, cx))
            .into_any_element()
    }
}

impl Render for Welcome {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let home = dirs::home_dir().unwrap_or_default();
        let query = self.search.read(cx).value().to_string();
        let recent = cx.global::<AppConfig>().0.recent_projects.clone();
        let has_recent = !recent.is_empty();
        let projects = fuzzy_filter(&query, recent, |p| p.to_str().unwrap_or_default());
        let rows: Vec<AnyElement> = projects
            .into_iter()
            .enumerate()
            .map(|(ix, path)| self.render_project(ix, path, &home, cx))
            .collect();
        let theme = cx.theme();

        let sidebar = v_flex()
            .w(px(220.))
            .flex_none()
            .p_4()
            .gap_4()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(
                h_flex().gap_2().child(style::signal_dot(theme.primary)).child(
                    v_flex()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Power IDEA"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(crate::APP_VERSION),
                        ),
                ),
            )
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .rounded(theme.radius)
                    .bg(theme.list_active)
                    .text_sm()
                    .child(Icon::new(IconName::Folder).small())
                    .child("Projects"),
            );

        let empty = v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_2()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(Icon::new(IconName::FolderOpen).text_color(theme.muted_foreground))
            .child(if has_recent {
                "No matching projects"
            } else {
                "Create a new project or open an existing folder"
            });

        let main = v_flex()
            .flex_1()
            .min_w_0()
            .px_6()
            .pt_5()
            .pb_4()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Projects"),
            )
            .child(
                // Input and buttons share the default (medium) height so the row lines up.
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .small()
                                        .text_color(theme.muted_foreground),
                                )
                                .cleanable(true),
                        ),
                    )
                    .child(
                        Button::new("new-project")
                            .primary()
                            .flex_none()
                            .icon(IconName::Plus)
                            .label("New Project")
                            .on_click(|_, window, cx| new_project::open_new_project_dialog(window, cx)),
                    )
                    .child(
                        Button::new("open-project")
                            .outline()
                            .flex_none()
                            .icon(IconName::FolderOpen)
                            .label("Open")
                            .on_click(|_, _, cx| crate::projects::prompt_open_project(cx)),
                    ),
            )
            .child(if rows.is_empty() {
                empty.into_any_element()
            } else {
                div()
                    .id("recent-projects")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(rows)
                    .into_any_element()
            });

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(TitleBar::new().child(div().text_sm().child("Welcome to Power IDEA")))
            .child(h_flex().flex_1().overflow_hidden().child(sidebar).child(main))
    }
}

#[cfg(test)]
mod tests {
    use super::{avatar_slot, initials};

    #[test]
    fn makes_avatars() {
        assert_eq!(initials("power-idea"), "PI");
        assert_eq!(initials("lms"), "L");
        assert_eq!(initials("my_cool app"), "MC");
        assert_eq!(initials("--"), "");
        assert_eq!(avatar_slot("lms", 6), avatar_slot("lms", 6));
        assert!(avatar_slot("power-idea", 6) < 6);
    }
}
