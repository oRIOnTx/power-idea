//! JetBrains-style macros in run configurations (`$FilePath$`, `$ProjectDir$`…),
//! so one configuration can run whichever file is open in the editor.

use std::path::Path;

use anyhow::{Result, bail};

use super::RunConfig;

/// Macros that need a file open in the editor.
const FILE_MACROS: [&str; 5] = [
    "$FilePath$",
    "$FileRelativePath$",
    "$FileDir$",
    "$FileName$",
    "$FileNameWithoutExtension$",
];

impl RunConfig {
    /// True when the configuration refers to the file open in the editor.
    pub fn uses_current_file(&self) -> bool {
        self.texts()
            .any(|text| FILE_MACROS.iter().any(|m| text.contains(m)))
    }

    /// The configuration with its macros replaced. `file` is the editor's file
    /// (absolute); values are shell-quoted in `command` and plain elsewhere.
    pub fn expand(&self, root: &Path, file: Option<&Path>) -> Result<RunConfig> {
        if file.is_none() && self.uses_current_file() {
            bail!(
                "{} runs the current file: open one in the editor first",
                self.name
            );
        }
        let values = macro_values(root, file);
        let plain = |text: &str| replace(text, &values, false);
        let mut expanded = self.clone();
        expanded.command = replace(&self.command, &values, true);
        expanded.cwd = self.cwd.as_deref().map(plain);
        expanded.env = self.env.iter().map(|(k, v)| (k.clone(), plain(v))).collect();
        if let Some(debug) = expanded.debug.as_mut() {
            debug.program = debug.program.as_deref().map(plain);
            debug.args = debug.args.iter().map(|a| plain(a)).collect();
            debug.build = debug.build.as_deref().map(|b| replace(b, &values, true));
        }
        Ok(expanded)
    }

    fn texts(&self) -> impl Iterator<Item = &str> {
        let debug = self.debug.iter().flat_map(|d| {
            d.program
                .iter()
                .chain(d.build.iter())
                .chain(d.args.iter())
                .map(String::as_str)
        });
        std::iter::once(self.command.as_str())
            .chain(self.cwd.as_deref())
            .chain(self.env.values().map(String::as_str))
            .chain(debug)
    }
}

fn macro_values(root: &Path, file: Option<&Path>) -> Vec<(&'static str, String)> {
    let mut values = vec![("$ProjectDir$", root.display().to_string())];
    let Some(file) = file else { return values };
    let text = |p: Option<&std::ffi::OsStr>| p.map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let relative = file.strip_prefix(root).unwrap_or(file);
    values.extend([
        ("$FilePath$", file.display().to_string()),
        ("$FileRelativePath$", relative.display().to_string()),
        (
            "$FileDir$",
            file.parent()
                .map_or_else(String::new, |p| p.display().to_string()),
        ),
        ("$FileName$", text(file.file_name())),
        ("$FileNameWithoutExtension$", text(file.file_stem())),
    ]);
    values
}

fn replace(text: &str, values: &[(&str, String)], quote: bool) -> String {
    values.iter().fold(text.to_string(), |text, (name, value)| {
        if quote {
            text.replace(name, &shell_quote(value))
        } else {
            text.replace(name, value)
        }
    })
}

/// Single-quotes `value` for a POSIX shell unless it is plainly safe.
fn shell_quote(value: &str) -> String {
    let safe = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | ':' | ','));
    if safe {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::shell_quote;
    use crate::{DebugSpec, RunConfig};

    #[test]
    fn expands_file_macros() {
        let config = RunConfig {
            cwd: Some("$FileDir$".into()),
            ..RunConfig::new(
                "current",
                "python3 $FilePath$ # $FileRelativePath$ $FileNameWithoutExtension$",
            )
        }
        .with_debug(DebugSpec {
            program: Some("$FilePath$".into()),
            ..DebugSpec::adapter("python")
        });
        assert!(config.uses_current_file());
        let root = Path::new("/p/my app");
        let run = config.expand(root, Some(&root.join("tools/do it.py"))).unwrap();
        assert_eq!(
            run.command,
            "python3 '/p/my app/tools/do it.py' # 'tools/do it.py' 'do it'"
        );
        assert_eq!(run.cwd.as_deref(), Some("/p/my app/tools"));
        assert_eq!(
            run.debug.unwrap().program.as_deref(),
            Some("/p/my app/tools/do it.py")
        );
        assert!(config.expand(root, None).is_err());
    }

    #[test]
    fn plain_configs_need_no_file() {
        let config = RunConfig::new("x", "$ProjectDir$/bin/run");
        assert!(!config.uses_current_file());
        assert_eq!(
            config.expand(Path::new("/p"), None).unwrap().command,
            "/p/bin/run"
        );
    }

    #[test]
    fn quotes_for_the_shell() {
        assert_eq!(shell_quote("/a/b.py"), "/a/b.py");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
