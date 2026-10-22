//! Route Telegram menus, callbacks and messages using the current authorized snapshot.

use super::{Action, Error, Service};
use crate::telegram::api::Button;
use crate::telegram::api::Input;
use crate::telegram::ledger::Binding;
use crate::telegram::ledger::Outbound;
use crate::telegram::ledger::Scope;
use engine::config::Config;
use engine::config::Workspace;
use engine::now;
use engine::reload::Snapshot;
use engine::runtimes;
use engine::storage::StoreError;

impl Service {
    pub(super) fn binding_scope(&self, chat: i64) -> crate::bindings::Scope {
        crate::bindings::Scope::Telegram {
            bot: self.bot,
            chat,
        }
    }

    pub(super) fn workspace<'a>(
        &self,
        config: &'a Config,
        id: &str,
    ) -> Result<&'a Workspace, Error> {
        crate::navigation::Navigator::new(config)
            .parent(id)
            .map(|selected| selected.project)
            .map_err(|error| match error {
                crate::navigation::Error::Ambiguous => Error::AmbiguousWorkspace,
                crate::navigation::Error::Selection => runtimes::Error::Selection.into(),
            })
    }

    pub(super) fn binding_workspace<'a>(
        &self,
        config: &'a Config,
        binding: &Binding,
    ) -> Result<&'a Workspace, Error> {
        let workspace = self.workspace(
            config,
            binding.workspace_id.as_deref().ok_or(Error::Unbound)?,
        )?;
        if workspace.provider_id != binding.instance_id {
            return Err(runtimes::Error::Identity.into());
        }
        Ok(workspace)
    }

    pub(super) async fn bound(&self, input: &Input) -> Result<Binding, Error> {
        self.store
            .binding(self.bot, input.chat, input.topic)
            .await?
            .ok_or(Error::Unbound)
    }

    pub(super) async fn button(
        &self,
        input: &Input,
        snapshot: &Snapshot,
        label: &str,
        action: Action,
    ) -> Result<Button, Error> {
        let body = serde_json::to_string(&action).map_err(|_| Error::Expired)?;
        let data = self
            .store
            .callback(self.bot, input, &snapshot.hash, &body, now())
            .await?;
        Ok(Button {
            label: crate::telegram::i18n::tr(input, label),
            data,
        })
    }

    pub(super) async fn respond(
        &self,
        input: &Input,
        key: &str,
        mut body: Outbound,
    ) -> Result<(), Error> {
        if let Some(id) = body.text_id.take() {
            body.text = crate::telegram::i18n::tr(input, &id);
        }
        body.config_hash = input
            .config_hash
            .clone()
            .or_else(|| Some(self.config.snapshot().hash.clone()));
        if let Some(binding) = self
            .store
            .binding(self.bot, input.chat, input.topic)
            .await?
        {
            body.binding_revision = Some(binding.revision);
        }
        self.store
            .enqueue(self.bot, input.chat, input.topic, key, &body)
            .await?;
        Ok(())
    }

    pub(super) async fn authorize(&self, input: &Input, hash: &str) -> Result<(), Error> {
        if self.config.snapshot().hash != hash {
            return Err(Error::Expired);
        }
        if input.callback.is_some()
            && !self
                .store
                .callback_live(self.bot, input, hash, now())
                .await?
        {
            return Err(Error::Expired);
        }
        if !self.store.owner.authorizes(input.user, input.chat, true) {
            return Err(StoreError::Unauthorized.into());
        }
        Ok(())
    }

    pub(super) fn output_scope(
        &self,
        config: &Config,
        workspace: &Workspace,
        cwd: &str,
    ) -> Result<Scope, Error> {
        let provider = config
            .providers
            .iter()
            .find(|p| p.id == workspace.provider_id)
            .ok_or(runtimes::Error::Selection)?;
        let host = config
            .hosts
            .iter()
            .find(|h| h.id == provider.host_id)
            .ok_or(runtimes::Error::Selection)?;
        Ok(Scope {
            workspace: workspace.id.clone(),
            cwd: cwd.into(),
            instance: provider.id.clone(),
            endpoint: runtimes::fingerprint(host, provider),
        })
    }

    pub(super) fn scope_allowed(&self, config: &Config, scope: &Scope) -> bool {
        let Ok(workspace) = self.workspace(config, &scope.workspace) else {
            return false;
        };
        if workspace.provider_id != scope.instance
            || runtimes::require_workspace(&scope.cwd, workspace).is_err()
        {
            return false;
        }
        self.output_scope(config, workspace, &scope.cwd)
            .is_ok_and(|current| current.endpoint == scope.endpoint)
    }
}
