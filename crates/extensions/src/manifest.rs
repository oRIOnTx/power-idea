//! `extension.toml`: what an extension is called, how to start it and what it contributes.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

pub const MANIFEST_FILE: &str = "extension.toml";

/// Event names an extension can subscribe to (`events = [...]`).
pub const EVENT_NAMES: &[&str] = &[
    "file_opened",
    "file_closed",
    "file_saved",
    "file_changed",
    "active_editor_changed",
];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Unique id, e.g. `hello-world`: lowercase letters, digits, `-`, `_` and `.`.
    pub id: String,
    /// Display name; the id when empty.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Executable to start: a path relative to the extension folder (`./main.py`,
    /// `bin/ext`) or a program on `PATH` (`node`, `python3`).
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Events to receive; an extension with events starts with the project,
    /// one without starts on its first command.
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    /// Sent back in `command/execute`; unique across all extensions.
    pub id: String,
    /// Shown in Find Action as "<extension name>: <title>".
    pub title: String,
    /// Default key binding in gpui syntax (`cmd-alt-h`), active in project windows.
    #[serde(default)]
    pub keybinding: Option<String>,
}

/// An installed extension: its manifest and folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    pub manifest: Manifest,
    pub dir: PathBuf,
}

impl Extension {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn name(&self) -> &str {
        if self.manifest.name.is_empty() {
            &self.manifest.id
        } else {
            &self.manifest.name
        }
    }

    /// Starts with the project (it listens to events) rather than on first use.
    pub fn starts_eagerly(&self) -> bool {
        !self.manifest.events.is_empty()
    }

    pub fn subscribes_to(&self, event: &str) -> bool {
        self.manifest.events.iter().any(|e| e == event)
    }

    pub fn command(&self, id: &str) -> Option<&CommandSpec> {
        self.manifest.commands.iter().find(|c| c.id == id)
    }

    /// The executable: relative paths resolve against the extension folder.
    pub fn program(&self) -> PathBuf {
        let command = Path::new(&self.manifest.command);
        if command.components().count() > 1 || command.is_absolute() {
            self.dir.join(command)
        } else {
            command.to_path_buf()
        }
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self> {
        let manifest: Manifest = toml::from_str(text)?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        if !is_valid_id(&self.id) {
            bail!(
                "id {:?} must use lowercase letters, digits, '-', '_' or '.'",
                self.id
            );
        }
        if self.command.trim().is_empty() {
            bail!("command is empty");
        }
        if let Some(event) = self.events.iter().find(|e| !EVENT_NAMES.contains(&e.as_str())) {
            bail!("unknown event {event:?} (known: {})", EVENT_NAMES.join(", "));
        }
        let mut seen = HashSet::new();
        for command in &self.commands {
            if command.id.trim().is_empty() || command.title.trim().is_empty() {
                bail!("every command needs an id and a title");
            }
            if !seen.insert(&command.id) {
                bail!("command {:?} is declared twice", command.id);
            }
        }
        Ok(())
    }
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// What `discover` found: usable extensions and the folders that failed, with why.
#[derive(Debug, Default)]
pub struct Discovery {
    pub extensions: Vec<Extension>,
    pub errors: Vec<String>,
}

/// Loads every `<dir>/*/extension.toml`, sorted by folder name. Duplicate extension
/// or command ids are reported and the later folder is skipped.
pub fn discover(dir: &Path) -> Discovery {
    let mut found = Discovery::default();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join(MANIFEST_FILE).is_file())
        .collect();
    folders.sort();

    let mut commands = HashSet::new();
    for folder in folders {
        let name = folder
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let manifest = match load(&folder) {
            Ok(manifest) => manifest,
            Err(err) => {
                found.errors.push(format!("{name}: {err:#}"));
                continue;
            }
        };
        if found.extensions.iter().any(|e| e.id() == manifest.id) {
            found
                .errors
                .push(format!("{name}: extension id {:?} is already used", manifest.id));
            continue;
        }
        if let Some(clash) = manifest.commands.iter().find(|c| commands.contains(&c.id)) {
            found
                .errors
                .push(format!("{name}: command {:?} is already used", clash.id));
            continue;
        }
        commands.extend(manifest.commands.iter().map(|c| c.id.clone()));
        found.extensions.push(Extension {
            manifest,
            dir: folder,
        });
    }
    found
}

fn load(folder: &Path) -> Result<Manifest> {
    let path = folder.join(MANIFEST_FILE);
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Manifest::parse(&text).context(MANIFEST_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = r#"
        id = "hello"
        name = "Hello"
        command = "./main.py"
        events = ["file_saved"]

        [[commands]]
        id = "hello.greet"
        title = "Greet"
        keybinding = "cmd-alt-g"
    "#;

    #[test]
    fn parses_a_manifest() {
        let manifest = Manifest::parse(HELLO).unwrap();
        assert_eq!(manifest.id, "hello");
        assert_eq!(manifest.commands[0].keybinding.as_deref(), Some("cmd-alt-g"));
        assert_eq!(manifest.events, ["file_saved"]);
    }

    #[test]
    fn rejects_bad_manifests() {
        assert!(Manifest::parse("id = \"Hello World\"\ncommand = \"x\"").is_err());
        assert!(Manifest::parse("id = \"a\"\ncommand = \" \"").is_err());
        assert!(Manifest::parse("id = \"a\"\ncommand = \"x\"\nevents = [\"boom\"]").is_err());
        assert!(Manifest::parse("id = \"a\"\ncommand = \"x\"\ncolor = 1").is_err());
        let twice = "id = \"a\"\ncommand = \"x\"\n[[commands]]\nid = \"c\"\ntitle = \"C\"\n\
                     [[commands]]\nid = \"c\"\ntitle = \"D\"";
        assert!(Manifest::parse(twice).is_err());
    }

    #[test]
    fn resolves_the_program() {
        let ext = |command: &str| Extension {
            manifest: Manifest {
                command: command.into(),
                ..Manifest::parse(HELLO).unwrap()
            },
            dir: PathBuf::from("/ext/hello"),
        };
        assert_eq!(ext("./main.py").program(), PathBuf::from("/ext/hello/./main.py"));
        assert_eq!(ext("bin/run").program(), PathBuf::from("/ext/hello/bin/run"));
        assert_eq!(ext("node").program(), PathBuf::from("node"));
        assert!(ext("x").starts_eagerly());
    }

    #[test]
    fn discovers_folders_and_reports_clashes() {
        let dir = std::env::temp_dir().join(format!("pi-ext-discover-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (folder, text) in [
            ("a", HELLO),
            ("b", HELLO),
            ("c", "id = \"broken\""),
            (
                "d",
                "id = \"other\"\ncommand = \"x\"\n[[commands]]\nid = \"hello.greet\"\ntitle = \"T\"",
            ),
        ] {
            fs::create_dir_all(dir.join(folder)).unwrap();
            fs::write(dir.join(folder).join(MANIFEST_FILE), text).unwrap();
        }
        fs::create_dir_all(dir.join("no-manifest")).unwrap();

        let found = discover(&dir);
        assert_eq!(found.extensions.len(), 1);
        assert_eq!(found.extensions[0].dir, dir.join("a"));
        assert_eq!(found.errors.len(), 3, "{:?}", found.errors);
        assert!(found.errors[0].starts_with("b: extension id"));
        let _ = fs::remove_dir_all(&dir);
    }
}
