//! The "Rules & patterns" card: search and tag filter, a list with switches, scope chips and
//! drag-to-reorder priority, templates, folder import/export, preview, export to project and
//! global install / uninstall.

use std::fs;
use std::path::{Path, PathBuf};

use claude_integration::rules::{self, AppliesTo};
use claude_integration::{ProjectFacts, Rule};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::setting::{SettingGroup, SettingItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::page::{mode_item, switch_item};
use super::{reload, state};
use crate::workspace::Workspace;

/// Search text and tag filter of the list, kept across renders.
struct Filter {
    input: Entity<InputState>,
    tag: Option<String>,
    _subscription: Subscription,
}

/// What is being dragged to reorder.
#[derive(Clone)]
struct DraggedRule {
    path: PathBuf,
    name: SharedString,
}

impl Render for DraggedRule {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.primary)
            .text_sm()
            .child(self.name.clone())
    }
}

pub fn group(workspace: WeakEntity<Workspace>) -> SettingGroup {
    let list_workspace = workspace.clone();
    SettingGroup::new()
        .title("Rules & patterns")
        .description(
            "Markdown rules in claude/rules, mixed into Claude's context for the projects they apply to, \
             highest priority first. The project's own CLAUDE.md and .claude settings take precedence.",
        )
        .item(switch_item(
            "Enabled",
            |s| s.rules.enabled,
            |s, v| s.rules.enabled = v,
        ))
        .item(mode_item(|s| s.rules.mode, |s, m| s.rules.mode = m))
        .item(SettingItem::render(move |_, window, cx| {
            list(&list_workspace, window, cx)
        }))
        .item(SettingItem::render(move |_, _, cx| {
            super::rule_actions::actions(&workspace, cx)
        }))
}

fn list(workspace: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    let filter = window.use_keyed_state("claude-rule-filter", cx, |window, cx| {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search rules"));
        let subscription = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.refresh_windows();
            }
        });
        Filter {
            input,
            tag: None,
            _subscription: subscription,
        }
    });
    let (query, tag) = {
        let filter = filter.read(cx);
        (filter.input.read(cx).value().to_lowercase(), filter.tag.clone())
    };
    let state = state(cx);
    let mut tags: Vec<String> = state.rules.iter().flat_map(|r| r.meta.tags.clone()).collect();
    tags.sort();
    tags.dedup();
    let shown: Vec<Rule> = state
        .rules
        .iter()
        .filter(|r| tag.as_ref().is_none_or(|t| r.meta.tags.contains(t)))
        .filter(|r| {
            query.is_empty()
                || r.meta.name.to_lowercase().contains(&query)
                || r.meta.description.to_lowercase().contains(&query)
                || r.body.to_lowercase().contains(&query)
        })
        .cloned()
        .collect();
    let all = state.rules.clone();
    let theme = cx.theme();
    let input = filter.read(cx).input.clone();

    let chips = tags.into_iter().map(|t| {
        let active = tag.as_deref() == Some(t.as_str());
        let filter = filter.clone();
        let value = t.clone();
        Button::new(SharedString::from(format!("tag-{t}")))
            .xsmall()
            .label(format!("#{t}"))
            .when(active, ButtonVariants::primary)
            .when(!active, ButtonVariants::ghost)
            .on_click(move |_, _, cx| {
                let value = value.clone();
                filter.update(cx, |f, cx| {
                    f.tag = if f.tag.as_ref() == Some(&value) {
                        None
                    } else {
                        Some(value)
                    };
                    cx.refresh_windows();
                });
            })
    });

    let rows = shown
        .iter()
        .enumerate()
        .map(|(ix, rule)| row(ix, rule, &all, workspace, cx));
    v_flex()
        .gap_2()
        .child(h_flex().gap_2().child(div().w(px(240.)).child(Input::new(&input).small())).children(chips))
        .child(
            v_flex()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .children(rows)
                .when(shown.is_empty(), |el| {
                    el.child(div().p_3().text_sm().text_color(theme.muted_foreground).child(
                        if all.is_empty() {
                            "No rules yet. Create one from a template below, or add .md files to claude/rules."
                        } else {
                            "No rule matches the filter."
                        },
                    ))
                }),
        )
        .into_any_element()
}

fn row(ix: usize, rule: &Rule, all: &[Rule], workspace: &WeakEntity<Workspace>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let path = rule.path.clone();
    let name: SharedString = rule.meta.name.clone().into();
    let dragged = DraggedRule {
        path: path.clone(),
        name: name.clone(),
    };
    let order: Vec<PathBuf> = all.iter().map(|r| r.path.clone()).collect();
    let drop_order = order.clone();
    let drop_target = path.clone();
    let (toggle_path, up_order, down_order) = (path.clone(), order.clone(), order);
    let (test_rule, edit_path, edit_ws, test_ws) =
        (rule.clone(), path.clone(), workspace.clone(), workspace.clone());

    h_flex()
        .id(("claude-rule", ix))
        .gap_2()
        .px_2()
        .py_1p5()
        .when(ix > 0, |el| el.border_t_1().border_color(theme.border))
        .drag_over::<DraggedRule>(|style, _, _, cx| style.bg(cx.theme().list_hover))
        .on_drop(move |dragged: &DraggedRule, window, cx| {
            move_rule(&drop_order, &dragged.path, &drop_target, window, cx);
        })
        .child(
            div()
                .id(("claude-rule-handle", ix))
                .cursor_grab()
                .text_color(theme.muted_foreground)
                .child("⠿")
                .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone())),
        )
        .child(
            Switch::new(("claude-rule-on", ix))
                .small()
                .checked(rule.meta.enabled)
                .on_click(move |checked, window, cx| set_enabled(&toggle_path, *checked, window, cx)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().text_sm().child(name))
                        .children(scope_chips(&rule.meta.applies_to, cx))
                        .children(rule.meta.tags.iter().map(|t| {
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!("#{t}"))
                        })),
                )
                .when(!rule.meta.description.is_empty(), |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(rule.meta.description.clone()),
                    )
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("priority {}", rule.meta.priority)),
        )
        .child(small_button(("rule-up", ix), "↑").on_click(move |_, window, cx| {
            shift(&up_order, ix, -1, window, cx);
        }))
        .child(
            small_button(("rule-down", ix), "↓").on_click(move |_, window, cx| {
                shift(&down_order, ix, 1, window, cx);
            }),
        )
        .child(
            small_button(("rule-test", ix), "Test").on_click(move |_, window, cx| {
                test_match(&test_rule, &test_ws, window, cx);
            }),
        )
        .child(
            small_button(("rule-edit", ix), "Edit").on_click(move |_, window, cx| {
                open_file(&edit_ws, &edit_path, window, cx);
            }),
        )
        .into_any_element()
}

fn small_button(id: impl Into<ElementId>, label: &'static str) -> Button {
    Button::new(id).ghost().xsmall().label(label)
}

fn scope_chips(applies_to: &AppliesTo, cx: &App) -> Vec<Div> {
    let theme = cx.theme();
    let chip = |text: String| {
        div()
            .px_1p5()
            .rounded(px(4.))
            .text_xs()
            .bg(theme.primary.opacity(0.12))
            .text_color(theme.primary)
            .child(text)
    };
    match applies_to {
        AppliesTo::All => vec![chip("All".into())],
        AppliesTo::Scoped(scope) => scope
            .languages
            .iter()
            .map(|l| chip(capitalize(l)))
            .chain(scope.paths.iter().map(|p| chip(p.clone())))
            .chain(scope.projects.iter().map(|p| chip(format!("project {p}"))))
            .collect(),
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// Rewrites one rule file, reporting a failure.
fn save(rule: &Rule, window: &mut Window, cx: &mut App) {
    if let Err(err) = fs::write(&rule.path, rule.render()) {
        let message = format!("Cannot save {}: {err}", rule.path.display());
        window.push_notification(Notification::error(message), cx);
    }
    reload(cx);
}

fn find(path: &Path, cx: &App) -> Option<Rule> {
    state(cx).rules.iter().find(|r| r.path == path).cloned()
}

fn set_enabled(path: &Path, enabled: bool, window: &mut Window, cx: &mut App) {
    let Some(mut rule) = find(path, cx) else { return };
    rule.meta.enabled = enabled;
    save(&rule, window, cx);
}

/// Applies a new order by renumbering priorities (10 per step, top highest).
fn apply_order(order: &[PathBuf], window: &mut Window, cx: &mut App) {
    let rules: Vec<Rule> = order.iter().filter_map(|p| find(p, cx)).collect();
    for (path, priority) in rules::reprioritize(&rules) {
        if let Some(mut rule) = find(&path, cx) {
            rule.meta.priority = priority;
            save(&rule, window, cx);
        }
    }
}

fn shift(order: &[PathBuf], ix: usize, delta: isize, window: &mut Window, cx: &mut App) {
    let target = ix.checked_add_signed(delta).filter(|t| *t < order.len());
    let Some(target) = target else { return };
    let mut order = order.to_vec();
    order.swap(ix, target);
    apply_order(&order, window, cx);
}

fn move_rule(order: &[PathBuf], dragged: &Path, onto: &Path, window: &mut Window, cx: &mut App) {
    if dragged == onto {
        return;
    }
    let mut order: Vec<PathBuf> = order.iter().filter(|p| p.as_path() != dragged).cloned().collect();
    let at = order.iter().position(|p| p == onto).unwrap_or(order.len());
    order.insert(at, dragged.to_path_buf());
    apply_order(&order, window, cx);
}

pub(super) fn facts(workspace: &WeakEntity<Workspace>, cx: &App) -> Option<ProjectFacts> {
    workspace.upgrade().map(|ws| ws.read(cx).claude_facts())
}

fn test_match(rule: &Rule, workspace: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
    let Some(facts) = facts(workspace, cx) else { return };
    let languages: Vec<&str> = facts.languages.iter().map(String::as_str).collect();
    let (matches, enabled) = (rule.matches(&facts), rule.meta.enabled);
    let message = format!(
        "{} {} {} (languages found: {}){}",
        rule.meta.name,
        if matches {
            "applies to"
        } else {
            "does not apply to"
        },
        facts.name(),
        if languages.is_empty() {
            "none".into()
        } else {
            languages.join(", ")
        },
        if matches && !enabled {
            " — but it is switched off"
        } else {
            ""
        },
    );
    let note = if matches {
        Notification::success(message)
    } else {
        Notification::info(message)
    };
    window.push_notification(note.title("Test match"), cx);
}

pub(super) fn open_file(workspace: &WeakEntity<Workspace>, path: &Path, window: &mut Window, cx: &mut App) {
    let path = path.to_path_buf();
    workspace
        .update(cx, |ws, cx| ws.open_path(&path, None, window, cx))
        .ok();
}
