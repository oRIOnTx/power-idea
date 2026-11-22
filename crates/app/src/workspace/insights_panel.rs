//! Claude Session Insights tool window: state, background indexing of
//! `~/.claude/projects` into the local store, and the user's choices
//! (period, project filter, selected session, dismissed tips).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use insights::{Insight, Period, SessionSummary, Store};

use super::insights_advice::Advice;
use super::{LeftPanel, Workspace};
use crate::config::{AppConfig, Config};

/// New sessions show up without a manual refresh while the panel exists.
const REINDEX_EVERY: Duration = Duration::from_secs(120);
/// Tips shown at once; the spec asks for no more than three.
const MAX_TIPS: usize = 3;

pub(super) enum InsightsStatus {
    Indexing,
    Failed(String),
    Ready,
}

pub(super) struct InsightsState {
    pub(super) status: InsightsStatus,
    store: Option<Arc<Store>>,
    pub(super) sessions: Vec<SessionSummary>,
    pub(super) dismissed: Vec<String>,
    pub(super) period: Period,
    pub(super) project: Option<String>,
    pub(super) selected: Option<String>,
    pub(super) advice: Advice,
    /// Re-index loop; dropping the state stops it.
    _refresh: Option<Task<()>>,
}

impl InsightsState {
    /// Sessions of the chosen project (all when no filter).
    pub(super) fn scoped_sessions(&self) -> Vec<SessionSummary> {
        self.sessions
            .iter()
            .filter(|s| self.project.as_ref().is_none_or(|p| &s.project == p))
            .cloned()
            .collect()
    }

    pub(super) fn tips(&self) -> Vec<Insight> {
        insights::rules::insights(&self.scoped_sessions(), &self.dismissed, MAX_TIPS)
    }
}

fn store_path() -> Option<PathBuf> {
    Config::dir().map(|dir| dir.join("insights.sqlite"))
}

fn open_store() -> Result<Store> {
    Store::open(&store_path().context("no config directory on this platform")?)
}

/// Indexes new transcripts and returns everything the panel shows.
fn load(store: &Store) -> Result<(Vec<SessionSummary>, Vec<String>)> {
    let claude = dirs::home_dir().context("no home directory")?.join(".claude");
    insights::index(store, &claude, &insights::DEFAULT_WEIGHTS)?;
    Ok((store.sessions()?, store.dismissed()?))
}

/// Settings → Insights → "Clear all analytics data" (asks first).
pub fn clear_insights(window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, |dialog, _, cx| {
        crate::style::danger_confirm(dialog, "eraser", "Clear", cx)
            .title("Clear all analytics data?")
            .child("Indexed sessions, scores and dismissed tips are deleted. Claude transcripts are not touched.")
            .on_ok(|_, window, cx| {
                let task = cx.background_spawn(async move { open_store()?.clear() });
                window
                    .spawn(cx, async move |cx| {
                        let result = task.await;
                        cx.update(|window, cx| match result {
                            Ok(()) => {
                                window.push_notification(Notification::success("Analytics data cleared"), cx);
                            }
                            Err(err) => window.push_notification(
                                Notification::error(format!("{err:#}")).title("Insights"),
                                cx,
                            ),
                        })
                        .ok();
                    })
                    .detach();
                true
            })
    });
}

impl Workspace {
    pub(super) fn insights_enabled(cx: &App) -> bool {
        cx.global::<AppConfig>().0.insights_enabled
    }

    /// Shows the Insights panel (indexing on first use) or hides the sidebar.
    pub(super) fn toggle_insights(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !Self::insights_enabled(cx) {
            window.push_notification(
                Notification::info("Turn on Claude Session Insights in Settings → Insights."),
                cx,
            );
            return;
        }
        self.toggle_left_panel(LeftPanel::Insights, cx);
        if self.show_project_view && self.left_panel == LeftPanel::Insights {
            self.ensure_insights(window, cx);
        }
    }

    pub(super) fn ensure_insights(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.insights.is_some() {
            return;
        }
        let refresh = cx.spawn_in(window, async move |this, cx| {
            loop {
                if this.update(cx, |ws, cx| ws.reindex_insights(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(REINDEX_EVERY).await;
            }
        });
        self.insights = Some(InsightsState {
            status: InsightsStatus::Indexing,
            store: None,
            sessions: Vec::new(),
            dismissed: Vec::new(),
            period: Period::default(),
            project: None,
            selected: None,
            advice: Advice::None,
            _refresh: Some(refresh),
        });
        cx.notify();
    }

    /// Reads new or changed transcripts off the UI thread, then reloads the panel.
    pub(super) fn reindex_insights(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.insights.as_ref() else {
            return;
        };
        let store = state.store.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let store = match store {
                        Some(store) => store,
                        None => Arc::new(open_store()?),
                    };
                    let (sessions, dismissed) = load(&store)?;
                    anyhow::Ok((store, sessions, dismissed))
                })
                .await;
            this.update(cx, |ws, cx| {
                let Some(state) = ws.insights.as_mut() else { return };
                match loaded {
                    Ok((store, sessions, dismissed)) => {
                        state.store = Some(store);
                        state.sessions = sessions;
                        state.dismissed = dismissed;
                        state.status = InsightsStatus::Ready;
                    }
                    Err(err) => state.status = InsightsStatus::Failed(format!("{err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn dismiss_tip(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let Some(state) = self.insights.as_mut() else {
            return;
        };
        state.dismissed.push(id.to_string());
        cx.notify();
        if let Some(store) = state.store.clone() {
            cx.background_spawn(async move {
                if let Err(err) = store.dismiss(id) {
                    eprintln!("insights: could not save a dismissed tip: {err:#}");
                }
            })
            .detach();
        }
    }
}
