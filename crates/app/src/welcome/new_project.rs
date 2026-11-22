//! "New Project" dialog: creates an empty folder (optionally a git
//! repository) and opens it.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;

use super::paths::{default_location, expand_tilde, new_project_dir, tilde};
use crate::config::AppConfig;

pub fn open_new_project_dialog(window: &mut Window, cx: &mut App) {
    let home = dirs::home_dir().unwrap_or_default();
    let recent = cx.global::<AppConfig>().0.recent_projects.clone();
    let location_text = tilde(&default_location(&recent, &home), &home);
    let name = cx.new(|cx| InputState::new(window, cx).default_value("untitled"));
    let location = cx.new(|cx| InputState::new(window, cx).default_value(location_text));
    let git = cx.new(|_| true);
    let error: Entity<Option<String>> = cx.new(|_| None);

    let create = {
        let (name, location, git, error) = (name.clone(), location.clone(), git.clone(), error.clone());
        Rc::new(move |window: &mut Window, cx: &mut App| {
            let home = dirs::home_dir().unwrap_or_default();
            let target = expand_tilde(&location.read(cx).value(), &home);
            match new_project_dir(&target, &name.read(cx).value()) {
                Ok(dir) => {
                    window.close_dialog(cx);
                    create_project(dir, *git.read(cx), window, cx);
                }
                Err(err) => error.update(cx, |e, cx| {
                    *e = Some(format!("{err:#}"));
                    cx.notify();
                }),
            }
        })
    };

    let focused = std::cell::Cell::new(false);
    window.open_dialog(cx, move |dialog, window, cx| {
        if !focused.replace(true) {
            name.update(cx, |input, cx| input.focus(window, cx));
        }
        let theme = cx.theme();
        let label = |text: &'static str| div().text_sm().text_color(theme.muted_foreground).child(text);
        let browse_location = location.clone();
        let create = create.clone();
        dialog
            .w(px(520.))
            .title("New Project")
            .child(
                v_flex()
                    .gap_3()
                    .child(v_flex().gap_1().child(label("Name")).child(Input::new(&name)))
                    .child(
                        v_flex().gap_1().child(label("Location")).child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&location)))
                                .child(Button::new("browse").small().label("Browse…").on_click(
                                    move |_, window, cx| choose_location(browse_location.clone(), window, cx),
                                )),
                        ),
                    )
                    .child(
                        Checkbox::new("git-init")
                            .label("Create Git repository")
                            .checked(*git.read(cx))
                            .on_click({
                                let git = git.clone();
                                move |checked: &bool, _, cx| git.update(cx, |g, _| *g = *checked)
                            }),
                    )
                    .children(
                        error
                            .read(cx)
                            .clone()
                            .map(|m| div().text_sm().text_color(theme.danger).child(m)),
                    ),
            )
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("create")
                            .primary()
                            .small()
                            .label("Create")
                            .on_click(move |_, window, cx| create(window, cx)),
                    ),
            )
    });
}

fn choose_location(location: Entity<InputState>, window: &mut Window, cx: &mut App) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Choose".into()),
    });
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.first()
            {
                let home = dirs::home_dir().unwrap_or_default();
                let text = tilde(path, &home);
                location
                    .update_in(cx, |input, window, cx| input.set_value(text, window, cx))
                    .ok();
            }
        })
        .detach();
}

/// Creates the folder (and repository) off the UI thread, then opens it.
fn create_project(dir: PathBuf, git: bool, window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            let target = dir.clone();
            let created = cx
                .background_spawn(async move {
                    std::fs::create_dir_all(&target)?;
                    if git {
                        vcs::Repo::init(&target)?;
                    }
                    anyhow::Ok(())
                })
                .await;
            cx.update(|window, cx| match created {
                Ok(()) => crate::projects::open_project(dir, cx),
                Err(err) => window.push_notification(
                    Notification::error(format!("{err:#}")).title("Cannot create the project"),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}
