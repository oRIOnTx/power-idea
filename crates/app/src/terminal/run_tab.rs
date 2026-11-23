//! Terminal tabs that run a configuration: they keep their output after the
//! process ends (with its exit code), can be stopped, and turn `file:line`
//! references into links (⌘+click), in plain terminals too.

use std::path::PathBuf;

use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::vte::ansi::Processor;
use gpui_kit::*;

use super::{TerminalEvent, TerminalView};

/// How a run tab's process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunExit {
    /// Exit code (`None` when killed by a signal).
    Code(Option<i32>),
    Stopped,
}

pub struct RunState {
    pub name: String,
    pub exit: Option<RunExit>,
}

impl TerminalView {
    /// Marks this terminal as running the configuration `name`.
    pub fn for_run(mut self, name: String) -> Self {
        self.title.clone_from(&name);
        self.run = Some(RunState { name, exit: None });
        self
    }

    /// Marks this run tab as a Claude Code tab.
    pub fn claude_tab(mut self) -> Self {
        self.claude = true;
        self
    }

    pub fn is_claude(&self) -> bool {
        self.claude
    }

    /// Folders used to resolve `file:line` links: project root and working folder.
    pub fn with_links(mut self, root: PathBuf, cwd: PathBuf) -> Self {
        self.link_dirs = Some((root, cwd));
        self
    }

    pub fn run_state(&self) -> Option<&RunState> {
        self.run.as_ref()
    }

    pub fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.exit.is_none())
    }

    /// A running process started from a run configuration; the Claude tab is not one,
    /// so the Run widget's Stop and ⌘F2 leave it alone.
    pub fn is_config_running(&self) -> bool {
        !self.claude && self.is_running()
    }

    /// Prints a dimmed status line into the terminal, after the program's output.
    fn print_status(&self, text: &str, color: u8) {
        let bytes = format!("\r\n\x1b[{color}m{text}\x1b[0m\r\n");
        let mut term = self.term.lock();
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new()
            .advance(&mut *term, bytes.as_bytes());
    }

    /// Called when the run's process exits.
    pub(super) fn finish(&mut self, exit: RunExit, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut().filter(|r| r.exit.is_none()) else {
            return;
        };
        run.exit = Some(exit);
        let (text, color) = match exit {
            RunExit::Code(Some(0)) => ("Process finished with exit code 0".to_string(), 32),
            RunExit::Code(Some(code)) => (format!("Process finished with exit code {code}"), 31),
            RunExit::Code(None) => ("Process terminated by a signal".to_string(), 31),
            RunExit::Stopped => ("Process stopped".to_string(), 33),
        };
        self.print_status(&text, color);
        cx.emit(TerminalEvent::Finished);
        cx.notify();
    }

    /// Stops the running process (the PTY closes, which ends the process group).
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.is_running() {
            return;
        }
        let _ = self.notifier.0.send(Msg::Shutdown);
        self.listener.close();
        self.finish(RunExit::Stopped, cx);
    }

    /// Text of a visible row (one char per cell).
    fn row_text(&self, line: usize) -> String {
        let term = self.term.lock();
        let offset = term.grid().display_offset() as i32;
        let row = Line(line as i32 - offset);
        (0..term.columns())
            .map(|col| term.grid()[row][Column(col)].c)
            .collect()
    }

    /// ⌘+click: opens the `file:line` under the pointer, if any.
    pub(super) fn open_link_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some((root, cwd)) = self.link_dirs.clone() else {
            return false;
        };
        let (column, line) = self.cell_at(position);
        let text = self.row_text(line);
        let byte = text.char_indices().nth(column).map_or(text.len(), |(ix, _)| ix);
        let Some(location) = run::find_locations(&text, &root, &cwd)
            .into_iter()
            .find(|l| l.range.contains(&byte))
        else {
            return false;
        };
        cx.emit(TerminalEvent::OpenLocation {
            path: location.path,
            line: location.line,
            column: location.column.unwrap_or(1),
        });
        true
    }
}
