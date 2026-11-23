//! Mouse and wheel handling of the terminal view: events go to full-screen
//! apps that asked for them (hold Shift to select text instead), otherwise
//! they select text and scroll the history.

use alacritty_terminal::grid::Scroll;
use gpui_kit::*;

use super::TerminalView;
use super::mouse::{Button, Mods, MouseAction, alternate_scroll, mouse_report, wants_mouse, wheel_lines};

/// Wheel events sent per scroll gesture at most, so a fling does not flood the app.
const MAX_WHEEL_EVENTS: i32 = 10;

fn button(button: MouseButton) -> Option<Button> {
    match button {
        MouseButton::Left => Some(Button::Left),
        MouseButton::Middle => Some(Button::Middle),
        MouseButton::Right => Some(Button::Right),
        MouseButton::Navigate(_) => None,
    }
}

fn mods(modifiers: Modifiers) -> Mods {
    Mods {
        shift: modifiers.shift,
        alt: modifiers.alt,
        control: modifiers.control,
    }
}

impl TerminalView {
    /// Viewport cell (column, line) under a window position.
    pub(super) fn cell_at(&self, position: Point<Pixels>) -> (usize, usize) {
        let metrics = self.metrics.unwrap_or(super::CellMetrics {
            width: px(8.),
            height: px(16.),
        });
        let local = position - self.origin;
        let column = ((local.x / metrics.width).max(0.) as usize).min(self.size.columns.saturating_sub(1));
        let line =
            ((local.y / metrics.height).max(0.) as usize).min(self.size.screen_lines.saturating_sub(1));
        (column, line)
    }

    /// Sends a mouse event to the running app; returns false when the IDE should handle it.
    pub(super) fn report_mouse(
        &mut self,
        action: MouseAction,
        position: Point<Pixels>,
        modifiers: Modifiers,
    ) -> bool {
        let mode = *self.term.lock().mode();
        if modifiers.shift || !wants_mouse(mode) {
            return false;
        }
        let (column, line) = self.cell_at(position);
        if matches!(action, MouseAction::Move(_)) {
            // Only report when the pointer enters another cell.
            if self.last_mouse_cell == Some((column, line)) {
                return true;
            }
            self.last_mouse_cell = Some((column, line));
        }
        if let Some(bytes) = mouse_report(mode, action, column, line, mods(modifiers)) {
            self.write(bytes);
        }
        true
    }

    pub(super) fn on_mouse_down_button(&mut self, event: &MouseDownEvent) -> bool {
        button(event.button)
            .is_some_and(|b| self.report_mouse(MouseAction::Press(b), event.position, event.modifiers))
    }

    pub(super) fn on_mouse_up_button(&mut self, event: &MouseUpEvent) -> bool {
        button(event.button)
            .is_some_and(|b| self.report_mouse(MouseAction::Release(b), event.position, event.modifiers))
    }

    pub(super) fn on_mouse_motion(&mut self, event: &MouseMoveEvent) -> bool {
        let held = event.pressed_button.and_then(button);
        self.report_mouse(MouseAction::Move(held), event.position, event.modifiers)
    }

    pub(super) fn on_wheel(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(metrics) = self.metrics else { return };
        if event.touch_phase == TouchPhase::Started {
            self.wheel_pending = 0.0;
        }
        let pixels = event.delta.pixel_delta(metrics.height).y / px(1.);
        let lines = wheel_lines(&mut self.wheel_pending, pixels, metrics.height / px(1.));
        if lines == 0 {
            return;
        }
        let mode = *self.term.lock().mode();
        if wants_mouse(mode) && !event.modifiers.shift {
            let action = if lines > 0 {
                MouseAction::WheelUp
            } else {
                MouseAction::WheelDown
            };
            for _ in 0..lines.abs().min(MAX_WHEEL_EVENTS) {
                self.report_mouse(action, event.position, event.modifiers);
            }
        } else if let Some(bytes) = alternate_scroll(mode, lines.clamp(-MAX_WHEEL_EVENTS, MAX_WHEEL_EVENTS)) {
            self.write(bytes);
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}
