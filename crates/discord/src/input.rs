use super::{Result, Service, Target, error};
use crate::Api;
use crate::Input;
use engine::reload::Snapshot;
use session::operation::OperationState as State;
use session::provider::Mutation;
use session::provider::Receipt;

impl Service {
    pub(super) async fn mutate(
        &self,
        api: &Api,
        input: &Input,
        snapshot: &Snapshot,
        key: &str,
        target: Target<'_>,
        mutation: Mutation,
    ) -> Result<Receipt> {
        let Target {
            workspace: ws,
            thread,
            binding,
        } = target;
        self.fresh(api, input, snapshot, true).await?;
        let provider = self
            .connections
            .get(&snapshot.config, ws)
            .await
            .map_err(error)?;
        if !provider.capabilities().control_verified {
            return Err("此实例尚未完成共享/断线验证，当前只读。".into());
        }
        let id = self
            .store
            .reserve_operation(
                &format!("discord:{}:{}:{key}", input.bot, input.id),
                &ws.provider_id,
                thread,
            )
            .await
            .map_err(error)?;
        let state = self.store.operation_state(&id).await.map_err(error)?;
        if state == State::Accepted {
            return self
                .store
                .receipt(&id)
                .await
                .map_err(error)?
                .ok_or("回执缺失，未重发。".into());
        }
        if state != State::Received {
            return Err("先前请求结果未知，未重发，请核对原客户端。".into());
        }
        self.store
            .transition(&id, State::Received, State::Prepared)
            .await
            .map_err(error)?;
        self.fresh(api, input, snapshot, true).await?;
        if let Some(b) = binding {
            let current = self.bound(input).await?;
            if current.revision != b.revision
                || current.thread != thread
                || current.instance != ws.provider_id
            {
                return Err("映射已更新，请重新发送。".into());
            }
        }
        self.store
            .transition(&id, State::Prepared, State::Dispatching)
            .await
            .map_err(error)?;
        engine::operations::execute_dispatched(&self.store, &id, provider.as_ref(), mutation)
            .await
            .map_err(error)
    }
}
