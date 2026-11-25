//! macOS notifications for finished runs while the IDE is in the background, through
//! `osascript` (no notification framework bindings in the app).

use std::process::{Command, Stdio};

/// Shows a system notification; failures are silent (a notification is a courtesy).
pub fn os_notification(title: &str, body: &str, failed: bool) {
    let sound = if failed { " sound name \"Basso\"" } else { "" };
    let script = format!(
        "display notification {} with title \"Power IDEA\" subtitle {}{sound}",
        applescript_string(body),
        applescript_string(title),
    );
    let _ = Command::new("osascript")
        .args(["-e", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// An AppleScript string literal: quotes and backslashes escaped, control characters dropped.
fn applescript_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::applescript_string;

    #[test]
    fn quotes_applescript_strings() {
        assert_eq!(applescript_string("CI #1 failed"), "\"CI #1 failed\"");
        assert_eq!(
            applescript_string("say \"hi\" \\ bye\nnow"),
            "\"say \\\"hi\\\" \\\\ bye now\""
        );
    }
}
