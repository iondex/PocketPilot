//! Shared workspace → project directory → session selection.
//! A UI workspace is a configured Host; the persisted Workspace type is a project.
use engine::{
    config::{Config, Host, Workspace},
    runtimes::{self, RuntimeRegistry},
    storage::Store,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("navigation selection is missing or outside the configured workspace")]
    Selection,
    #[error("navigation selection is ambiguous")]
    Ambiguous,
}

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

/// A scoped project page; native controls never need to reconstruct its parent host.
pub struct ProjectPage<'a> {
    pub host: &'a Host,
    pub projects: Page<&'a Workspace>,
}

/// Read-only session navigation shares provider selection and the bounded picker query.
pub struct SessionPage<'a> {
    pub selection: ProjectSelection<'a>,
    pub sessions: runtimes::PickerPage,
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
        let mut matches = self
            .config
            .hosts
            .iter()
            .filter(|h| h.name == key || h.ssh_alias.as_deref() == Some(key));
        let host = matches.next().ok_or(Error::Selection)?;
        if matches.next().is_some() {
            return Err(Error::Ambiguous);
        }
        Ok(self.view(host))
    }

    pub fn project(&self, workspace: &str, key: &str) -> Result<ProjectSelection<'a>, Error> {
        let view = self.workspace(workspace)?;
        if let Some(project) = self.config.workspaces.iter().find(|p| p.id == key) {
            if !view
                .projects
                .iter()
                .any(|candidate| candidate.id == project.id)
            {
                return Err(Error::Selection);
            }
            return Ok(ProjectSelection {
                host: view.host,
                project,
            });
        }
        let mut matches = view
            .projects
            .into_iter()
            .filter(|p| project_matches(p, key));
        let project = matches.next().ok_or(Error::Selection)?;
        if matches.next().is_some() {
            return Err(Error::Ambiguous);
        }
        Ok(ProjectSelection {
            host: view.host,
            project,
        })
    }

    pub fn parent(&self, key: &str) -> Result<ProjectSelection<'a>, Error> {
        if let Some(project) = self.config.workspaces.iter().find(|p| p.id == key) {
            return self.selection(project);
        }
        // Absolute paths are project keys. A relative host/project key is an explicit scope.
        if !key.starts_with('/')
            && let Some((host, project)) = key.split_once('/')
        {
            return self.project(host, project);
        }
        let mut matches = self
            .config
            .workspaces
            .iter()
            .filter(|p| project_matches(p, key));
        let project = matches.next().ok_or(Error::Selection)?;
        if matches.next().is_some() {
            return Err(Error::Ambiguous);
        }
        self.selection(project)
    }

    fn selection(&self, project: &'a Workspace) -> Result<ProjectSelection<'a>, Error> {
        let provider = self
            .config
            .providers
            .iter()
            .find(|p| p.id == project.provider_id)
            .ok_or(Error::Selection)?;
        self.project(&provider.host_id, &project.id)
    }

    pub fn workspaces_page(&self, offset: usize, size: usize) -> Page<WorkspaceView<'a>> {
        page(self.workspaces(), offset, size)
    }

    pub fn projects_page(
        &self,
        workspace: &str,
        offset: usize,
        size: usize,
    ) -> Result<ProjectPage<'a>, Error> {
        let view = self.workspace(workspace)?;
        Ok(ProjectPage {
            host: view.host,
            projects: page(view.projects, offset, size),
        })
    }

    /// Return to the page containing the selected project, using the same ordering and bounds.
    pub fn project_offset(&self, project: &str, size: usize) -> Result<usize, Error> {
        let selected = self.parent(project)?;
        let projects = self.view(selected.host).projects;
        let index = projects
            .iter()
            .position(|project| project.id == selected.project.id)
            .ok_or(Error::Selection)?;
        let size = size.clamp(1, 100);
        Ok(index / size * size)
    }

    /// Scope is resolved before connecting; navigation must not create or resume a session.
    pub async fn sessions_page(
        &self,
        store: &Store,
        runtimes: &RuntimeRegistry,
        project: &str,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<SessionPage<'a>, runtimes::Error> {
        let selection = self
            .parent(project)
            .map_err(|_| runtimes::Error::Selection)?;
        let provider = runtimes.get(self.config, selection.project).await?;
        let sessions = runtimes::list_picker(
            store,
            provider.as_ref(),
            selection.project,
            cursor,
            limit.clamp(1, 100),
        )
        .await?;
        Ok(SessionPage {
            selection,
            sessions,
        })
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
        projects.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.cwd.cmp(&b.cwd))
                .then_with(|| a.id.cmp(&b.id))
        });
        WorkspaceView { host, projects }
    }
}

fn project_matches(project: &Workspace, key: &str) -> bool {
    project.id == key || project.short_name == key || project.name == key || project.cwd == key
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
    let offset = if offset < total {
        offset / size * size
    } else {
        0
    };
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
