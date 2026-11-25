//! Database tool window: project data sources, schema browser and query
//! consoles (SQL for PostgreSQL/MySQL/SQLite, commands for Redis).

mod completion;
mod connection_dialog;
pub mod console;
mod database_picker;
mod editing;
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
pub use database_picker::open_database_picker;

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
    /// Connections to the databases picked with Discover Databases, keyed by (data source id,
    /// database). A database equal to the data source's own `database` uses `states` instead.
    catalogs: HashMap<(String, String), ConnState>,
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
            catalogs: HashMap::new(),
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
        self.forget(&config.id);
        match self.configs.iter_mut().find(|c| c.id == config.id) {
            Some(existing) => *existing = config,
            None => self.configs.push(config),
        }
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        self.configs.retain(|c| c.id != id);
        self.forget(id);
        db::delete_password(id);
        self.save()
    }

    /// Drops every connection of the data source `id` (its own and its databases').
    pub fn forget(&mut self, id: &str) {
        self.states.remove(id);
        self.catalogs.retain(|(owner, _), _| owner != id);
    }

    /// Saves the databases shown under `id` and drops connections to the ones no longer shown.
    pub fn set_databases(&mut self, id: &str, databases: Vec<String>) -> Result<()> {
        let config = self
            .configs
            .iter_mut()
            .find(|c| c.id == id)
            .context("the data source was removed")?;
        self.catalogs
            .retain(|(owner, name), _| owner != id || databases.contains(name));
        config.databases = databases;
        self.save()
    }

    /// The `catalogs` key for `database` of `id`; `None` means the data source's own connection.
    fn catalog_key(&self, id: &str, database: Option<&str>) -> Option<(String, String)> {
        let database = database?;
        let own = self.config(id).is_some_and(|c| c.database == database);
        (!own).then(|| (id.to_string(), database.to_string()))
    }

    /// Connection state of `id`, or of one of its databases.
    pub fn state(&self, id: &str, database: Option<&str>) -> Option<&ConnState> {
        match self.catalog_key(id, database) {
            Some(key) => self.catalogs.get(&key),
            None => self.states.get(id),
        }
    }

    pub fn set_state(&mut self, id: &str, database: Option<&str>, state: ConnState) {
        match self.catalog_key(id, database) {
            Some(key) => self.catalogs.insert(key, state),
            None => self.states.insert(id.to_string(), state),
        };
    }

    /// Schema loaded on connect (for console completion).
    pub fn schema(&self, id: &str, database: Option<&str>) -> Vec<SchemaNode> {
        match self.state(id, database) {
            Some(ConnState::Connected { schema, .. }) => schema.clone(),
            _ => Vec::new(),
        }
    }

    pub fn connected(&self, id: &str, database: Option<&str>) -> Option<Arc<Database>> {
        match self.state(id, database) {
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;

    use db::ConnectionConfig;

    use super::{ConnState, DataSourcesFile, DatabaseStore};

    fn store(configs: Vec<ConnectionConfig>) -> DatabaseStore {
        DatabaseStore {
            file: std::env::temp_dir().join(format!("power-idea-dbstore-{}.toml", std::process::id())),
            configs,
            states: HashMap::new(),
            catalogs: HashMap::new(),
            expanded: HashSet::new(),
        }
    }

    fn source(database: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: "ds".into(),
            database: database.into(),
            ..Default::default()
        }
    }

    #[test]
    fn data_sources_without_databases_still_load_and_omit_the_key() {
        let file: DataSourcesFile = toml::from_str("[[connection]]\nid = \"ds\"\nkind = \"mysql\"").unwrap();
        assert!(file.connections[0].databases.is_empty());
        assert!(!toml::to_string(&file).unwrap().contains("databases"));
    }

    #[test]
    fn the_own_database_reuses_the_data_source_connection() {
        let mut store = store(vec![source("app")]);
        store.set_state("ds", Some("app"), ConnState::Failed("own".into()));
        store.set_state("ds", Some("other"), ConnState::Failed("other".into()));
        assert!(matches!(store.state("ds", None), Some(ConnState::Failed(e)) if e == "own"));
        assert!(matches!(store.state("ds", Some("other")), Some(ConnState::Failed(e)) if e == "other"));
        store.forget("ds");
        assert!(store.state("ds", Some("other")).is_none());
    }

    #[test]
    fn set_databases_drops_connections_of_hidden_databases() {
        let mut store = store(vec![source("")]);
        store.set_state("ds", Some("a"), ConnState::Connecting);
        store.set_state("ds", Some("b"), ConnState::Connecting);
        store.set_databases("ds", vec!["b".into()]).unwrap();
        assert!(store.state("ds", Some("a")).is_none());
        assert!(store.state("ds", Some("b")).is_some());
        assert_eq!(store.configs[0].databases, ["b"]);
        assert!(store.set_databases("gone", Vec::new()).is_err());
        let _ = fs::remove_file(&store.file);
    }
}
