//! Integrated terminal: `alacritty_terminal` handles the PTY and VT parsing,
//! this module renders the grid with gpui and translates input.

use std::borrow::Cow;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point as GridPoint};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Rgb};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::theme::Theme;
use gpui_kit::*;

use crate::actions::TERMINAL_CONTEXT;
use crate::config::AppConfig;

actions!(terminal, [Copy, Paste, Clear]);

const SCROLLBACK: usize = 10_000;
const LINE_HEIGHT_FACTOR: f32 = 1.35;

/// Events from the PTY thread, plus a waker for the UI task awaiting them.
#[derive(Default)]
struct Mailbox {
    events: Vec<TermEvent>,
    waker: Option<Waker>,
    closed: bool,
}

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Mailbox>>);

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
    fn next_batch(&self) -> impl Future<Output = Option<Vec<TermEvent>>> + '_ {
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

    fn close(&self) {
        let mut mailbox = self.0.lock().unwrap();
        mailbox.closed = true;
        if let Some(waker) = mailbox.waker.take() {
            waker.wake();
        }
    }
}

pub enum TerminalEvent {
    TitleChanged,
    Exited,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

#[derive(Clone, Copy, PartialEq)]
struct CellMetrics {
    width: Pixels,
    height: Pixels,
}

pub struct TerminalView {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    listener: Listener,
    focus_handle: FocusHandle,
    pub title: String,
    size: TermSize,
    metrics: Option<CellMetrics>,
    /// Top-left of the grid in window coordinates, captured at layout.
    origin: Point<Pixels>,
    selecting: bool,
    _events_task: Task<()>,
}

/// A running shell: PTY reader thread plus the shared terminal state.
pub struct Session {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    listener: Listener,
}

impl Session {
    pub fn spawn(cwd: PathBuf) -> anyhow::Result<Self> {
        let size = TermSize::new(80, 24);
        let window_size = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let options = tty::Options {
            working_directory: Some(cwd),
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

impl TerminalView {
    pub fn new(session: Session, cx: &mut Context<Self>) -> Self {
        let Session {
            term,
            notifier,
            listener,
        } = session;
        let events = listener.clone();
        let events_task = cx.spawn(async move |this, cx| {
            while let Some(batch) = events.next_batch().await {
                let keep_going = this
                    .update(cx, |this, cx| this.handle_events(batch, cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        });

        Self {
            term,
            notifier,
            listener,
            focus_handle: cx.focus_handle(),
            title: "Local".into(),
            size: TermSize::new(80, 24),
            metrics: None,
            origin: Point::default(),
            selecting: false,
            _events_task: events_task,
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Returns false once the shell has exited.
    fn handle_events(&mut self, events: Vec<TermEvent>, cx: &mut Context<Self>) -> bool {
        for event in events {
            match event {
                TermEvent::Title(title) => {
                    self.title = title;
                    cx.emit(TerminalEvent::TitleChanged);
                }
                TermEvent::ResetTitle => {
                    self.title = "Local".into();
                    cx.emit(TerminalEvent::TitleChanged);
                }
                TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
                TermEvent::ClipboardStore(_, text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                TermEvent::ClipboardLoad(_, format) => {
                    let text = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .unwrap_or_default();
                    self.write(format(&text).into_bytes());
                }
                TermEvent::ColorRequest(index, format) => {
                    let rgb = to_rgb(ansi_color(&AnsiColor::Indexed(index as u8), cx));
                    self.write(format(rgb).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(format) => {
                    let size = self.window_size();
                    self.write(format(size).into_bytes());
                }
                TermEvent::Exit | TermEvent::ChildExit(_) => {
                    self.listener.close();
                    cx.emit(TerminalEvent::Exited);
                    return false;
                }
                _ => {}
            }
        }
        cx.notify();
        true
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(bytes);
    }

    fn window_size(&self) -> WindowSize {
        let metrics = self.metrics.unwrap_or(CellMetrics {
            width: px(8.),
            height: px(16.),
        });
        WindowSize {
            num_lines: self.size.screen_lines as u16,
            num_cols: self.size.columns as u16,
            cell_width: f32::from(metrics.width) as u16,
            cell_height: f32::from(metrics.height) as u16,
        }
    }

    fn font(cx: &App) -> (Font, Pixels) {
        let theme = cx.theme();
        let size = px(cx.global::<AppConfig>().0.editor_font_size);
        (font(theme.mono_font_family.clone()), size)
    }

    fn measure(window: &Window, cx: &App) -> CellMetrics {
        let (font, size) = Self::font(cx);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let width = text_system
            .advance(font_id, size, 'm')
            .map(|s| s.width)
            .unwrap_or(size * 0.6);
        CellMetrics {
            width,
            height: (size * LINE_HEIGHT_FACTOR).round(),
        }
    }

    /// Resizes the grid and PTY to fit `bounds`.
    fn layout(&mut self, bounds: Bounds<Pixels>, metrics: CellMetrics) {
        self.origin = bounds.origin;
        let columns = ((bounds.size.width / metrics.width).floor() as usize).max(2);
        let lines = ((bounds.size.height / metrics.height).floor() as usize).max(1);
        if self.metrics == Some(metrics) && self.size.columns == columns && self.size.screen_lines == lines {
            return;
        }
        self.metrics = Some(metrics);
        self.size = TermSize::new(columns, lines);
        self.term.lock().resize(TermSize::new(columns, lines));
        let _ = self.notifier.0.send(Msg::Resize(self.window_size()));
    }

    fn grid_point(&self, position: Point<Pixels>, origin: Point<Pixels>) -> (GridPoint, Direction) {
        let metrics = self.metrics.unwrap_or(CellMetrics {
            width: px(8.),
            height: px(16.),
        });
        let local = position - origin;
        let col_f = (local.x / metrics.width).max(0.);
        let column = (col_f as usize).min(self.size.columns.saturating_sub(1));
        let row = ((local.y / metrics.height).max(0.) as usize).min(self.size.screen_lines.saturating_sub(1));
        let offset = self.term.lock().grid().display_offset() as i32;
        let side = if col_f.fract() < 0.5 {
            Direction::Left
        } else {
            Direction::Right
        };
        (GridPoint::new(Line(row as i32 - offset), Column(column)), side)
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mode = *self.term.lock().mode();
        let Some(bytes) = keystroke_to_bytes(&event.keystroke, mode) else {
            return;
        };
        cx.stop_propagation();
        {
            let mut term = self.term.lock();
            term.selection = None;
            term.scroll_display(Scroll::Bottom);
        }
        self.write(bytes);
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.term.lock().selection_to_string() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if bracketed {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        self.write(bytes.into_bytes());
    }

    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        // Ctrl-L lets the shell redraw its prompt on a clean screen.
        self.write(b"\x0c".as_slice());
        cx.notify();
    }

    fn render_lines(&self, cx: &App) -> (Vec<StyledText>, Option<(usize, usize, CursorShape)>) {
        let (font, _) = Self::font(cx);
        let term = self.term.lock();
        let content = term.renderable_content();
        let lines = term.screen_lines();
        let offset = content.display_offset as i32;
        let selection = content.selection;
        let default_bg = cx.theme().background;

        let mut texts: Vec<(String, Vec<TextRun>)> = vec![(String::new(), Vec::new()); lines];
        for indexed in content.display_iter {
            let cell = &indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let row = (indexed.point.line.0 + offset) as usize;
            let Some((text, runs)) = texts.get_mut(row) else {
                continue;
            };

            let mut fg = ansi_color(&cell.fg, cx);
            let mut bg = ansi_color(&cell.bg, cx);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg.a *= 0.7;
            }
            if selection.is_some_and(|s| s.contains(indexed.point)) {
                bg = cx.theme().selection;
            }
            let ch = if cell.flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };

            let mut run_font = font.clone();
            if cell.flags.contains(Flags::BOLD) {
                run_font.weight = FontWeight::BOLD;
            }
            if cell.flags.contains(Flags::ITALIC) {
                run_font.style = FontStyle::Italic;
            }
            let run = TextRun {
                len: ch.len_utf8(),
                font: run_font,
                color: fg,
                background_color: (bg != default_bg).then_some(bg),
                underline: cell
                    .flags
                    .intersects(Flags::ALL_UNDERLINES)
                    .then(|| UnderlineStyle {
                        thickness: px(1.),
                        color: Some(fg),
                        wavy: cell.flags.contains(Flags::UNDERCURL),
                    }),
                strikethrough: cell.flags.contains(Flags::STRIKEOUT).then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(fg),
                }),
            };

            text.push(ch);
            match runs.last_mut() {
                Some(last) if same_style(last, &run) => last.len += run.len,
                _ => runs.push(run),
            }
        }

        let cursor = (content.cursor.shape != CursorShape::Hidden).then(|| {
            let row = (content.cursor.point.line.0 + offset) as usize;
            (row, content.cursor.point.column.0, content.cursor.shape)
        });
        let rows = texts
            .into_iter()
            .map(|(text, runs)| StyledText::new(text).with_runs(runs))
            .collect();
        (rows, cursor.filter(|(row, ..)| *row < lines))
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
        self.listener.close();
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = Self::measure(window, cx);
        let (rows, cursor) = self.render_lines(cx);
        let theme = cx.theme();
        let focused = self.focus_handle.is_focused(window);
        let this = cx.entity().downgrade();

        let cursor_el = cursor.map(|(row, col, shape)| {
            let base = div()
                .absolute()
                .left(metrics.width * col as f32)
                .top(metrics.height * row as f32)
                .w(metrics.width)
                .h(metrics.height);
            match (focused, shape) {
                (true, CursorShape::Block) => base.bg(theme.caret.opacity(0.6)),
                (true, CursorShape::Underline) => base.border_b_2().border_color(theme.caret),
                (true, _) => base.w(px(2.)).bg(theme.caret),
                (false, _) => base.border_1().border_color(theme.caret.opacity(0.6)),
            }
        });

        div()
            .id("terminal")
            .key_context(TERMINAL_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::clear))
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .px_2()
            .pt_1()
            .bg(theme.background)
            .text_color(theme.foreground)
            .cursor(CursorStyle::IBeam)
            .child(
                div()
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                this.update(cx, |this, _| this.layout(bounds, metrics)).ok();
                                bounds
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        div()
                            .absolute()
                            .font(Self::font(cx).0)
                            .text_size(Self::font(cx).1)
                            .line_height(metrics.height)
                            .whitespace_nowrap()
                            .children(rows.into_iter().map(|row| div().h(metrics.height).child(row))),
                    )
                    .children(cursor_el)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.focus_handle.focus(window, cx);
                            let origin = this.origin;
                            let (point, side) = this.grid_point(event.position, origin);
                            let ty = match event.click_count {
                                2 => SelectionType::Semantic,
                                3 => SelectionType::Lines,
                                _ => SelectionType::Simple,
                            };
                            this.term.lock().selection = Some(Selection::new(ty, point, side));
                            this.selecting = true;
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if !this.selecting || event.pressed_button != Some(MouseButton::Left) {
                            return;
                        }
                        let origin = this.origin;
                        let (point, side) = this.grid_point(event.position, origin);
                        if let Some(selection) = this.term.lock().selection.as_mut() {
                            selection.update(point, side);
                        }
                        cx.notify();
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.selecting = false;
                            let mut term = this.term.lock();
                            if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
                                term.selection = None;
                            }
                            cx.notify();
                        }),
                    )
                    .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                        let Some(metrics) = this.metrics else { return };
                        let lines =
                            (event.delta.pixel_delta(metrics.height).y / metrics.height).round() as i32;
                        if lines != 0 {
                            this.term.lock().scroll_display(Scroll::Delta(lines));
                            cx.notify();
                        }
                    })),
            )
    }
}

fn same_style(a: &TextRun, b: &TextRun) -> bool {
    a.font == b.font
        && a.color == b.color
        && a.background_color == b.background_color
        && a.underline == b.underline
        && a.strikethrough == b.strikethrough
}

fn to_rgb(color: Hsla) -> Rgb {
    let rgba = color.to_rgb();
    Rgb {
        r: (rgba.r * 255.) as u8,
        g: (rgba.g * 255.) as u8,
        b: (rgba.b * 255.) as u8,
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Hsla {
    Rgba {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a: 1.,
    }
    .into()
}

/// Maps ANSI colors onto the active theme so the terminal matches the IDE.
fn ansi_color(color: &AnsiColor, cx: &App) -> Hsla {
    let theme: &Theme = cx.theme();
    match color {
        AnsiColor::Spec(c) => rgb(c.r, c.g, c.b),
        AnsiColor::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => theme.foreground,
            NamedColor::DimForeground => theme.muted_foreground,
            NamedColor::Background => theme.background,
            NamedColor::Cursor => theme.caret,
            other => indexed_color(*other as usize, theme),
        },
        AnsiColor::Indexed(ix) => indexed_color(*ix as usize, theme),
    }
}

fn indexed_color(ix: usize, theme: &Theme) -> Hsla {
    let dark = theme.mode.is_dark();
    match ix {
        0 => {
            if dark {
                rgb(0x3b, 0x3f, 0x45)
            } else {
                rgb(0x24, 0x29, 0x2e)
            }
        }
        1 => theme.red,
        2 => theme.green,
        3 => theme.yellow,
        4 => theme.blue,
        5 => theme.magenta,
        6 => theme.cyan,
        7 => {
            if dark {
                rgb(0xc9, 0xcd, 0xd3)
            } else {
                rgb(0x6a, 0x73, 0x7d)
            }
        }
        8 => theme.muted_foreground,
        9 => theme.red_light,
        10 => theme.green_light,
        11 => theme.yellow_light,
        12 => theme.blue_light,
        13 => theme.magenta_light,
        14 => theme.cyan_light,
        15 => {
            if dark {
                rgb(0xff, 0xff, 0xff)
            } else {
                rgb(0x2b, 0x30, 0x36)
            }
        }
        16..=231 => {
            let ix = ix - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            rgb(level(ix / 36), level((ix / 6) % 6), level(ix % 6))
        }
        232..=255 => {
            let v = (8 + (ix - 232) * 10) as u8;
            rgb(v, v, v)
        }
        _ => theme.foreground,
    }
}

/// Translates a keystroke into bytes for the PTY, or `None` to let the IDE handle it.
fn keystroke_to_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &keystroke.modifiers;
    if m.platform || m.function {
        return None;
    }
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let csi = |app: &str, normal: &str| Some(if app_cursor { app } else { normal }.as_bytes().to_vec());

    let special: Option<Vec<u8>> = match keystroke.key.as_str() {
        "enter" => Some(b"\r".to_vec()),
        "backspace" if m.alt => Some(b"\x1b\x7f".to_vec()),
        "backspace" => Some(b"\x7f".to_vec()),
        "tab" if m.shift => Some(b"\x1b[Z".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "escape" => Some(b"\x1b".to_vec()),
        "up" => csi("\x1bOA", "\x1b[A"),
        "down" => csi("\x1bOB", "\x1b[B"),
        "right" if m.alt => Some(b"\x1bf".to_vec()),
        "left" if m.alt => Some(b"\x1bb".to_vec()),
        "right" => csi("\x1bOC", "\x1b[C"),
        "left" => csi("\x1bOD", "\x1b[D"),
        "home" => csi("\x1bOH", "\x1b[H"),
        "end" => csi("\x1bOF", "\x1b[F"),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        _ => None,
    };
    if special.is_some() {
        return special;
    }

    if m.control {
        let key = keystroke.key.as_bytes();
        if key.len() == 1 {
            let byte = match key[0] {
                b @ b'a'..=b'z' => b - b'a' + 1,
                b'@' | b' ' | b'2' => 0,
                b'[' | b'3' => 0x1b,
                b'\\' | b'4' => 0x1c,
                b']' | b'5' => 0x1d,
                b'^' | b'6' => 0x1e,
                b'_' | b'-' | b'7' => 0x1f,
                _ => return None,
            };
            return Some(vec![byte]);
        }
        return None;
    }

    if m.alt {
        // Option as Meta: ESC prefix + the unmodified key.
        let mut bytes = b"\x1b".to_vec();
        let key = if m.shift {
            keystroke.key.to_uppercase()
        } else {
            keystroke.key.clone()
        };
        bytes.extend_from_slice(key.as_bytes());
        return Some(bytes);
    }

    keystroke.key_char.as_ref().map(|text| text.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::term::TermMode;
    use gpui_kit::Keystroke;

    use super::{Session, keystroke_to_bytes};

    fn bytes(key: &'static str) -> Option<Vec<u8>> {
        keystroke_to_bytes(&Keystroke::parse(key).unwrap(), TermMode::empty())
    }

    #[test]
    fn translates_keys() {
        assert_eq!(bytes("ctrl-c"), Some(vec![3]));
        assert_eq!(bytes("enter"), Some(b"\r".to_vec()));
        assert_eq!(bytes("up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(bytes("alt-b"), Some(b"\x1bb".to_vec()));
        assert_eq!(bytes("cmd-c"), None);
        assert_eq!(
            keystroke_to_bytes(&Keystroke::parse("up").unwrap(), TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
    }

    #[test]
    fn runs_a_shell_command() {
        use alacritty_terminal::event::Notify as _;
        use alacritty_terminal::grid::Dimensions as _;
        use alacritty_terminal::index::{Column, Line};

        let session = Session::spawn(std::env::temp_dir()).expect("spawn shell");
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
