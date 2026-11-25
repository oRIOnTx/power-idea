//! Acting on runs: re-run all / failed jobs, cancel, and starting a workflow by hand (dispatch,
//! then finding and opening the run it created). Deploy workflows ask before they start.

use ci::dispatch::DispatchInput;
use ci::time::now_millis;
use ci::{Run, State, Workflow};
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use serde_json::{Map, Value};
use std::time::Duration;

use super::{CiEvent, CiView, Scope, fetch};
use crate::config::ProjectSettings;
use crate::style;

/// A workflow and the inputs of its `workflow_dispatch` trigger (`None`: it has none).
#[derive(Debug, Clone)]
pub struct WorkflowEntry {
    pub workflow: Workflow,
    pub dispatch: Option<Vec<DispatchInput>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAction {
    Rerun,
    RerunFailed,
    Cancel,
}

impl RunAction {
    fn done(self) -> &'static str {
        match self {
            Self::Rerun => "Re-running all jobs",
            Self::RerunFailed => "Re-running failed jobs",
            Self::Cancel => "Cancelling",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Run(u64, RunAction),
    Dispatch,
}

/// How long to look for the run a dispatch created before giving up.
const FIND_ATTEMPTS: u32 = 15;
const FIND_EVERY: Duration = Duration::from_secs(2);
/// GitHub needs a moment before a re-run or cancel shows in the run's status.
const SETTLE: Duration = Duration::from_millis(1500);

impl CiView {
    pub(super) fn load_workflows(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        let root = self.root.clone();
        cx.spawn(async move |this, cx| {
            let (workflows, (patterns, context_limit)) = cx
                .background_spawn(async move {
                    let settings = ProjectSettings::load(&root).ci;
                    let config = (settings.deploy_patterns(), settings.claude_context_chars());
                    (fetch::workflows(&client, &root), config)
                })
                .await;
            this.update(cx, |view, cx| {
                view.deploy_patterns = patterns;
                view.context_limit = context_limit;
                match workflows {
                    Ok(workflows) => view.workflows = workflows,
                    Err(err) => {
                        if err.is_unexpected() {
                            error_journal::report_message(&err.to_string(), "GitHub Actions workflows");
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn workflow_entry(&self, workflow_id: u64) -> Option<&WorkflowEntry> {
        self.workflows.iter().find(|w| w.workflow.id == workflow_id)
    }

    /// The workflow deploys (by name or file name): starting or re-running it asks first.
    pub(crate) fn is_deploy(&self, workflow_id: u64, name: &str) -> bool {
        let path = self
            .workflow_entry(workflow_id)
            .map_or_default(|w| w.workflow.path.as_str());
        ci::dispatch::is_deploy(name, path, &self.deploy_patterns)
    }

    pub(crate) fn action_available(&self, run: &Run, action: RunAction) -> bool {
        let state = run.state();
        match action {
            RunAction::Cancel => state.is_active(),
            RunAction::Rerun => !state.is_active(),
            RunAction::RerunFailed => matches!(state, State::Failure | State::Cancelled),
        }
    }

    pub(crate) fn is_busy(&self, run_id: u64, action: RunAction) -> bool {
        self.busy == Some(Busy::Run(run_id, action))
    }

    /// Re-run / cancel the selected run; deploy workflows ask before a re-run.
    pub(crate) fn run_action(&mut self, action: RunAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.selected_run().cloned() else {
            return;
        };
        if !self.action_available(&run, action) || self.busy.is_some() {
            return;
        }
        let name = run.workflow_name().to_string();
        if action == RunAction::Cancel || !self.is_deploy(run.workflow_id, &name) {
            return self.start_run_action(run.id, action, cx);
        }
        let view = cx.entity().downgrade();
        let title = format!("Re-run {name} #{}?", run.run_number);
        let body = format!(
            "{name} looks like a deploy workflow. It runs again on {} ({}).",
            run.head_branch.as_deref().unwrap_or_default(),
            run.short_sha()
        );
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let view = view.clone();
            style::danger_confirm(dialog, "rocket", "Re-run", cx)
                .title(title.clone())
                .child(body.clone())
                .on_ok(move |_, _, cx| {
                    view.update(cx, |view, cx| view.start_run_action(run.id, action, cx))
                        .ok();
                    true
                })
        });
    }

    fn start_run_action(&mut self, run_id: u64, action: RunAction, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        let label = self
            .find_run(run_id)
            .map_or_default(|r| format!("{} #{}", r.workflow_name(), r.run_number));
        self.busy = Some(Busy::Run(run_id, action));
        cx.notify();
        self.actions_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    match action {
                        RunAction::Rerun => client.rerun(run_id),
                        RunAction::RerunFailed => client.rerun_failed(run_id),
                        RunAction::Cancel => client.cancel(run_id),
                    }
                })
                .await;
            if result.is_ok() {
                cx.background_executor().timer(SETTLE).await;
            }
            this.update(cx, |view, cx| {
                view.busy = None;
                match result {
                    Ok(()) => {
                        if action != RunAction::Cancel {
                            view.triggered.insert(run_id);
                        }
                        // The jobs start over: reload them, not just the run.
                        if let Some(detail) = view.detail.as_mut().filter(|d| d.run_id == run_id) {
                            detail.loaded = false;
                        }
                        cx.emit(CiEvent::Notify {
                            title: action.done().to_string(),
                            body: label,
                            failed: false,
                        });
                        view.refresh(cx);
                    }
                    Err(err) => cx.emit(CiEvent::Notify {
                        title: format!("{label}: {} failed", action.done().to_lowercase()),
                        body: err.to_string(),
                        failed: true,
                    }),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Opens the Run workflow form (for the selected run's workflow when it can be started).
    pub(crate) fn open_run_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.client.is_none() {
            return;
        }
        let preferred = self
            .selected_run()
            .map(|r| r.workflow_id)
            .filter(|id| self.workflow_entry(*id).is_some_and(|w| w.dispatch.is_some()));
        let git_ref = self.branch.clone().unwrap_or_default();
        super::run_dialog::open(
            cx.entity().downgrade(),
            &self.workflows,
            preferred,
            git_ref,
            window,
            cx,
        );
    }

    /// Starts `workflow_id` on `git_ref`; a deploy workflow asks first.
    pub(crate) fn dispatch(
        &mut self,
        workflow_id: u64,
        git_ref: String,
        inputs: Map<String, Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.workflow_entry(workflow_id) else {
            return;
        };
        let name = entry.workflow.name.clone();
        if !self.is_deploy(workflow_id, &name) {
            return self.start_dispatch(workflow_id, git_ref, inputs, cx);
        }
        let view = cx.entity().downgrade();
        let title = format!("Run {name} on {git_ref}?");
        let details = if inputs.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = inputs
                .iter()
                .map(|(k, v)| format!("{k} = {}", v.as_str().unwrap_or_default()))
                .collect();
            format!(" Inputs: {}.", pairs.join(", "))
        };
        let body = format!("{name} looks like a deploy workflow.{details}");
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let (view, git_ref, inputs) = (view.clone(), git_ref.clone(), inputs.clone());
            style::danger_confirm(dialog, "rocket", "Run", cx)
                .title(title.clone())
                .child(body.clone())
                .on_ok(move |_, _, cx| {
                    let (git_ref, inputs) = (git_ref.clone(), inputs.clone());
                    view.update(cx, |view, cx| {
                        view.start_dispatch(workflow_id, git_ref, inputs, cx);
                    })
                    .ok();
                    true
                })
        });
    }

    fn start_dispatch(
        &mut self,
        workflow_id: u64,
        git_ref: String,
        inputs: Map<String, Value>,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.client.clone() else { return };
        let name = self
            .workflow_entry(workflow_id)
            .map_or_default(|w| w.workflow.name.clone());
        self.busy = Some(Busy::Dispatch);
        cx.notify();
        let since = now_millis();
        self.actions_task = Some(cx.spawn(async move |this, cx| {
            let started = {
                let (client, git_ref) = (client.clone(), git_ref.clone());
                cx.background_spawn(async move { client.dispatch(workflow_id, &git_ref, inputs) })
                    .await
            };
            let mut run_id = match started {
                Ok(id) => id,
                Err(err) => {
                    this.update(cx, |view, cx| {
                        view.busy = None;
                        cx.emit(CiEvent::Notify {
                            title: format!("Could not start {name}"),
                            body: err.to_string(),
                            failed: true,
                        });
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let mut found = None;
            for _ in 0..FIND_ATTEMPTS {
                if run_id.is_some() {
                    break;
                }
                cx.background_executor().timer(FIND_EVERY).await;
                let (client, git_ref) = (client.clone(), git_ref.clone());
                let run = cx
                    .background_spawn(async move { client.find_dispatched_run(workflow_id, &git_ref, since) })
                    .await;
                if let Ok(Some(run)) = run {
                    run_id = Some(run.id);
                    found = Some(run);
                }
            }
            this.update(cx, |view, cx| {
                view.busy = None;
                view.dispatched(&name, &git_ref, run_id, found, cx);
            })
            .ok();
        }));
    }

    /// Opens the run a dispatch created and watches it for the "finished" notification.
    fn dispatched(
        &mut self,
        name: &str,
        git_ref: &str,
        run_id: Option<u64>,
        run: Option<Run>,
        cx: &mut Context<Self>,
    ) {
        let Some(run_id) = run_id else {
            cx.emit(CiEvent::Notify {
                title: format!("{name} started on {git_ref}"),
                body: "The run did not show up yet; it will appear in the list.".into(),
                failed: false,
            });
            self.refresh(cx);
            return;
        };
        self.triggered.insert(run_id);
        if let Some(run) = run
            && self.find_run(run_id).is_none()
        {
            self.tracked_runs.push(run);
        }
        if self.branch.as_deref() != Some(git_ref) {
            self.set_scope(Scope::All, cx);
        }
        cx.emit(CiEvent::Notify {
            title: format!("{name} started on {git_ref}"),
            body: String::new(),
            failed: false,
        });
        self.select_run(run_id, true, cx);
        self.refresh(cx);
        cx.notify();
    }
}
