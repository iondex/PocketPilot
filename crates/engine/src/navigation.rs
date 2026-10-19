//! Shared workspace → project directory → session selection.
//! A UI workspace is a configured Host; the persisted Workspace type is a project.
use crate::config::{Config, Host, Workspace};

#[derive(Debug, thiserror::Error)]
#[error("navigation selection is missing, ambiguous or outside the configured workspace")]
pub struct Error;

impl i18n::LocalizedError for Error {
    fn message(&self) -> i18n::Message {
        i18n::Message::new("error.navigation.selection")
    }
}

pub struct WorkspaceView<'a> {
    pub host: &'a Host,
    pub projects: Vec<&'a Workspace>,
}

pub struct ProjectSelection<'a> {
    pub host: &'a Host,
    pub project: &'a Workspace,
}

pub struct Navigator<'a> {
    config: &'a Config,
}

impl<'a> Navigator<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub fn workspaces(&self) -> Vec<WorkspaceView<'a>> {
        let mut views = self
            .config
            .hosts
            .iter()
            .map(|host| self.view(host))
            .collect::<Vec<_>>();
        views.sort_by(|a, b| (&a.host.name, &a.host.id).cmp(&(&b.host.name, &b.host.id)));
        views
    }

    pub fn workspace(&self, key: &str) -> Result<WorkspaceView<'a>, Error> {
        // Stable IDs win even if a display name happens to equal another ID.
        if let Some(host) = self.config.hosts.iter().find(|h| h.id == key) {
            return Ok(self.view(host));
        }
        let mut matches = self.config.hosts.iter().filter(|h| h.name == key);
        let host = matches.next().ok_or(Error)?;
        if matches.next().is_some() {
            return Err(Error);
        }
        Ok(self.view(host))
    }

    pub fn project(&self, workspace: &str, key: &str) -> Result<ProjectSelection<'a>, Error> {
        let view = self.workspace(workspace)?;
        let mut matches = view
            .projects
            .into_iter()
            .filter(|p| p.id == key || p.short_name == key || p.cwd == key);
        let project = matches.next().ok_or(Error)?;
        if matches.next().is_some() {
            return Err(Error);
        }
        Ok(ProjectSelection {
            host: view.host,
            project,
        })
    }

    pub fn parent(&self, key: &str) -> Result<ProjectSelection<'a>, Error> {
        let mut matches = self
            .config
            .workspaces
            .iter()
            .filter(|p| p.id == key || p.short_name == key);
        let project = matches.next().ok_or(Error)?;
        if matches.next().is_some() {
            return Err(Error);
        }
        let provider = self
            .config
            .providers
            .iter()
            .find(|p| p.id == project.provider_id)
            .ok_or(Error)?;
        self.project(&provider.host_id, &project.id)
    }

    fn view(&self, host: &'a Host) -> WorkspaceView<'a> {
        let mut projects = self
            .config
            .workspaces
            .iter()
            .filter(|ws| {
                self.config
                    .providers
                    .iter()
                    .any(|p| p.id == ws.provider_id && p.host_id == host.id)
            })
            .collect::<Vec<_>>();
        projects.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        WorkspaceView { host, projects }
    }
}

pub struct Page<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub offset: usize,
    pub previous: Option<usize>,
    pub next: Option<usize>,
}

pub fn page<T>(items: Vec<T>, offset: usize, size: usize) -> Page<T> {
    let size = size.clamp(1, 100);
    let total = items.len();
    let offset = if offset < total { offset } else { 0 };
    Page {
        items: items.into_iter().skip(offset).take(size).collect(),
        total,
        offset,
        previous: (offset > 0).then(|| offset.saturating_sub(size)),
        next: (offset.saturating_add(size) < total).then(|| offset + size),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Config {
        let mut config: Config = toml::from_str("[[hosts]]\nid='a'\nname='Alpha'\ntransport='local_proxy'\n[[hosts]]\nid='b'\nname='Beta'\ntransport='local_proxy'").unwrap();
        for (id, host) in [("pa", "a"), ("pb", "b")] {
            config.providers.push(crate::model::Provider {
                id: id.into(),
                host_id: host.into(),
                kind: "codex".into(),
                connection_mode: "existing_proxy".into(),
                compatibility_profile: "fixture".into(),
                control_verified: false,
                endpoint: None,
            });
        }
        for (id, provider, name) in [
            ("a2", "pa", "Zeta"),
            ("b1", "pb", "Remote"),
            ("a1", "pa", "Alpha"),
        ] {
            config.workspaces.push(Workspace {
                id: id.into(),
                provider_id: provider.into(),
                name: name.into(),
                short_name: id.into(),
                cwd: format!("/projects/{id}"),
                additional_cwds: Vec::new(),
                updated_at: None,
            });
        }
        config
    }

    #[test]
    fn both_adapters_see_the_same_order_and_cannot_cross_workspaces() {
        let config = catalog();
        let navigation = Navigator::new(&config);
        assert_eq!(
            navigation
                .workspaces()
                .iter()
                .map(|v| v.host.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(
            navigation
                .workspace("Alpha")
                .unwrap()
                .projects
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "a2"]
        );
        assert!(navigation.project("a", "b1").is_err());
        assert_eq!(
            navigation
                .project("Beta", "/projects/b1")
                .unwrap()
                .project
                .id,
            "b1"
        );
        assert_eq!(navigation.parent("b1").unwrap().host.id, "b");
    }

    #[test]
    fn removed_or_ambiguous_selections_do_not_fall_through_to_another_host() {
        let mut config = catalog();
        config.hosts[1].name = "Alpha".into();
        assert!(Navigator::new(&config).workspace("Alpha").is_err());
        assert!(Navigator::new(&config).workspace("a").is_ok());
        config.hosts.retain(|host| host.id != "b");
        assert!(Navigator::new(&config).parent("b1").is_err());
    }

    #[test]
    fn paging_is_bounded_and_shrinking_catalogs_return_to_the_first_page() {
        let middle = page((0..25).collect(), 8, 8);
        assert_eq!(middle.items, (8..16).collect::<Vec<_>>());
        assert_eq!((middle.previous, middle.next), (Some(0), Some(16)));
        let shrunk = page(vec![1, 2], 80, usize::MAX);
        assert_eq!(shrunk.offset, 0);
        assert_eq!(shrunk.items, [1, 2]);
        assert_eq!(page(Vec::<u8>::new(), usize::MAX, 0).total, 0);
    }
}
