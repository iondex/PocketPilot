//! Shared binding inventory and observation controls. Platform repositories retain scoped SQL.
use crate::navigation::{self, Navigator};
use engine::{
    config::Config,
    runtimes,
    storage::{Store, StoreError},
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, Transaction};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Scope {
    Telegram {
        bot: i64,
        chat: i64,
    },
    Discord {
        bot: u64,
        guild: u64,
        owner: u64,
    },
    Slack {
        app: String,
        team: String,
        bot: String,
        channel: String,
        owner: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Telegram { topic: i32 },
    Discord { channel: String },
    Slack { id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub binding: i64,
    pub observation: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    Pause,
    Resume,
    Detach,
}

/// Telegram's historical bindings pin the provider via the core instance ledger.
/// Slack and Discord additionally record the original directory and endpoint.
pub struct Pin {
    pub cwd: String,
    pub endpoint: String,
}

pub struct Record {
    pub target: Target,
    pub reference: session::SessionRef,
    pub project: String,
    pub pin: Option<Pin>,
    pub active: bool,
    pub revision: i64,
    pub title: Option<String>,
    pub root: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Watching,
    Paused,
    Detached,
    Invalid,
}

impl State {
    pub fn text(self, locale: &str) -> String {
        i18n::text(
            locale,
            match self {
                Self::Watching => "bindings.watching",
                Self::Paused => "bindings.paused",
                Self::Detached => "bindings.detached",
                Self::Invalid => "bindings.invalid",
            },
        )
    }
}

/// Binding decisions are shared; native action encoding and widget layout stay local.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ConfirmDetach { version: Version },
    Change { version: Version, change: Change },
}

pub struct Control {
    pub label: &'static str,
    pub action: Action,
}

pub struct Entry {
    pub record: Record,
    pub version: Version,
    pub state: State,
    pub title: String,
    pub workspace: String,
    pub project: String,
    pub runtime_status: Option<session::RuntimeStatus>,
    pub checked_at: Option<i64>,
}

impl Entry {
    /// Every platform maps these decisions to its own persisted controls.
    pub fn controls(&self, confirmation: bool) -> Vec<Control> {
        if !self.record.active {
            return Vec::new();
        }
        if confirmation {
            return vec![Control {
                label: "bindings.confirm_button",
                action: Action::Change {
                    version: self.version,
                    change: Change::Detach,
                },
            }];
        }
        let mut controls = Vec::new();
        let observation = match self.state {
            State::Watching => Some(("bindings.pause", Change::Pause)),
            State::Paused => Some(("bindings.resume", Change::Resume)),
            _ => None,
        };
        if let Some((label, change)) = observation {
            controls.push(Control {
                label,
                action: Action::Change {
                    version: self.version,
                    change,
                },
            });
        }
        controls.push(Control {
            label: "bindings.detach",
            action: Action::ConfirmDetach {
                version: self.version,
            },
        });
        controls
    }

    pub fn description(&self, locale: &str) -> String {
        format!(
            "{} › {} · {}",
            self.workspace,
            self.project,
            self.state.text(locale)
        )
    }

    pub fn jump_url(&self, scope: &Scope) -> Option<String> {
        match (scope, &self.record.target) {
            (Scope::Discord { guild, .. }, Target::Discord { channel }) => {
                Some(format!("https://discord.com/channels/{guild}/{channel}"))
            }
            (Scope::Slack { team, channel, .. }, Target::Slack { .. }) => {
                self.record.root.as_ref().map(|root| {
                    format!("https://app.slack.com/client/{team}/{channel}/thread/{channel}-{root}")
                })
            }
            _ => None,
        }
    }

    pub fn details(&self, locale: &str) -> String {
        let mut text = self.description(locale);
        if let Some(status) = &self.runtime_status {
            text.push_str(&format!(
                "\n{}",
                i18n::Message::new("bindings.runtime")
                    .arg("status", status.text_in(locale))
                    .render(locale)
            ));
        }
        if let Some(checked_at) = self.checked_at {
            text.push_str(&format!(
                "\n{}",
                i18n::Message::new("bindings.checked")
                    .arg("seconds", engine::now().saturating_sub(checked_at).max(0))
                    .render(locale)
            ));
        }
        text
    }
}

#[async_trait::async_trait]
pub trait Repository: Send + Sync {
    fn core(&self) -> &Store;

    async fn records(&self, scope: &Scope) -> Result<Vec<Record>, StoreError>;

    /// The scope and binding revision are checked under the caller's transaction.
    /// Pause/resume retain the binding revision, so existing task input stays valid.
    async fn claim(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        scope: &Scope,
        target: &Target,
        revision: i64,
        change: Change,
    ) -> Result<bool, StoreError>;
}

pub struct Manager<'a> {
    repository: &'a dyn Repository,
    scope: Scope,
}

impl<'a> Manager<'a> {
    pub fn new(repository: &'a dyn Repository, scope: Scope) -> Self {
        Self { repository, scope }
    }

    pub async fn page(
        &self,
        config: &Config,
        offset: usize,
        size: usize,
        locale: &str,
    ) -> Result<navigation::Page<Entry>, StoreError> {
        let mut records = self.repository.records(&self.scope).await?;
        records.sort_by(|a, b| {
            (&a.project, &a.reference.instance, &a.reference.thread).cmp(&(
                &b.project,
                &b.reference.instance,
                &b.reference.thread,
            ))
        });
        let page = navigation::page(records, offset, size);
        let mut entries = Vec::new();
        for record in page.items {
            entries.push(self.describe(config, record, locale).await?);
        }
        Ok(navigation::Page {
            items: entries,
            total: page.total,
            offset: page.offset,
            previous: page.previous,
            next: page.next,
        })
    }

    pub async fn entry(
        &self,
        config: &Config,
        target: &Target,
        locale: &str,
    ) -> Result<Entry, StoreError> {
        let record = self
            .repository
            .records(&self.scope)
            .await?
            .into_iter()
            .find(|record| &record.target == target)
            .ok_or(StoreError::ContainerConflict)?;
        self.describe(config, record, locale).await
    }

    /// A confirmation must describe the same revision the user selected.
    pub async fn confirmation(
        &self,
        config: &Config,
        target: &Target,
        version: Option<Version>,
        locale: &str,
    ) -> Result<Entry, StoreError> {
        let entry = self.entry(config, target, locale).await?;
        if !entry.record.active || version.is_some_and(|version| version != entry.version) {
            return Err(StoreError::Transition);
        }
        Ok(entry)
    }

    async fn describe(
        &self,
        config: &Config,
        record: Record,
        locale: &str,
    ) -> Result<Entry, StoreError> {
        let mut selected = Navigator::new(config)
            .parent(&record.project)
            .ok()
            .filter(|selected| {
                selected.project.provider_id == record.reference.instance
                    && record.pin.as_ref().is_none_or(|pin| {
                        runtimes::require_workspace(&pin.cwd, selected.project).is_ok()
                            && config
                                .providers
                                .iter()
                                .find(|p| p.id == record.reference.instance)
                                .is_some_and(|p| {
                                    runtimes::fingerprint(selected.host, p) == pin.endpoint
                                })
                    })
            });
        if let Some(selection) = &selected {
            let provider = config
                .providers
                .iter()
                .find(|provider| provider.id == record.reference.instance)
                .ok_or(StoreError::InstanceConflict)?;
            let hash = runtimes::fingerprint(selection.host, provider);
            match self
                .repository
                .core()
                .check_instance(&record.reference.instance, &hash, false)
                .await
            {
                Ok(()) => {}
                Err(StoreError::InstanceConflict) => selected = None,
                Err(error) => return Err(error),
            }
        }
        let workspace = selected
            .as_ref()
            .map(|s| s.host.name.clone())
            .unwrap_or_else(|| i18n::text(locale, "bindings.unavailable"));
        let project = selected
            .as_ref()
            .map(|s| s.project.name.clone())
            .unwrap_or_else(|| i18n::text(locale, "bindings.unavailable"));
        let preference = preference(self.repository.core(), &self.scope, &record.target).await?;
        let cache = sqlx::query(
            "SELECT payload,checked_at FROM integration_session_cache WHERE instance=? AND thread=?",
        )
        .bind(&record.reference.instance)
        .bind(&record.reference.thread)
        .fetch_optional(self.repository.core().pool())
        .await
        .map_err(|_| StoreError::Database)?;
        let cached: Option<session::Session> = cache
            .as_ref()
            .map(|row| serde_json::from_str(row.get("payload")))
            .transpose()
            .map_err(|_| StoreError::Database)?;
        let title = record
            .title
            .as_ref()
            .filter(|title| !title.trim().is_empty())
            .or_else(|| {
                cached
                    .as_ref()
                    .map(|session| &session.title)
                    .filter(|title| !title.trim().is_empty())
            })
            .cloned()
            .unwrap_or_else(|| i18n::text(locale, "bindings.untitled"));
        let version = Version {
            binding: record.revision,
            observation: preference.revision,
        };
        let state = if !record.active {
            State::Detached
        } else if selected.is_none() {
            State::Invalid
        } else if preference.paused {
            State::Paused
        } else {
            State::Watching
        };
        Ok(Entry {
            record,
            version,
            state,
            title,
            workspace,
            project,
            runtime_status: cached.map(|s| s.status),
            checked_at: cache.map(|row| row.get("checked_at")),
        })
    }

    /// Only explicit, freshly authorized UI actions call this method. No native runtime I/O.
    pub async fn change(
        &self,
        target: &Target,
        version: Version,
        change: Change,
    ) -> Result<(), StoreError> {
        let key = preference_key(&self.scope, target)?;
        let mut transaction = self
            .repository
            .core()
            .pool()
            .begin()
            .await
            .map_err(|_| StoreError::Database)?;
        if !self
            .repository
            .claim(
                &mut transaction,
                &self.scope,
                target,
                version.binding,
                change,
            )
            .await?
        {
            return Err(StoreError::Transition);
        }
        let current: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM integration_binding_preferences WHERE key=?")
                .bind(&key)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| StoreError::Database)?;
        if current.unwrap_or(0) != version.observation {
            return Err(StoreError::Transition);
        }
        if matches!(change, Change::Detach) {
            sqlx::query("DELETE FROM integration_binding_preferences WHERE key=?")
                .bind(&key)
                .execute(&mut *transaction)
                .await
                .map_err(|_| StoreError::Database)?;
        } else {
            sqlx::query(
                "INSERT INTO integration_binding_preferences(key,paused,revision) VALUES(?,?,?) ON CONFLICT(key)
                DO UPDATE SET paused=excluded.paused,revision=excluded.revision",
            )
            .bind(&key)
            .bind(matches!(change, Change::Pause))
            .bind(version.observation.saturating_add(1))
            .execute(&mut *transaction)
            .await
            .map_err(|_| StoreError::Database)?;
        }
        transaction.commit().await.map_err(|_| StoreError::Database)
    }
}

fn preference_key(scope: &Scope, target: &Target) -> Result<String, StoreError> {
    serde_json::to_string(&(scope, target))
        .map(|value| engine::digest(&value))
        .map_err(|_| StoreError::Database)
}

#[derive(Default)]
struct Observation {
    paused: bool,
    revision: i64,
}

async fn preference(
    store: &Store,
    scope: &Scope,
    target: &Target,
) -> Result<Observation, StoreError> {
    let row =
        sqlx::query("SELECT paused,revision FROM integration_binding_preferences WHERE key=?")
            .bind(preference_key(scope, target)?)
            .fetch_optional(store.pool())
            .await
            .map_err(|_| StoreError::Database)?;
    Ok(row.map_or_else(Observation::default, |row| Observation {
        paused: row.get("paused"),
        revision: row.get("revision"),
    }))
}

pub async fn observing(store: &Store, scope: &Scope, target: &Target) -> Result<bool, StoreError> {
    preference(store, scope, target)
        .await
        .map(|preference| !preference.paused)
}

/// Cache metadata on existing read paths, so inventory pages do not connect to every runtime.
pub async fn remember(store: &Store, session: &session::Session) -> Result<(), StoreError> {
    let payload = serde_json::to_string(session).map_err(|_| StoreError::Database)?;
    sqlx::query(
        "INSERT INTO integration_session_cache(instance,thread,payload,checked_at) VALUES(?,?,?,?) ON
        CONFLICT(instance,thread) DO UPDATE SET payload=excluded.payload,checked_at=excluded.checked_at",
    )
    .bind(&session.reference.instance)
    .bind(&session.reference.thread)
    .bind(payload)
    .bind(engine::now())
    .execute(store.pool())
    .await
    .map_err(|_| StoreError::Database)?;
    Ok(())
}

#[cfg(test)]
#[path = "bindings_tests.rs"]
mod tests;
