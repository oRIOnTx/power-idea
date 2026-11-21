//! Database tool window: project data sources, schema browser and query
//! consoles (SQL for PostgreSQL/MySQL/SQLite, commands for Redis).

mod connection_dialog;
pub mod console;
mod results;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use db::{ConnectionConfig, Database, SchemaNode};
use serde::{Deserialize, Serialize};

pub use connection_dialog::open_connection_dialog;
pub use console::QueryConsole;

/// Connection state of one data source.
pub enum ConnState {
    Connecting,
    Connected {
        db: Arc<Database>,
        schema: Vec<SchemaNode>,
    },
    Failed(String),
}

#[derive(Default, Serialize, Deserialize)]
struct DataSourcesFile {
    #[serde(default, rename = "connection")]
    connections: Vec<ConnectionConfig>,
}

/// Data sources of one project, persisted in `.power-idea/databases.toml`
/// (passwords stay in the Keychain).
pub struct DatabaseStore {
    file: PathBuf,
    pub configs: Vec<ConnectionConfig>,
    pub states: HashMap<String, ConnState>,
    /// Expanded tree nodes, keyed by "conn-id/schema/table".
    pub expanded: HashSet<String>,
}

impl DatabaseStore {
    pub fn load(project_root: &Path) -> Self {
        let file = project_root.join(".power-idea").join("databases.toml");
        let configs = fs::read_to_string(&file)
            .ok()
            .and_then(|text| toml::from_str::<DataSourcesFile>(&text).ok())
            .map_or_default(|f| f.connections);
        Self {
            file,
            configs,
            states: HashMap::new(),
            expanded: HashSet::new(),
        }
    }

    fn save(&self) -> Result<()> {
        fs::create_dir_all(self.file.parent().context("invalid path")?)?;
        let text = toml::to_string_pretty(&DataSourcesFile {
            connections: self.configs.clone(),
        })?;
        fs::write(&self.file, text).with_context(|| format!("writing {}", self.file.display()))
    }

    pub fn config(&self, id: &str) -> Option<&ConnectionConfig> {
        self.configs.iter().find(|c| c.id == id)
    }

    /// Adds or replaces a data source and drops its stale connection.
    pub fn upsert(&mut self, config: ConnectionConfig) -> Result<()> {
        self.states.remove(&config.id);
        match self.configs.iter_mut().find(|c| c.id == config.id) {
            Some(existing) => *existing = config,
            None => self.configs.push(config),
        }
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        self.configs.retain(|c| c.id != id);
        self.states.remove(id);
        db::delete_password(id);
        self.save()
    }

    pub fn connected(&self, id: &str) -> Option<Arc<Database>> {
        match self.states.get(id) {
            Some(ConnState::Connected { db, .. }) => Some(db.clone()),
            _ => None,
        }
    }
}

/// A fresh unique id for a new data source.
pub fn new_connection_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("ds-{nanos:x}")
}

/// Connects in a blocking way (call from a background task).
pub fn connect(config: &ConnectionConfig) -> Result<(Arc<Database>, Vec<SchemaNode>)> {
    let password = db::load_password(&config.id);
    let database = Database::connect(config, password.as_deref())?;
    let schema = database.schema().unwrap_or_default();
    Ok((Arc::new(database), schema))
}
