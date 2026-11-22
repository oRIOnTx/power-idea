//! Which containers belong to the project: compose services (from
//! `docker compose config --format json`, so no YAML parser is needed) and
//! containers whose compose labels point at the project folder.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::model::{Container, State};

const COMPOSE_FILES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

/// The compose file docker compose would pick in `root`, if any.
pub fn compose_file(root: &Path) -> Option<PathBuf> {
    COMPOSE_FILES
        .iter()
        .map(|name| root.join(name))
        .find(|p| p.is_file())
}

pub fn has_dockerfile(root: &Path) -> bool {
    root.join("Dockerfile").is_file()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComposeProject {
    pub name: String,
    /// Service names, sorted.
    pub services: Vec<String>,
}

/// Output of `docker compose config --format json`.
pub fn parse_compose_config(json: &str) -> Option<ComposeProject> {
    let value: Value = serde_json::from_str(json).ok()?;
    let mut services: Vec<String> = value.get("services")?.as_object()?.keys().cloned().collect();
    services.sort();
    Some(ComposeProject {
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        services,
    })
}

/// A row of the Docker panel: a container, or a compose service without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub container: Option<Container>,
}

impl Row {
    pub fn state(&self) -> State {
        self.container.as_ref().map_or(State::NotCreated, |c| c.state)
    }
}

fn belongs(container: &Container, root: &Path, project: Option<&ComposeProject>) -> bool {
    let same_dir = !container.working_dir.is_empty() && {
        let dir = Path::new(&container.working_dir);
        // Resolve symlinks when both exist (e.g. /private/tmp); otherwise compare as written.
        match (dir.canonicalize(), root.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => dir == root,
        }
    };
    same_dir || project.is_some_and(|p| !p.name.is_empty() && container.project == p.name)
}

/// Project services (with or without containers) first; with `all`, every other container too.
pub fn rows(root: &Path, project: Option<&ComposeProject>, containers: &[Container], all: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut used = vec![false; containers.len()];
    for service in project.map_or(&[][..], |p| &p.services[..]) {
        let mut found = false;
        for (ix, container) in containers.iter().enumerate() {
            if container.service == *service && belongs(container, root, project) {
                used[ix] = true;
                found = true;
                rows.push(Row {
                    title: service.clone(),
                    container: Some(container.clone()),
                });
            }
        }
        if !found {
            rows.push(Row {
                title: service.clone(),
                container: None,
            });
        }
    }
    // Project containers of services no longer in the file, then (optionally) the rest.
    for (ix, container) in containers.iter().enumerate() {
        if used[ix] {
            continue;
        }
        if all || belongs(container, root, project) {
            rows.push(Row {
                title: row_title(container).to_string(),
                container: Some(container.clone()),
            });
        }
    }
    rows
}

/// Which containers a panel group holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKind {
    /// The open project's services and containers.
    Project,
    /// Another compose project, by its name.
    Compose(String),
    /// Containers started without compose.
    Standalone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub kind: GroupKind,
    pub rows: Vec<Row>,
}

/// [`rows`] grouped like JetBrains Services: the project first, then (with `all`) other
/// compose projects by name, then standalone containers. Empty groups are left out.
pub fn groups(
    root: &Path,
    project: Option<&ComposeProject>,
    containers: &[Container],
    all: bool,
) -> Vec<Group> {
    let mut groups = vec![Group {
        kind: GroupKind::Project,
        rows: rows(root, project, containers, false),
    }];
    if all {
        let mut others: Vec<&Container> = containers.iter().filter(|c| !belongs(c, root, project)).collect();
        others.sort_by(|a, b| (&a.project, row_title(a)).cmp(&(&b.project, row_title(b))));
        for container in others {
            let kind = match container.project.as_str() {
                "" => GroupKind::Standalone,
                name => GroupKind::Compose(name.to_string()),
            };
            let row = Row {
                title: row_title(container).to_string(),
                container: Some(container.clone()),
            };
            match groups.iter_mut().find(|g| g.kind == kind) {
                Some(group) => group.rows.push(row),
                None => groups.push(Group {
                    kind,
                    rows: vec![row],
                }),
            }
        }
        // Standalone containers go last, after every compose project.
        groups.sort_by_key(|g| match g.kind {
            GroupKind::Project => 0,
            GroupKind::Compose(_) => 1,
            GroupKind::Standalone => 2,
        });
    }
    groups.retain(|g| !g.rows.is_empty());
    groups
}

fn row_title(container: &Container) -> &str {
    if container.service.is_empty() {
        &container.name
    } else {
        &container.service
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ComposeProject, GroupKind, groups, parse_compose_config, rows};
    use crate::model::{Container, State};

    fn container(name: &str, project: &str, service: &str, dir: &str, state: State) -> Container {
        Container {
            id: format!("id-{name}"),
            name: name.into(),
            image: "img".into(),
            state,
            status: String::new(),
            health: None,
            ports: Vec::new(),
            project: project.into(),
            service: service.into(),
            working_dir: dir.into(),
        }
    }

    #[test]
    fn parses_compose_config_json() {
        let project =
            parse_compose_config(r#"{"name":"shop","services":{"web":{"image":"x"},"db":{}}}"#).unwrap();
        assert_eq!(
            project,
            ComposeProject {
                name: "shop".into(),
                services: vec!["db".into(), "web".into()]
            }
        );
        assert!(parse_compose_config("nope").is_none());
    }

    #[test]
    fn matches_containers_to_services() {
        let root = Path::new("/nonexistent/shop");
        let project = ComposeProject {
            name: "shop".into(),
            services: vec!["db".into(), "redis".into(), "web".into()],
        };
        let containers = vec![
            container("shop-web-1", "shop", "web", "/nonexistent/shop", State::Running),
            container("shop-db-1", "shop", "db", "/nonexistent/shop", State::Exited),
            container("other-api-1", "other", "api", "/elsewhere", State::Running),
            container("shop-old-1", "shop", "old", "/nonexistent/shop", State::Exited),
        ];
        let titles = |all| -> Vec<(String, State)> {
            rows(root, Some(&project), &containers, all)
                .into_iter()
                .map(|r| (r.title.clone(), r.state()))
                .collect()
        };
        assert_eq!(
            titles(false),
            [
                ("db".into(), State::Exited),
                ("redis".into(), State::NotCreated),
                ("web".into(), State::Running),
                ("old".into(), State::Exited),
            ]
        );
        assert_eq!(titles(true).len(), 5);
        // Without a compose file, containers labelled with the project folder still count.
        let plain: Vec<String> = rows(root, None, &containers, false)
            .into_iter()
            .map(|r| r.title)
            .collect();
        assert_eq!(plain, ["web", "db", "old"]);
    }

    #[test]
    fn groups_other_containers_by_compose_project() {
        let root = Path::new("/nonexistent/shop");
        let project = ComposeProject {
            name: "shop".into(),
            services: vec!["web".into()],
        };
        let containers = vec![
            container("shop-web-1", "shop", "web", "/nonexistent/shop", State::Running),
            container("loose", "", "", "", State::Running),
            container("blog-db-1", "blog", "db", "/elsewhere/blog", State::Running),
            container("api-web-1", "api", "web", "/elsewhere/api", State::Exited),
            container("blog-app-1", "blog", "app", "/elsewhere/blog", State::Running),
        ];
        let summary = |all| -> Vec<(GroupKind, Vec<String>)> {
            groups(root, Some(&project), &containers, all)
                .into_iter()
                .map(|g| (g.kind, g.rows.into_iter().map(|r| r.title).collect()))
                .collect()
        };
        assert_eq!(summary(false), [(GroupKind::Project, vec!["web".to_string()])]);
        assert_eq!(
            summary(true),
            [
                (GroupKind::Project, vec!["web".to_string()]),
                (GroupKind::Compose("api".into()), vec!["web".to_string()]),
                (
                    GroupKind::Compose("blog".into()),
                    vec!["app".to_string(), "db".to_string()]
                ),
                (GroupKind::Standalone, vec!["loose".to_string()]),
            ]
        );
    }
}
