//! Integrated terminal: `alacritty_terminal` handles the PTY and VT parsing,
//! this module renders the grid with gpui and translates input.

mod colors;
mod input;
mod session;

use std::borrow::Cow;

use alacritty_terminal::event::{Event as TermEvent, Notify, WindowSize};
use alacritty_terminal::event_loop::{Msg, Notifier};
use alacritty_terminal::grid::{Dimensions as _, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point as GridPoint};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use std::sync::Arc;

use self::colors::{ansi_color, to_rgb};
use self::input::keystroke_to_bytes;
use self::session::Listener;
pub use self::session::Session;
use crate::actions::TERMINAL_CONTEXT;
use crate::config::AppConfig;

actions!(terminal, [Copy, Paste, Clear]);

const LINE_HEIGHT_FACTOR: f32 = 1.35;

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

    /// Types `text` at the prompt without pressing Enter.
    pub fn type_text(&self, text: &str) {
        self.write(text.as_bytes().to_vec());
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
            .map_or(size * 0.6, |s| s.width);
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
