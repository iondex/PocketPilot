use crate::slack::{
    Error, Result,
    api::{Api, Platform},
    config::Config,
    model::*,
    storage::{Binding, Store},
};
use crate::{Context, Integration, IntegrationError};
use engine::reload::Snapshot;
use i18n::LocalizedError;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, task::JoinSet};

#[path = "commands.rs"]
mod commands;

#[path = "menus.rs"]
mod menus;

#[path = "input.rs"]
mod input;

#[path = "observation.rs"]
mod observation;

pub struct Slack {
    settings: Config,
}

impl Slack {
    pub fn new(settings: Config) -> Self {
        Self { settings }
    }
}

#[async_trait::async_trait]
impl Integration for Slack {
    fn platform(&self) -> crate::Platform {
        crate::Platform::Slack
    }

    fn capabilities(&self) -> crate::rendering::Capabilities {
        crate::rendering::Capabilities::for_platform(self.platform())
    }

    async fn run(
        &self,
        context: Context,
        mut stop: watch::Receiver<bool>,
    ) -> std::result::Result<(), IntegrationError> {
        async {
            let (api, bot) = Api::connect(&self.settings).await?;
            let store = Store::new(
                context.store.clone(),
                &self.settings.app_id,
                &self.settings.team_id,
                &bot,
            );
            store.recover().await?;
            let (shutdown, receiver) = watch::channel(false);
            let service = Arc::new(Service {
                settings: self.settings.clone(),
                context,
                store,
                direct_channel: api.direct_channel.clone(),
                api: api.clone(),
                bot,
                stop: receiver.clone(),
                cutoff: AtomicI64::new(engine::now()),
                verified: AtomicBool::new(false),
            });
            let mut tasks = JoinSet::new();
            tasks.spawn(crate::slack::socket::run(service.clone(), api, receiver));
            tasks.spawn(service.clone().work());
            tasks.spawn(service.clone().observe());
            tasks.spawn(service.deliver());
            let result = tokio::select! {
                _=stop.changed()=>Ok(()),
                next=tasks.join_next()=>match next { Some(Ok(r))=>r, _=>Err(Error::Request) },
            };
            let _ = shutdown.send(true);
            let _ = tokio::time::timeout(Duration::from_secs(4), async {
                while tasks.join_next().await.is_some() {}
            })
            .await;
            tasks.abort_all();
            result
        }
        .await
        .map_err(|e: Error| IntegrationError {
            platform: "slack",
            message: e.message(),
        })
    }
}

pub struct Service {
    pub(crate) settings: Config,
    pub(crate) context: Context,
    pub(crate) store: Store,
    pub(crate) api: Arc<dyn Platform>,
    pub(crate) bot: String,
    pub(crate) direct_channel: String,
    pub(crate) stop: watch::Receiver<bool>,
    pub(crate) cutoff: AtomicI64,
    pub(crate) verified: AtomicBool,
}

impl Service {
    pub(crate) fn locale(&self) -> String {
        self.context.config.snapshot().config.language.clone()
    }

    pub(crate) async fn connected(&self) -> Result<()> {
        let mut tx = self.store.core.pool().begin().await?;
        sqlx::query("UPDATE slack_actions SET consumed=1 WHERE namespace=? AND expires<=?")
            .bind(&self.store.namespace)
            .bind(engine::now() + 300)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE slack_inbox SET state='draft' WHERE namespace=? AND state='awaiting_confirmation'")
            .bind(&self.store.namespace).execute(&mut *tx).await?;
        tx.commit().await?;
        self.cutoff.store(engine::now() + 1, Ordering::SeqCst);
        Ok(())
    }

    pub(crate) fn fresh(&self, r: &Request, hash: &str) -> Result<Arc<Snapshot>> {
        let snapshot = self.context.config.snapshot();
        if !self.verified.load(Ordering::SeqCst)
            || *self.stop.borrow()
            || snapshot.hash != hash
            || !self.settings.permits(&self.settings.team_id, &r.user)
            || r.channel != self.direct_channel
        {
            return Err(Error::Ui("slack.error.scope"));
        }
        Ok(snapshot)
    }

    pub(crate) fn workspace<'a>(
        &self,
        snapshot: &'a Snapshot,
        id: &str,
    ) -> Result<&'a engine::config::Workspace> {
        engine::navigation::Navigator::new(&snapshot.config)
            .parent(id)
            .map(|selection| selection.project)
            .map_err(|_| Error::Ui("error.navigation.selection"))
    }

    pub(crate) fn validate<'a>(
        &self,
        snapshot: &'a Snapshot,
        b: &Binding,
    ) -> Result<&'a engine::config::Workspace> {
        if b.owner != self.settings.owner_user_id || b.channel != self.direct_channel {
            return Err(Error::Ui("slack.error.scope"));
        }
        let ws = self.workspace(snapshot, &b.workspace)?;
        let provider = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == ws.provider_id)
            .ok_or(Error::Ui("slack.error.scope"))?;
        let host = snapshot
            .config
            .hosts
            .iter()
            .find(|h| h.id == provider.host_id)
            .ok_or(Error::Ui("slack.error.scope"))?;
        if b.instance != ws.provider_id
            || engine::runtimes::require_workspace(&b.cwd, ws).is_err()
            || b.endpoint != engine::runtimes::fingerprint(host, provider)
        {
            return Err(Error::Ui("slack.error.scope"));
        }
        Ok(ws)
    }

    pub(crate) async fn bound(&self, r: &Request) -> Result<Binding> {
        let id = r
            .binding
            .as_deref()
            .ok_or(Error::Ui("slack.error.unbound"))?;
        let b = self
            .store
            .binding(id)
            .await?
            .ok_or(Error::Ui("slack.error.unbound"))?;
        if Some(b.revision) != r.revision
            || r.root.as_deref() != Some(&b.root)
            || b.channel != r.channel
            || b.owner != r.user
        {
            return Err(Error::Ui("slack.error.scope"));
        }
        Ok(b)
    }

    pub(crate) async fn intake(&self, mut r: Request, source: &str) -> Result<()> {
        let snapshot = self.context.config.snapshot();
        self.fresh(&r, &snapshot.hash)?;
        if let Command::Input { text } = &r.command {
            if text.trim().is_empty() || text.len() > 64 * 1024 {
                return Ok(());
            }
            let Some(root) = &r.root else {
                return Ok(());
            };
            let Some(b) = self.store.at(&r.channel, root).await? else {
                return Ok(());
            };
            self.validate(&snapshot, &b)?;
            r.binding = Some(b.id);
            r.revision = Some(b.revision);
        }
        let draft = r.command.writes()
            && (r.date < self.cutoff.load(Ordering::SeqCst)
                || !(0..=300).contains(&engine::now().saturating_sub(r.date)));
        self.store
            .enqueue(source, &r, &snapshot.hash, draft)
            .await?;
        Ok(())
    }

    pub(crate) async fn click(
        &self,
        nonce: &str,
        channel: Option<&str>,
        user: &str,
        title: Option<&str>,
        source: InteractionSource<'_>,
    ) -> Result<Option<(Request, Action)>> {
        let snapshot = self.context.config.snapshot();
        let result = self
            .store
            .consume(nonce, channel, user, &snapshot.hash, title, source)
            .await?;
        if let Some((r, _)) = &result {
            self.fresh(r, &snapshot.hash)?;
            if r.binding.is_some() {
                self.bound(r).await?;
            }
        }
        Ok(result)
    }

    pub(crate) async fn work(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.clone();
        loop {
            if !self.verified.load(Ordering::SeqCst) {
                tokio::select! { _=stop.changed()=>return Ok(()), _=tokio::time::sleep(Duration::from_millis(100))=>{} }
                continue;
            }
            self.notify_drafts().await?;
            if let Some(row) = self.store.pending().await? {
                let request: Request = serde_json::from_str(&row.request)?;
                if request.command.writes()
                    && (request.date < self.cutoff.load(Ordering::SeqCst)
                        || !(0..=300).contains(&engine::now().saturating_sub(request.date)))
                {
                    self.store.state(&row.id, "pending", "draft").await?;
                    continue;
                }
                if self.store.state(&row.id, "pending", "processing").await? {
                    let r = request;
                    let result = self.command(&r, &row.config_hash).await;
                    if let Err(error) = result {
                        tracing::warn!(request_id=%row.id,"Slack command failed; input is not replayed");
                        let content =
                            crate::slack::render::message(error.localized(&self.locale()));
                        self.store.output(&r, &content, &row.config_hash).await?;
                    }
                    self.store.state(&row.id, "processing", "done").await?;
                }
            }
            tokio::select! { _=stop.changed()=>return Ok(()), _=tokio::time::sleep(Duration::from_millis(250))=>{} }
        }
    }

    async fn notify_drafts(&self) -> Result<()> {
        for row in self.store.drafts().await? {
            let generation = self.cutoff.load(Ordering::SeqCst);
            let r: Request = serde_json::from_str(&row.request)?;
            let snapshot = self.context.config.snapshot();
            let input = matches!(r.command, Command::Input { .. });
            let mut reply = r.clone();
            reply.id = format!("draft-notice:{}:{}", r.id, uuid::Uuid::new_v4());
            let mut content = crate::slack::render::message(i18n::text(
                &self.locale(),
                if row.state == "uncertain" {
                    "slack.input.uncertain"
                } else {
                    "slack.input.draft"
                },
            ));
            if row.state == "draft"
                && input
                && snapshot.hash == row.config_hash
                && self.bound(&r).await.is_ok()
            {
                let nonce = self
                    .store
                    .action(
                        &r,
                        Action::Run(Command::Confirm {
                            inbox: row.id.clone(),
                        }),
                        &snapshot.hash,
                    )
                    .await?;
                content.blocks.push(crate::slack::render::button(
                    &i18n::text(&self.locale(), "slack.confirm"),
                    &nonce,
                ));
                self.store.output(&reply, &content, &snapshot.hash).await?;
                self.store
                    .state(&row.id, "draft", "awaiting_confirmation")
                    .await?;
                if generation != self.cutoff.load(Ordering::SeqCst)
                    || !self.verified.load(Ordering::SeqCst)
                {
                    self.store
                        .state(&row.id, "awaiting_confirmation", "draft")
                        .await?;
                }
            } else {
                self.store.output(&reply, &content, &snapshot.hash).await?;
                self.store.state(&row.id, &row.state, "done").await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
impl Service {
    pub fn local_fixture(
        settings: Config,
        context: Context,
        api: Arc<dyn Platform>,
        bot: String,
        direct_channel: String,
        stop: watch::Receiver<bool>,
    ) -> Self {
        let store = Store::new(
            context.store.clone(),
            &settings.app_id,
            &settings.team_id,
            &bot,
        );
        Self {
            settings,
            context,
            api,
            bot,
            direct_channel,
            store,
            stop,
            cutoff: AtomicI64::new(engine::now() - 1),
            verified: AtomicBool::new(true),
        }
    }

    pub async fn handle_fixture(&self, r: &Request) -> Result<()> {
        self.command(r, &self.context.config.snapshot().hash).await
    }
}
