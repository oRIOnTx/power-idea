//! Run configurations in the workspace: choosing, running (each in its own
//! terminal tab, kept after the process ends), stopping and editing
//! `.power-idea/run.toml`. Works for any language: a configuration is a command.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _};
use gpui_kit::*;
use run::RunConfig;

use super::Workspace;
use crate::config::AppConfig;
use crate::palette::{Palette, PaletteMode};
use crate::style;
use crate::terminal::{Session, TerminalView};

impl Workspace {
    /// Reloads user and detected configurations in the background.
    pub(super) fn load_run_configs(
        &mut self,
        then_choose: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let configs = cx.background_spawn(async move { run::all(&root) }).await;
            this.update_in(cx, |ws, window, cx| {
                match configs {
                    Ok(configs) => ws.run_configs = configs,
                    Err(err) => window.push_notification(
                        Notification::error(format!("{err:#}")).title("run.toml has an error"),
                        cx,
                    ),
                }
                if ws
                    .selected_run
                    .as_ref()
                    .is_some_and(|name| !ws.run_configs.iter().any(|c| &c.name == name))
                {
                    ws.selected_run = None;
                }
                if ws.selected_run.is_none() {
                    ws.selected_run = ws.run_configs.first().map(|c| c.name.clone());
                }
                if then_choose {
                    ws.open_run_chooser_now(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// "Run…" (⌃⌥R): picks a configuration to run.
    pub(super) fn choose_run_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_run_configs(true, window, cx);
    }

    fn open_run_chooser_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut source = self.palette_source(cx);
        source.run_configs = self.run_configs.clone();
        Palette::open(PaletteMode::RunConfigs, source, window, cx);
    }

    /// Run (⌃R): the selected configuration, or the chooser when there is none.
    pub(super) fn run_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self
            .selected_run
            .as_ref()
            .and_then(|name| self.run_configs.iter().find(|c| &c.name == name))
            .cloned();
        match config {
            Some(config) => self.run_config(config, window, cx),
            None => self.choose_run_config(window, cx),
        }
    }

    /// Runs `config` in a terminal tab, replacing an earlier tab of the same configuration.
    pub fn run_config(&mut self, config: RunConfig, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.resolve_run_config(&config, window, cx) else {
            return;
        };
        self.selected_run = Some(config.name.clone());
        if let Some(ix) = self
            .terminals
            .iter()
            .position(|t| t.read(cx).run_state().is_some_and(|r| r.name == config.name))
        {
            self.terminals[ix].update(cx, |t, cx| t.stop(cx));
            self.close_terminal(ix, window, cx);
        }
        let cwd = config.working_dir(&self.root);
        let shell =
            Some(cx.global::<AppConfig>().0.terminal_shell.trim().to_string()).filter(|s| !s.is_empty());
        let session = match Session::spawn_command(cwd.clone(), shell, &config.command, &config.env) {
            Ok(session) => session,
            Err(err) => return self.notify_error(format!("Cannot run {}: {err:#}", config.name), window, cx),
        };
        let root = self.root.clone();
        let terminal = cx.new(|cx| {
            TerminalView::new(session, cx)
                .for_run(config.name)
                .with_links(root, cwd)
        });
        self.add_terminal(terminal, window, cx);
    }

    /// `config` with its macros (`$FilePath$`…) filled in from the active editor,
    /// whose file is saved first when the configuration runs it; None after notifying.
    pub(super) fn resolve_run_config(
        &mut self,
        config: &RunConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<RunConfig> {
        let file = self.active().map(|tab| tab.path.clone());
        if config.uses_current_file()
            && let Some(ix) = self.active_tab
            && self.tabs[ix].is_dirty(cx)
            && !self.save_tab(ix, window, cx)
        {
            return None;
        }
        match config.expand(&self.root, file.as_deref()) {
            Ok(config) => Some(config),
            Err(err) => {
                self.notify_error(format!("{err:#}"), window, cx);
                None
            }
        }
    }

    /// Stop (⌘F2): the active run tab, or the most recent running one.
    pub(super) fn stop_run(&mut self, cx: &mut Context<Self>) {
        let active = self
            .terminals
            .get(self.active_terminal)
            .filter(|t| t.read(cx).is_running());
        let target = active
            .or_else(|| self.terminals.iter().rev().find(|t| t.read(cx).is_running()))
            .cloned();
        if let Some(terminal) = target {
            terminal.update(cx, |t, cx| t.stop(cx));
            cx.notify();
        }
    }

    fn any_running(&self, cx: &App) -> bool {
        self.terminals.iter().any(|t| t.read(cx).is_running())
    }

    /// Opens `.power-idea/run.toml`, creating it from the detected configurations.
    pub fn edit_run_configs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = run::config_file(&self.root);
        if !path.exists() {
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, run::template(&self.root)));
            if let Err(err) = written {
                return self.notify_error(format!("Cannot create run.toml: {err}"), window, cx);
            }
        }
        self.open_file(".power-idea/run.toml", None, window, cx);
    }

    /// Title-bar widget: configuration picker, Run and Stop.
    pub(super) fn render_run_widget(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.any_running(cx);
        let label = self.selected_run.clone().unwrap_or_else(|| "Run…".into());
        let theme = cx.theme();
        let mut row = gpui_kit::component::h_flex().gap_0p5().child(
            Button::new("run-config-widget")
                .ghost()
                .xsmall()
                .label(label)
                .dropdown_caret(true)
                .tooltip("Run Configurations (⌃⌥R)")
                .on_click(cx.listener(|ws, _, window, cx| ws.choose_run_config(window, cx))),
        );
        row = row.child(
            style::icon_button_sized(
                "run-start",
                style::lucide("play").text_color(theme.green),
                "Run (⌃R)",
                style::BUTTON_TITLE_BAR,
            )
            .on_click(cx.listener(|ws, _, window, cx| ws.run_selected(window, cx))),
        );
        row = row.child(
            style::icon_button_sized(
                "debug-start",
                style::lucide("bug").text_color(theme.green),
                "Debug (⌃D)",
                style::BUTTON_TITLE_BAR,
            )
            .on_click(cx.listener(|ws, _, window, cx| ws.debug_selected(window, cx))),
        );
        if running || self.debug_active() {
            row = row.child(
                style::icon_button_sized(
                    "run-stop",
                    style::lucide("square").text_color(theme.danger),
                    "Stop (⌘F2)",
                    style::BUTTON_TITLE_BAR,
                )
                .on_click(cx.listener(|ws, _, _, cx| {
                    if ws.debug_active() {
                        ws.stop_debug(cx);
                    } else {
                        ws.stop_run(cx);
                    }
                })),
            );
        }
        row
    }
}
