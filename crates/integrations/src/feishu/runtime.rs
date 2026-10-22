//! Integration lifecycle and shared authorization. Native UI and SQL remain platform-local.
use super::{
    Error, Result,
    api::{Api, Platform},
    config::{Config, Region},
    model::*,
    render::{Card, CardRenderer},
    storage::Store,
};
use crate::{
    Context, Integration, IntegrationError,
    navigation::Navigator,
    rendering::{RenderRequest, Renderer},
};
use i18n::LocalizedError;
use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

#[path = "menus.rs"]
mod menus;

#[path = "input.rs"]
mod input;

#[path = "observation.rs"]
mod observation;

pub struct Feishu {
    region: Region,
    settings: Config,
}

impl Feishu {
    pub fn new(settings: Config) -> Self {
        Self::for_region(Region::Feishu, settings)
    }

    /// Construct either platform using the shared protocol and runtime lifecycle.
    pub fn for_region(region: Region, settings: Config) -> Self {
        Self { region, settings }
    }
}

#[async_trait::async_trait]
impl Integration for Feishu {
    fn platform(&self) -> crate::Platform {
        self.region.platform()
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
            let api = Arc::new(Api::new(self.region, &self.settings)?);
            let identity = api.identity().await?;
            let store = Store::new(context.store.clone(), self.region, &self.settings);
            // Recovery classifies old intent before any new platform event can be claimed.
            store.recover().await?;
            let (shutdown, receiver) = watch::channel(false);
            let service = Arc::new(Service {
                region: self.region,
                settings: self.settings.clone(),
                context,
                store,
                api: api.clone(),
                bot: identity.open_id,
                cutoff: AtomicI64::new(engine::now()),
                stop: receiver,
            });
            let mut tasks = tokio::task::JoinSet::new();
            tasks.spawn(super::socket::run(service.clone(), api));
            tasks.spawn(service.clone().work());
            tasks.spawn(service.clone().observe());
            tasks.spawn(service.deliver());
            let result = tokio::select! {
                _ = stop.changed() => Ok(()),
                next = tasks.join_next() => match next {
                    Some(Ok(r)) => r,
                    _ => Err(Error::Request),
                },
            };
            // Give bridge workers time to exit; native agent tasks have a separate owner.
            let _ = shutdown.send(true);
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                while tasks.join_next().await.is_some() {}
            })
            .await;
            tasks.abort_all();
            result
        }
        .await
        .map_err(|error: Error| IntegrationError {
            platform: self.region.as_str(),
            message: error.message(),
        })
    }
}

pub(super) struct Service {
    pub region: Region,
    pub settings: Config,
    pub context: Context,
    pub store: Store,
    pub api: Arc<dyn Platform>,
    pub bot: String,
    pub cutoff: AtomicI64,
    pub stop: watch::Receiver<bool>,
}

impl Service {
    pub(super) fn locale(&self) -> String {
        self.context.config.snapshot().config.language.clone()
    }

    pub(super) fn text(&self, id: &'static str) -> String {
        i18n::text(&self.locale(), id)
    }

    pub(super) fn date_valid(&self, date: i64) -> bool {
        date >= self.cutoff.load(Ordering::SeqCst)
            && (0..=300).contains(&engine::now().saturating_sub(date))
    }

    fn fresh(&self, input: &Input, hash: &str) -> Result<Arc<engine::reload::Snapshot>> {
        let snapshot = self.context.config.snapshot();
        if snapshot.hash != hash || !self.date_valid(input.date) {
            return Err(Error::Ui("feishu.expired"));
        }
        Ok(snapshot)
    }

    fn validate<'a>(
        &self,
        snapshot: &'a engine::reload::Snapshot,
        b: &Binding,
    ) -> Result<&'a engine::config::Workspace> {
        if !b.active || b.state != "ready" || !self.settings.group(&b.chat) {
            return Err(Error::Ui("feishu.error.scope"));
        }
        let selected = Navigator::new(&snapshot.config)
            .parent(&b.project)
            .map_err(|_| Error::Ui("feishu.error.scope"))?;
        let provider = snapshot
            .config
            .providers
            .iter()
            .find(|p| p.id == b.instance && p.id == selected.project.provider_id)
            .ok_or(Error::Ui("feishu.error.scope"))?;
        engine::runtimes::require_workspace(&b.cwd, selected.project)?;
        if engine::runtimes::fingerprint(selected.host, provider) != b.endpoint {
            return Err(Error::Ui("feishu.error.scope"));
        }
        Ok(selected.project)
    }

    async fn current_binding(&self, b: &Binding) -> Result<Binding> {
        let current = self.store.binding(&b.id).await?;
        if !current.active
            || current.state != "ready"
            || current.revision != b.revision
            || current.root != b.root
            || current.topic != b.topic
        {
            return Err(Error::Ui("feishu.expired"));
        }
        Ok(current)
    }

    async fn conversation(
        &self,
        destination: &Destination,
        sections: Vec<session::presentation::Section>,
        hash: &str,
        binding: Option<&Binding>,
    ) -> Result<()> {
        let mut cards = Vec::new();
        for section in sections {
            cards.extend(
                CardRenderer
                    .render(RenderRequest {
                        source: &section.body,
                        format: session::markdown::Format::Markdown,
                        heading: Some(&section.heading),
                    })
                    .map_err(|_| Error::Ui("feishu.error.large"))?
                    .parts,
            );
        }
        self.store
            .output(destination, &cards, hash, binding, None, &[])
            .await
    }

    async fn work(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.clone();
        let mut jobs = tokio::task::JoinSet::new();
        loop {
            while jobs.len() < 4 {
                let Some(row) = self.store.next().await? else {
                    break;
                };
                let service = self.clone();
                jobs.spawn(async move {
                    let input: Input = serde_json::from_str(&row.payload)?;
                    let result = service.process(&input, &row.config_hash).await;
                    if let Err(error) = &result {
                        tracing::warn!(
                            platform = service.region.as_str(),
                            input_id = %row.id,
                            "Chat action failed; no automatic replay"
                        );
                        let current = service.context.config.snapshot();
                        if current.hash == row.config_hash && service.date_valid(input.date) {
                            let mut card = Card::new(&service.text("feishu.title"));
                            card.markdown(super::render::literal(
                                &error.localized(&service.locale()),
                            ));
                            let _ = service
                                .store
                                .output(
                                    &Destination {
                                        chat: input.chat.clone(),
                                        root: input.root.clone(),
                                    },
                                    &[card],
                                    &row.config_hash,
                                    None,
                                    None,
                                    &[],
                                )
                                .await;
                        }
                    }
                    service.store.done(&row.id, result.is_err()).await
                });
            }
            tokio::select! {
                _ = stop.changed() => {
                    jobs.abort_all();
                    return Ok(());
                },
                result = jobs.join_next(), if !jobs.is_empty() => match result {
                    Some(Ok(r)) => r?,
                    _ => return Err(Error::Request),
                },
                _ = tokio::time::sleep(Duration::from_millis(200)) => {},
            }
        }
    }

    async fn process(&self, input: &Input, hash: &str) -> Result<()> {
        self.fresh(input, hash)?;
        match &input.command {
            Command::Unsupported => Err(Error::Ui("feishu.error.unsupported")),
            Command::Menu(action) => self.menu(input, hash, action.clone()).await,
            Command::Create { project, title } => self.create(input, hash, project, title).await,
            Command::Text(text) => self.input(input, hash, text).await,
        }
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
