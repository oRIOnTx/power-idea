//! Current directories of terminal shells, read from the OS (`lsof` on macOS,
//! `/proc/<pid>/cwd` on Linux). Blocking: call it off the UI thread.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

/// Current directory of each shell in `pids` (the PID a tab spawned) that could be read.
pub fn shell_cwds(pids: &[u32]) -> HashMap<u32, PathBuf> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let table = Command::new("ps")
        .args(["-axo", "pid=,ppid=,comm="])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(String::new, |o| String::from_utf8_lossy(&o.stdout).into_owned());
    let shells: Vec<(u32, u32)> = pids
        .iter()
        .map(|&pid| (pid, interactive_shell(&table, pid)))
        .collect();
    let dirs = read_cwds(&shells.iter().map(|&(_, shell)| shell).collect::<Vec<_>>());
    shells
        .into_iter()
        .filter_map(|(pid, shell)| Some((pid, dirs.get(&shell)?.clone())))
        .collect()
}

/// The spawned PID may be `login`, which forks the user's shell: follow it down to the shell.
fn interactive_shell(ps_table: &str, pid: u32) -> u32 {
    let rows: Vec<(u32, u32, &str)> = ps_table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            Some((pid, ppid, fields.next()?))
        })
        .collect();
    let is_login = |pid: u32| {
        rows.iter()
            .any(|&(p, _, comm)| p == pid && comm.ends_with("login"))
    };
    if !is_login(pid) {
        return pid;
    }
    rows.iter()
        .find(|&&(_, ppid, comm)| ppid == pid && is_shell(comm))
        .map_or(pid, |&(child, _, _)| child)
}

fn is_shell(comm: &str) -> bool {
    let name = comm.rsplit('/').next().unwrap_or(comm).trim_start_matches('-');
    matches!(
        name,
        "zsh" | "bash" | "fish" | "sh" | "dash" | "ksh" | "tcsh" | "nu"
    )
}

#[cfg(target_os = "linux")]
fn read_cwds(pids: &[u32]) -> HashMap<u32, PathBuf> {
    pids.iter()
        .filter_map(|&pid| Some((pid, std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?)))
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn read_cwds(pids: &[u32]) -> HashMap<u32, PathBuf> {
    let list = pids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    Command::new("lsof")
        .args(["-a", "-d", "cwd", "-Fn", "-p", &list])
        .output()
        .map_or_else(
            |_| HashMap::new(),
            |o| parse_lsof(&String::from_utf8_lossy(&o.stdout)),
        )
}

/// `lsof -F` output: `p<pid>` starts a process, `n<path>` is its file name.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn parse_lsof(output: &str) -> HashMap<u32, PathBuf> {
    let mut dirs = HashMap::new();
    let mut pid = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            pid = rest.parse().ok();
        } else if let (Some(rest), Some(pid)) = (line.strip_prefix('n'), pid) {
            dirs.insert(pid, PathBuf::from(rest));
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{interactive_shell, parse_lsof, shell_cwds};

    #[test]
    fn parses_lsof_fields() {
        let dirs = parse_lsof("p12\nfcwd\nn/Users/me/app\np34\nfcwd\nn/tmp\n");
        assert_eq!(dirs.get(&12), Some(&PathBuf::from("/Users/me/app")));
        assert_eq!(dirs.get(&34), Some(&PathBuf::from("/tmp")));
    }

    #[test]
    fn follows_login_to_the_shell() {
        let table = "  10     1 login\n  11    10 -zsh\n  12    11 vim\n  20     1 /bin/zsh\n";
        assert_eq!(interactive_shell(table, 10), 11);
        assert_eq!(
            interactive_shell(table, 20),
            20,
            "a shell spawned directly is itself"
        );
    }

    #[test]
    fn reads_the_cwd_of_a_running_process() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("10")
            .current_dir(&dir)
            .spawn()
            .unwrap();
        let dirs = shell_cwds(&[child.id()]);
        let _ = child.kill();
        let _ = child.wait();
        let got = dirs.get(&child.id()).and_then(|d| d.canonicalize().ok());
        assert_eq!(got, Some(dir));
    }
}
