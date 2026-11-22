//! Finds programs running in the foreground of terminal shells, so closing a
//! project or quitting can warn before the PTY hangup kills them.

use std::process::Command;

/// Names of the foreground programs of those `shells` (PIDs) that are not idle at
/// their prompt. Blocks on `ps`; call it off the UI thread.
pub fn foreground_programs(shells: &[u32]) -> Vec<String> {
    if shells.is_empty() {
        return Vec::new();
    }
    // `tpgid` is the terminal's foreground process group: the shell's own group while
    // it waits at the prompt, the job's group while a command runs. Background
    // helpers (prompt daemons, `&` jobs) don't count.
    let output = Command::new("ps")
        .args(["-axo", "pid=,pgid=,tpgid=,comm="])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            parse_foreground(&String::from_utf8_lossy(&output.stdout), shells)
        }
        _ => Vec::new(),
    }
}

struct Row<'a> {
    pid: i64,
    pgid: i64,
    tpgid: i64,
    command: &'a str,
}

impl<'a> Row<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let (pid, rest) = next_field(line)?;
        let (pgid, rest) = next_field(rest)?;
        let (tpgid, rest) = next_field(rest)?;
        Some(Self {
            pid: pid.parse().ok()?,
            pgid: pgid.parse().ok()?,
            tpgid: tpgid.parse().ok()?,
            command: rest.trim(),
        })
    }

    fn name(&self) -> String {
        let name = self.command.rsplit('/').next().unwrap_or(self.command);
        name.trim_start_matches('-').to_string()
    }
}

/// Splits off the first whitespace-separated field; the rest keeps its spaces.
fn next_field(text: &str) -> Option<(&str, &str)> {
    text.trim_start().split_once(char::is_whitespace)
}

fn parse_foreground(ps: &str, shells: &[u32]) -> Vec<String> {
    let rows: Vec<Row> = ps.lines().filter_map(Row::parse).collect();
    shells
        .iter()
        .filter_map(|&shell| {
            let row = rows.iter().find(|r| r.pid == i64::from(shell))?;
            if row.tpgid <= 0 || row.tpgid == row.pgid {
                return None;
            }
            // The job's group leader names it; it may already be gone in a pipeline.
            let leader = rows.iter().find(|r| r.pid == row.tpgid);
            Some(leader.map_or_else(|| "a process".to_string(), Row::name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_foreground;

    const PS: &str = "\
  100   100   100 /bin/zsh
  101   101   140 -zsh
  120   120     0 /opt/homebrew/bin/gitstatusd
  140   140   140 /Users/me/.rbenv/versions/3.3.0/bin/ruby
  141   140   140 /usr/bin/My Tool
  200   200   250 /bin/zsh
";

    #[test]
    fn idle_shells_are_not_busy() {
        assert!(parse_foreground(PS, &[100]).is_empty());
    }

    #[test]
    fn names_the_foreground_job() {
        assert_eq!(parse_foreground(PS, &[100, 101]), vec!["ruby"]);
    }

    #[test]
    fn unknown_or_vanished_processes() {
        assert_eq!(parse_foreground(PS, &[200]), vec!["a process"]);
        assert!(parse_foreground(PS, &[999]).is_empty());
    }

    #[test]
    fn commands_with_spaces_and_login_dash() {
        let ps = "  5 5 7 -zsh\n  7 7 7 /Applications/Some App.app/Contents/MacOS/Some App\n";
        assert_eq!(parse_foreground(ps, &[5]), vec!["Some App"]);
    }
}
