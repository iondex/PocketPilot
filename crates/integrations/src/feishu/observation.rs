//! Read-only observation and frozen delivery have separate durable cursors.
use super::*;
use std::collections::BTreeMap;

impl Service {
    pub(super) async fn observe(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.clone();
        let mut jobs = tokio::task::JoinSet::new();
        let mut busy = std::collections::HashSet::new();
        loop {
            for binding in self.store.ready_bindings().await? {
                if jobs.len() >= 8 || !busy.insert(binding.id.clone()) {
                    continue;
                }
                let service = self.clone();
                jobs.spawn(async move {
                    if service.refresh(&binding).await.is_err() {tracing::warn!(binding_id=%binding.id,"Feishu observation unavailable; native session unchanged");}
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    binding.id
                });
            }
            tokio::select! {
                _=stop.changed()=>{jobs.abort_all();return Ok(());},
                result=jobs.join_next(),if !jobs.is_empty()=>match result {Some(Ok(id))=>{busy.remove(&id);},_=>return Err(Error::Request)},
                _=tokio::time::sleep(Duration::from_millis(500))=>{},
            }
        }
    }

    async fn refresh(&self, b: &Binding) -> Result<()> {
        let scope = super::super::binding_store::scope(&self.settings);
        if !crate::bindings::observing(
            &self.store.core,
            &scope,
            &crate::bindings::Target::Feishu { id: b.id.clone() },
        )
        .await?
        {
            return Ok(());
        }
        let snapshot = self.context.config.snapshot();
        let project = self.validate(&snapshot, b)?;
        let provider = self.context.runtimes.get(&snapshot.config, project).await?;
        let current = provider.read(&b.session).await?;
        engine::runtimes::require_workspace(&current.cwd, project)?;
        crate::bindings::remember(&self.store.core, &current).await?;
        let _permit = self
            .context
            .runtimes
            .history_limit(&b.instance)
            .await
            .acquire_owned()
            .await
            .map_err(|_| Error::Request)?;
        let history = provider
            .history(&b.session)
            .await?
            .without_transient_errors();
        if !history.all_pages_read {
            return Err(Error::Ui("feishu.error.history"));
        }
        let (cursor, sections) = capture(b, &history, &self.store.namespace, &self.locale())?;
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
        self.current_binding(b).await?;
        let status = serde_json::to_string(&(&current.title, &current.status))?;
        let status_card = (status != b.status)
            .then(|| self.status_card(&snapshot, b, &current))
            .transpose()?;
        self.store
            .publish(
                b,
                &cards,
                &cursor,
                &status,
                status_card.as_ref(),
                &snapshot.hash,
            )
            .await
    }

    pub(super) async fn deliver(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.clone();
        loop {
            if let Some(output) = self.store.outgoing().await? {
                let result: Result<Option<String>> = async {
                    let snapshot = self.context.config.snapshot();
                    if snapshot.hash != output.config_hash {
                        return Err(Error::Ui("feishu.expired"));
                    }
                    if let Some(id) = &output.binding {
                        let binding = self.store.binding(id).await?;
                        if Some(binding.revision) != output.revision {
                            return Err(Error::Ui("feishu.expired"));
                        }
                        self.validate(&snapshot, &binding)?;
                        if output.automatic
                            && !crate::bindings::observing(
                                &self.store.core,
                                &super::super::binding_store::scope(&self.settings),
                                &crate::bindings::Target::Feishu { id: id.clone() },
                            )
                            .await?
                        {
                            return Err(Error::Ui("feishu.expired"));
                        }
                    } else if engine::now().saturating_sub(output.created) > 300 {
                        return Err(Error::Ui("feishu.expired"));
                    }
                    let destination: Destination = serde_json::from_str(&output.destination)?;
                    let card: Card = serde_json::from_str(&output.content)?;
                    if let Some(message) = &output.edit {
                        self.api.patch(message, &card).await?;
                        Ok(None)
                    } else {
                        Ok(Some(
                            self.api
                                .send(&destination, &card, &output.id)
                                .await?
                                .message_id,
                        ))
                    }
                }
                .await;
                self.store
                    .delivered(
                        &output,
                        result.as_ref().ok().and_then(|v| v.as_deref()),
                        result.is_err(),
                    )
                    .await?;
                if result.is_err() {
                    tracing::warn!(delivery_id=%output.id,"Feishu delivery unavailable; no automatic replay");
                }
            }
            tokio::select! {_=stop.changed()=>return Ok(()),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
}

pub(super) fn capture(
    binding: &Binding,
    history: &session::History,
    namespace: &str,
    locale: &str,
) -> Result<(String, Vec<session::presentation::Section>)> {
    if !history.all_pages_read {
        return Err(Error::Ui("feishu.error.history"));
    }
    let key = |item: &session::TextItem| {
        format!("{}:{}:{}", item.turn_id.len(), item.turn_id, item.item_id)
    };
    let cursor: BTreeMap<String, String> = history
        .items
        .iter()
        .map(|i| (key(i), engine::observation::visible_hash(i)))
        .collect();
    let sections = if !binding.ready {
        session::presentation::latest(history, locale)
    } else {
        let previous: BTreeMap<String, String> = serde_json::from_str(&binding.cursor)?;
        let own = format!("pp:feishu:{}:", engine::digest(namespace));
        history
            .items
            .iter()
            .filter(|i| !i.is_transient_error())
            .filter(|i| !engine::observation::unchanged(i, previous.get(&key(i))))
            .filter(|i| {
                !(i.role == session::Role::User
                    && i.client_id.as_ref().is_some_and(|id| id.starts_with(&own)))
            })
            .map(|i| session::presentation::Section::item(i, locale))
            .collect()
    };
    Ok((serde_json::to_string(&cursor)?, sections))
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
