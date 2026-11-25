//! "Run workflow" dialog: pick a workflow that has `workflow_dispatch` (others are shown
//! disabled with the reason), a ref (the current branch by default) and its inputs.

use ci::dispatch::{DispatchInput, InputKind};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{CiView, WorkflowEntry};
use crate::style;

enum Field {
    Text(Entity<InputState>),
    Bool(bool),
    Choice(String),
}

pub struct RunWorkflowForm {
    view: WeakEntity<CiView>,
    workflows: Vec<WorkflowEntry>,
    selected: Option<u64>,
    git_ref: Entity<InputState>,
    fields: Vec<(DispatchInput, Field)>,
    error: Option<String>,
    focused_once: bool,
}

pub(super) fn open(
    view: WeakEntity<CiView>,
    workflows: &[WorkflowEntry],
    preferred: Option<u64>,
    git_ref: String,
    window: &mut Window,
    cx: &mut App,
) {
    let workflows = workflows.to_vec();
    let form = cx.new(|cx| RunWorkflowForm::new(view, workflows, preferred, git_ref, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.w(px(560.)).title("Run Workflow").child(form.clone())
    });
}

impl RunWorkflowForm {
    fn new(
        view: WeakEntity<CiView>,
        workflows: Vec<WorkflowEntry>,
        preferred: Option<u64>,
        git_ref: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let git_ref = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("branch, tag or SHA")
                .default_value(git_ref)
        });
        Self::submit_on_enter(&git_ref, window, cx);
        let selected = preferred.or_else(|| {
            workflows
                .iter()
                .find(|w| w.dispatch.is_some())
                .map(|w| w.workflow.id)
        });
        let mut form = Self {
            view,
            workflows,
            selected: None,
            git_ref,
            fields: Vec::new(),
            error: None,
            focused_once: false,
        };
        if let Some(id) = selected {
            form.select(id, window, cx);
        }
        form
    }

    fn submit_on_enter(input: &Entity<InputState>, window: &mut Window, cx: &mut Context<Self>) {
        cx.subscribe_in(input, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { secondary: false, .. } = event {
                this.run(window, cx);
            }
        })
        .detach();
    }

    fn select(&mut self, workflow_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inputs) = self
            .workflows
            .iter()
            .find(|w| w.workflow.id == workflow_id)
            .and_then(|w| w.dispatch.clone())
        else {
            return;
        };
        self.selected = Some(workflow_id);
        self.error = None;
        self.fields = inputs
            .into_iter()
            .map(|input| {
                let default = input.default.clone().unwrap_or_default();
                let field = match &input.kind {
                    InputKind::Boolean => Field::Bool(default == "true"),
                    InputKind::Choice(options) if !options.is_empty() => {
                        Field::Choice(if options.contains(&default) {
                            default
                        } else {
                            options.first().cloned().unwrap_or_default()
                        })
                    }
                    _ => {
                        let placeholder = if input.description.is_empty() {
                            input.name.clone()
                        } else {
                            input.description.clone()
                        };
                        let state = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(placeholder)
                                .default_value(default)
                        });
                        Self::submit_on_enter(&state, window, cx);
                        Field::Text(state)
                    }
                };
                (input, field)
            })
            .collect();
        cx.notify();
    }

    fn values(&self, cx: &App) -> Vec<(String, String)> {
        self.fields
            .iter()
            .map(|(input, field)| {
                let value = match field {
                    Field::Text(state) => state.read(cx).value().to_string(),
                    Field::Bool(on) => on.to_string(),
                    Field::Choice(choice) => choice.clone(),
                };
                (input.name.clone(), value)
            })
            .collect()
    }

    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workflow_id) = self.selected else {
            self.error = Some("Choose a workflow".into());
            return cx.notify();
        };
        let git_ref = self.git_ref.read(cx).value().trim().to_string();
        if git_ref.is_empty() {
            self.error = Some("Enter a branch, tag or commit".into());
            return cx.notify();
        }
        let inputs: Vec<DispatchInput> = self.fields.iter().map(|(i, _)| i.clone()).collect();
        let body = match ci::dispatch::dispatch_body(&inputs, &self.values(cx)) {
            Ok(body) => body,
            Err(message) => {
                self.error = Some(message);
                return cx.notify();
            }
        };
        window.close_dialog(cx);
        let view = self.view.clone();
        // The confirmation for deploys opens as a new dialog once this one is gone.
        window.defer(cx, move |window, cx| {
            view.update(cx, |view, cx| {
                view.dispatch(workflow_id, git_ref, body, window, cx);
            })
            .ok();
        });
    }

    fn render_workflows(&self, cx: &mut Context<Self>) -> Div {
        v_flex()
            .gap_1()
            .children(self.workflows.iter().enumerate().map(|(ix, entry)| {
                let id = entry.workflow.id;
                let enabled = entry.dispatch.is_some();
                let button = Button::new(("ci-run-workflow", ix))
                    .small()
                    .ghost()
                    .w_full()
                    .justify_start()
                    .icon(style::lucide("workflow"))
                    .label(entry.workflow.name.clone())
                    .selected(self.selected == Some(id))
                    .disabled(!enabled)
                    .on_click(cx.listener(move |form, _, window, cx| form.select(id, window, cx)));
                if enabled {
                    button.into_any_element()
                } else {
                    let why: SharedString = format!(
                        "{} has no workflow_dispatch trigger, so it cannot be started by hand",
                        entry.workflow.path
                    )
                    .into();
                    div()
                        .id(("ci-run-workflow-off", ix))
                        .tooltip(move |window, cx| Tooltip::new(why.clone()).build(window, cx))
                        .child(button)
                        .into_any_element()
                }
            }))
    }

    fn render_field(&self, ix: usize, input: &DispatchInput, field: &Field, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let label = h_flex()
            .gap_1()
            .text_xs()
            .child(div().font_weight(FontWeight::MEDIUM).child(input.name.clone()))
            .when(input.required, |el| {
                el.child(div().text_color(theme.danger).child("*"))
            })
            .when(!input.description.is_empty(), |el| {
                el.child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format!("— {}", input.description)),
                )
            });
        let control = match (field, &input.kind) {
            (Field::Text(state), _) => Input::new(state).small().into_any_element(),
            (Field::Bool(on), _) => Checkbox::new(("ci-input-bool", ix))
                .checked(*on)
                .label(if *on { "true" } else { "false" })
                .on_click(cx.listener(move |form, checked: &bool, _, cx| {
                    if let Some((_, Field::Bool(value))) = form.fields.get_mut(ix) {
                        *value = *checked;
                    }
                    cx.notify();
                }))
                .into_any_element(),
            (Field::Choice(current), InputKind::Choice(options)) => h_flex()
                .flex_wrap()
                .gap_1()
                .children(options.iter().enumerate().map(|(o, option)| {
                    let value = option.clone();
                    Button::new(SharedString::from(format!("ci-choice-{ix}-{o}")))
                        .xsmall()
                        .label(option.clone())
                        .selected(option == current)
                        .on_click(cx.listener(move |form, _, _, cx| {
                            if let Some((_, Field::Choice(slot))) = form.fields.get_mut(ix) {
                                slot.clone_from(&value);
                            }
                            cx.notify();
                        }))
                }))
                .into_any_element(),
            (Field::Choice(_), _) => div().into_any_element(),
        };
        v_flex().gap_1().child(label).child(control)
    }
}

impl Render for RunWorkflowForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            self.git_ref.update(cx, |input, cx| input.focus(window, cx));
        }
        let theme = cx.theme().clone();
        let fields: Vec<Div> = self
            .fields
            .iter()
            .enumerate()
            .map(|(ix, (input, field))| self.render_field(ix, input, field, cx))
            .collect();
        let deploy = self.selected.and_then(|id| {
            let entry = self.workflows.iter().find(|w| w.workflow.id == id)?;
            let view = self.view.upgrade()?;
            view.read(cx).is_deploy(id, &entry.workflow.name).then_some(())
        });
        let none_dispatchable = self.workflows.iter().all(|w| w.dispatch.is_none());
        v_flex()
            .gap_3()
            .when(self.workflows.is_empty(), |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("Loading workflows…"),
                )
            })
            .when(none_dispatchable && !self.workflows.is_empty(), |el| {
                el.child(div().text_xs().text_color(theme.muted_foreground).child(
                    "No workflow here has an `on: workflow_dispatch` trigger; add one to start it from the IDE.",
                ))
            })
            .child(
                div()
                    .id("ci-run-workflows")
                    .max_h(px(220.))
                    .overflow_y_scrollbar()
                    .child(self.render_workflows(cx)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_xs().font_weight(FontWeight::MEDIUM).child("Use workflow from"))
                    .child(Input::new(&self.git_ref).small()),
            )
            .children(fields)
            .children(self.error.clone().map(|e| div().text_xs().text_color(theme.danger).child(e)))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .when(deploy.is_some(), |el| {
                        el.child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(theme.warning)
                                .child("Deploy workflow: you will be asked to confirm."),
                        )
                    })
                    .child(
                        style::dialog_button("ci-run-cancel")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        style::dialog_button("ci-run-start")
                            .primary()
                            .icon(style::lucide("play"))
                            .label("Run workflow")
                            .disabled(self.selected.is_none())
                            .on_click(cx.listener(|form, _, window, cx| form.run(window, cx))),
                    ),
            )
    }
}
