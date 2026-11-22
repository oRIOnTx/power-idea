//! Mouse reporting for full-screen terminal apps (vim, htop, lazygit…): xterm
//! mouse protocols in the default (X10), UTF-8 (1005) and SGR (1006) encodings.

use alacritty_terminal::term::TermMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press(Button),
    Release(Button),
    /// Movement, with the button held (if any).
    Move(Option<Button>),
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

/// Whether the running app asked for mouse events.
pub fn wants_mouse(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

fn button_code(button: Button) -> u8 {
    match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    }
}

/// Bytes reporting `action` at 0-based `(column, line)`, or `None` when the
/// current mode does not report it (or the position cannot be encoded).
pub fn mouse_report(
    mode: TermMode,
    action: MouseAction,
    column: usize,
    line: usize,
    mods: Mods,
) -> Option<Vec<u8>> {
    if !wants_mouse(mode) {
        return None;
    }
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    let mut code = match action {
        MouseAction::Press(button) => button_code(button),
        // Without SGR, a release does not say which button.
        MouseAction::Release(button) => {
            if sgr {
                button_code(button)
            } else {
                3
            }
        }
        MouseAction::Move(button) => {
            let reported = mode.contains(TermMode::MOUSE_MOTION)
                || (mode.contains(TermMode::MOUSE_DRAG) && button.is_some());
            if !reported {
                return None;
            }
            32 + button.map_or(3, button_code)
        }
        MouseAction::WheelUp => 64,
        MouseAction::WheelDown => 65,
    };
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.control {
        code += 16;
    }
    let (x, y) = (column + 1, line + 1);

    if sgr {
        let end = if matches!(action, MouseAction::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    let mut bytes = b"\x1b[M".to_vec();
    bytes.push(32 + code);
    for value in [x, y] {
        let value = 32 + value;
        if mode.contains(TermMode::UTF8_MOUSE) {
            let ch = char::from_u32(value as u32).filter(|_| value < 2048)?;
            let mut buf = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        } else {
            // The default encoding has one byte per coordinate.
            bytes.push(u8::try_from(value).ok()?);
        }
    }
    Some(bytes)
}

/// Arrow-key presses for wheel scrolling in full-screen apps that do not use
/// the mouse (less, man): xterm's "alternate scroll" mode.
pub fn alternate_scroll(mode: TermMode, lines: i32) -> Option<Vec<u8>> {
    if wants_mouse(mode) || !mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) || lines == 0 {
        return None;
    }
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let key: &[u8] = match (lines > 0, app_cursor) {
        (true, true) => b"\x1bOA",
        (true, false) => b"\x1b[A",
        (false, true) => b"\x1bOB",
        (false, false) => b"\x1b[B",
    };
    Some(key.repeat(lines.unsigned_abs() as usize))
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::term::TermMode;

    use super::{Button, Mods, MouseAction, alternate_scroll, mouse_report};

    #[test]
    fn reports_in_sgr() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        let report =
            |action| String::from_utf8(mouse_report(mode, action, 4, 9, Mods::default()).unwrap()).unwrap();
        assert_eq!(report(MouseAction::Press(Button::Left)), "\x1b[<0;5;10M");
        assert_eq!(report(MouseAction::Release(Button::Right)), "\x1b[<2;5;10m");
        assert_eq!(report(MouseAction::WheelDown), "\x1b[<65;5;10M");
        // Plain click mode does not report motion.
        assert_eq!(
            mouse_report(mode, MouseAction::Move(Some(Button::Left)), 0, 0, Mods::default()),
            None
        );
        let ctrl = Mods {
            control: true,
            ..Mods::default()
        };
        assert_eq!(
            mouse_report(mode, MouseAction::Press(Button::Left), 0, 0, ctrl).unwrap(),
            b"\x1b[<16;1;1M"
        );
    }

    #[test]
    fn reports_drag_and_default_encoding() {
        let mode = TermMode::MOUSE_DRAG;
        assert_eq!(
            mouse_report(mode, MouseAction::Move(Some(Button::Left)), 0, 0, Mods::default()).unwrap(),
            [0x1b, b'[', b'M', 32 + 32, 33, 33]
        );
        assert_eq!(
            mouse_report(mode, MouseAction::Move(None), 0, 0, Mods::default()),
            None
        );
        assert_eq!(
            mouse_report(mode, MouseAction::Release(Button::Left), 1, 2, Mods::default()).unwrap(),
            [0x1b, b'[', b'M', 35, 34, 35]
        );
        // Too far right for one byte.
        assert_eq!(
            mouse_report(mode, MouseAction::Press(Button::Left), 300, 0, Mods::default()),
            None
        );
        assert!(
            mouse_report(
                mode | TermMode::UTF8_MOUSE,
                MouseAction::Press(Button::Left),
                300,
                0,
                Mods::default()
            )
            .is_some()
        );
        assert_eq!(
            mouse_report(TermMode::empty(), MouseAction::WheelUp, 0, 0, Mods::default()),
            None
        );
    }

    #[test]
    fn scrolls_with_arrows_on_the_alternate_screen() {
        let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;
        assert_eq!(alternate_scroll(mode, 2).unwrap(), b"\x1b[A\x1b[A");
        assert_eq!(
            alternate_scroll(mode | TermMode::APP_CURSOR, -1).unwrap(),
            b"\x1bOB"
        );
        assert_eq!(alternate_scroll(TermMode::ALTERNATE_SCROLL, 1), None);
        assert_eq!(alternate_scroll(mode | TermMode::MOUSE_REPORT_CLICK, 1), None);
    }
}
