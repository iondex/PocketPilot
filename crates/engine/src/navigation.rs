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
#[path = "navigation_tests.rs"]
mod tests;
