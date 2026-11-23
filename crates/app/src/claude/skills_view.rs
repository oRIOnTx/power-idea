//! The skills card: a switch per skill in claude/skills, with its description.

use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{ClaudeState, state};

/// Rows of the skills card: a switch per skill and its description.
pub fn skills_list(cx: &App) -> AnyElement {
    let state: &ClaudeState = state(cx);
    let theme = cx.theme();
    let disabled = state.settings.skills.disabled.clone();
    v_flex()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .children(state.skills.iter().enumerate().map(|(ix, skill)| {
            let name = skill.name.clone();
            h_flex()
                .gap_2()
                .px_2()
                .py_1p5()
                .when(ix > 0, |el| el.border_t_1().border_color(theme.border))
                .child(
                    Switch::new(("claude-skill-on", ix))
                        .small()
                        .checked(!disabled.contains(&skill.name))
                        .on_click(move |checked, _, cx| {
                            let (name, checked) = (name.clone(), *checked);
                            super::update_settings(cx, |s| {
                                s.skills.disabled.retain(|d| *d != name);
                                if !checked {
                                    s.skills.disabled.push(name);
                                    s.skills.disabled.sort();
                                }
                            });
                        }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().text_sm().child(skill.name.clone()))
                                .when(skill.builtin, |el| {
                                    el.child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child("built-in"),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(skill.description.clone()),
                        ),
                )
        }))
        .when(state.skills.is_empty(), |el| {
            el.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("No skills. Add SKILL.md folders to claude/skills."),
            )
        })
        .into_any_element()
}
