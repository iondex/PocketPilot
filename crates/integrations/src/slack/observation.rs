use super::*;
use crate::slack::render;
use session::{History, Role};
use std::collections::BTreeMap;

pub(super) fn hashes(history: &History) -> Result<String> {
    Ok(serde_json::to_string(
        &history
            .items
            .iter()
            .map(|i| {
                (
                    format!("{}:{}:{}", i.turn_id.len(), i.turn_id, i.item_id),
                    engine::observation::visible_hash(i),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    )?)
}

impl Service {
    pub(super) async fn observe(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.clone();
        let mut jobs = tokio::task::JoinSet::new();
        let mut busy = std::collections::HashSet::new();
        loop {
            for b in self.store.bindings().await? {
                if jobs.len() >= 8 || !busy.insert(b.id.clone()) {
                    continue;
                }
                let service = self.clone();
                jobs.spawn(async move {
                    if service.refresh(&b).await.is_err() {
                        tracing::warn!(binding_id=%b.id,"Slack observation unavailable; session unchanged");
                        let snapshot=service.context.config.snapshot();
                        if service.validate(&snapshot,&b).is_ok() {
                            let text=i18n::text(&service.locale(),"slack.observation_unavailable");
                            let _=service.publish_card(&b,&text,&snapshot.hash,false).await;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    b.id
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
        let snapshot = self.context.config.snapshot();
        let ws = self.validate(&snapshot, b)?;
        let provider = self.context.runtimes.get(&snapshot.config, ws).await?;
        let current = provider.read(&b.thread).await?;
        engine::runtimes::require_workspace(&current.cwd, ws)?;
        let _permit = self
            .context
            .runtimes
            .history_limit(&b.instance)
            .await
            .acquire_owned()
            .await
            .map_err(|_| Error::Request)?;
        let history = provider
            .history(&b.thread)
            .await?
            .without_transient_errors();
        if !history.all_pages_read {
            return Err(Error::Ui("slack.error.history"));
        }
        let r = Request {
            uploads: Vec::new(),
            menu_message: None,
            id: format!("observe:{}", b.id),
            channel: b.channel.clone(),
            user: b.owner.clone(),
            root: Some(b.root.clone()),
            binding: Some(b.id.clone()),
            revision: Some(b.revision),
            date: engine::now(),
            command: Command::Status,
        };
        self.fresh(&r, &snapshot.hash)?;
        self.bound(&r).await?;
        let old: BTreeMap<String, String> = serde_json::from_str(&b.observed)?;
        let observed = hashes(&history)?;
        let text = if b.history_ready {
            history
                .items
                .iter()
                .filter(|i| {
                    !engine::observation::unchanged(
                        i,
                        old.get(&format!("{}:{}:{}", i.turn_id.len(), i.turn_id, i.item_id)),
                    )
                })
                .filter(|i| {
                    !(i.role == Role::User
                        && i.client_id.as_ref().is_some_and(|id| {
                            id.starts_with(&format!("pp:slack:{}:", self.store.namespace))
                        }))
                })
                .map(|i| session::presentation::Section::item(i, &self.locale()))
                .collect::<Vec<_>>()
        } else {
            session::presentation::latest(&history, &self.locale())
        };
        self.store
            .publish(b, &render::conversation(text), &observed, &snapshot.hash)
            .await?;
        let text = i18n::t!(
            "common.session.details",
            locale = &self.locale(),
            title = session::delivery::short_title(&current.title, 80),
            state = render::state(&current.status, &self.locale()),
            workspace = &ws.name,
        );
        self.publish_card(b, &text, &snapshot.hash, false).await
    }

    pub(super) async fn publish_card(
        &self,
        b: &Binding,
        text: &str,
        hash: &str,
        force: bool,
    ) -> Result<()> {
        let cached = format!("{text}\n[card:2]");
        if force || cached != b.card {
            let mut content = self.root_card(b, text, hash).await?;
            content.edit = true;
            let r = Request {
                uploads: Vec::new(),
                menu_message: None,
                id: format!("card:{}:{}", b.id, uuid::Uuid::new_v4()),
                channel: b.channel.clone(),
                user: b.owner.clone(),
                root: Some(b.root.clone()),
                binding: Some(b.id.clone()),
                revision: Some(b.revision),
                date: engine::now(),
                command: Command::Status,
            };
            self.fresh(&r, hash)?;
            self.bound(&r).await?;
            self.store.output(&r, &content, hash).await?;
            self.store.card(b, &cached).await?;
        }
        Ok(())
    }

    pub(super) async fn deliver(self: Arc<Self>) -> Result<()> {
        let mut stop = self.stop.clone();
        loop {
            if !self.verified.load(Ordering::SeqCst) {
                tokio::select! { _=stop.changed()=>return Ok(()), _=tokio::time::sleep(Duration::from_millis(250))=>{} }
                continue;
            }
            for o in self.store.outgoing().await? {
                let r = Request {
                    uploads: Vec::new(),
                    menu_message: None,
                    id: o.id.clone(),
                    channel: o.channel.clone(),
                    user: self.settings.owner_user_id.clone(),
                    root: o.root.clone(),
                    binding: o.binding.clone(),
                    revision: o.revision,
                    date: engine::now(),
                    command: Command::Status,
                };
                let scope_hash = if r.binding.is_some() {
                    self.context.config.snapshot().hash.clone()
                } else {
                    o.config_hash.clone()
                };
                let valid = self.fresh(&r, &scope_hash);
                let valid = if let Ok(snapshot) = valid {
                    if r.binding.is_some() {
                        match self.bound(&r).await {
                            Ok(b) => self.validate(&snapshot, &b).is_ok(),
                            Err(_) => false,
                        }
                    } else {
                        true
                    }
                } else {
                    false
                };
                if !valid {
                    self.store.finish_output(&o.id, "rejected", 0).await?;
                    continue;
                }
                if self.api.channel(&o.channel).await.is_err() {
                    continue;
                }
                if self.fresh(&r, &scope_hash).is_err() {
                    continue;
                }
                if r.binding.is_some() && self.bound(&r).await.is_err() {
                    continue;
                }
                let content: Content = serde_json::from_str(&o.content)?;
                if content.document {
                    self.store.finish_output(&o.id, "rejected", 0).await?;
                    continue;
                }
                let parts: Vec<Content> = if let Some(saved) = &o.rendered_parts {
                    serde_json::from_str(saved)?
                } else {
                    let parts = render::parts(&content)?;
                    self.store.prepare_parts(&o, &parts).await?;
                    parts
                };
                let Some(part) = usize::try_from(o.next_part).ok().and_then(|i| parts.get(i))
                else {
                    self.store.finish_output(&o.id, "rejected", 0).await?;
                    continue;
                };
                if !self.store.claim_output(&o).await? {
                    continue;
                }
                let result = if content.edit {
                    match &o.root {
                        Some(root) => self.api.update(&o.channel, root, part).await,
                        None => Err(Error::Request),
                    }
                } else {
                    self.api
                        .post(&o.channel, o.root.as_deref(), part)
                        .await
                        .map(|_| ())
                };
                match result {
                    Ok(()) => {
                        self.store
                            .acknowledge_part(&o, (o.next_part as usize) + 1 < parts.len())
                            .await?
                    }
                    Err(Error::RateLimit(delay)) => self.store.rate_limit(&o.id, delay).await?,
                    Err(_) if content.edit => {
                        self.store.finish_output(&o.id, "pending", 30).await?
                    }
                    Err(_) => {
                        self.store
                            .finish_output(&o.id, "outcome_unknown", 0)
                            .await?;
                        tracing::warn!(delivery_id=%o.id,"Slack send outcome unknown; not resent");
                    }
                }
            }
            tokio::select! {_=stop.changed()=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(500))=>{}}
        }
    }
}
