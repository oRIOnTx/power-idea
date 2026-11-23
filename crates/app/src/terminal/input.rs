//! Keyboard input translation (xterm sequences).

use alacritty_terminal::term::TermMode;
use gpui_kit::Keystroke;

/// Translates a keystroke into bytes for the PTY, or `None` to let the IDE handle it.
pub(super) fn keystroke_to_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
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
        if let &[key] = keystroke.key.as_bytes() {
            let byte = match key {
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

    use super::keystroke_to_bytes;

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
}
