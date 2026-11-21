//! Shell process: PTY + `alacritty_terminal` event loop, and the bridge that
//! wakes the UI when the PTY thread produces events.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Notifier};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::tty;

const SCROLLBACK: usize = 10_000;

/// Events from the PTY thread, plus a waker for the UI task awaiting them.
#[derive(Default)]
pub(super) struct Mailbox {
    events: Vec<TermEvent>,
    waker: Option<Waker>,
    closed: bool,
}

#[derive(Clone, Default)]
pub(super) struct Listener(Arc<Mutex<Mailbox>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let mut mailbox = self.0.lock().unwrap();
        mailbox.events.push(event);
        if let Some(waker) = mailbox.waker.take() {
            waker.wake();
        }
    }
}

impl Listener {
    /// Resolves with all pending events, or `None` once the listener is closed.
    pub(super) fn next_batch(&self) -> impl Future<Output = Option<Vec<TermEvent>>> + '_ {
        std::future::poll_fn(move |cx| {
            let mut mailbox = self.0.lock().unwrap();
            if !mailbox.events.is_empty() {
                return Poll::Ready(Some(std::mem::take(&mut mailbox.events)));
            }
            if mailbox.closed {
                return Poll::Ready(None);
            }
            mailbox.waker = Some(cx.waker().clone());
            Poll::Pending
        })
    }

    pub(super) fn close(&self) {
        let mut mailbox = self.0.lock().unwrap();
        mailbox.closed = true;
        if let Some(waker) = mailbox.waker.take() {
            waker.wake();
        }
    }
}

/// A running shell: PTY reader thread plus the shared terminal state.
pub struct Session {
    pub(super) term: Arc<FairMutex<Term<Listener>>>,
    pub(super) notifier: Notifier,
    pub(super) listener: Listener,
}

impl Session {
    /// Starts `shell` (or the user's default shell when `None`) in `cwd`.
    pub fn spawn(cwd: PathBuf, shell: Option<String>) -> anyhow::Result<Self> {
        let size = TermSize::new(80, 24);
        let window_size = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let options = tty::Options {
            working_directory: Some(cwd),
            shell: shell.map(|program| tty::Shell::new(program, vec!["-l".into()])),
            env: [("TERM".to_string(), "xterm-256color".to_string())].into(),
            ..Default::default()
        };
        let pty = tty::new(&options, window_size, 0)?;

        let listener = Listener::default();
        let config = term::Config {
            scrolling_history: SCROLLBACK,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, false, false)?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();
        Ok(Self {
            term,
            notifier,
            listener,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Session;

    #[test]
    fn runs_a_shell_command() {
        use alacritty_terminal::event::Notify as _;
        use alacritty_terminal::grid::Dimensions as _;
        use alacritty_terminal::index::{Column, Line};

        let session = Session::spawn(std::env::temp_dir(), None).expect("spawn shell");
        session.notifier.notify(b"echo power-$((6*7))\r".as_slice());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let screen: String = {
                let term = session.term.lock();
                let grid = term.grid();
                (0..term.screen_lines() as i32)
                    .map(|line| {
                        (0..term.columns())
                            .map(|col| grid[Line(line)][Column(col)].c)
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            if screen.contains("power-42") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no output, screen:\n{screen}"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
