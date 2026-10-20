//! Refresh catalog state and discover workspace cwd metadata without agent mutations.
use engine::catalog::{Catalog, WorkspaceDiscovery};
use integrations::Context;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::{sync::watch, task::JoinSet, time::Instant};

enum Discovery {
    Endpoints(
        String,
        Result<Vec<adapters::ServerEndpoint>, adapters::codex::Error>,
    ),
    Workspaces(String, Result<WorkspaceDiscovery, engine::runtimes::Error>),
}

pub async fn watch(context: Context, stop: watch::Receiver<bool>) {
    let catalog = Catalog::new(context.store.pool().clone());
    watch_catalog(context, catalog, stop).await;
}

pub(crate) async fn watch_catalog(
    context: Context,
    catalog: Catalog,
    mut stop: watch::Receiver<bool>,
) {
    let mut next = HashMap::<String, Instant>::new();
    let mut busy = HashSet::<String>::new();
    let mut jobs = JoinSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            Some(result)=jobs.join_next(),if !jobs.is_empty()=> {
                match result {
                    Ok(Discovery::Endpoints(host_id,result))=> {
                        busy.remove(&format!("endpoint:{host_id}"));
                        if context.config.snapshot().config.hosts.iter().all(|h|h.id!=host_id) {continue;}
                        match result {
                            Ok(endpoints)=>for endpoint in endpoints {
                                if let Err(error)=catalog.discover(&host_id,endpoint).await {tracing::warn!(%error,%host_id,"provider catalog update failed");}
                            },
                            Err(error)=>tracing::warn!(%error,%host_id,"endpoint discovery unavailable; no server started"),
                        }
                    },
                    Ok(Discovery::Workspaces(host_id,result))=> {
                        let key=format!("workspaces:{host_id}");
                        busy.remove(&key);
                        if let Err(error)=result {
                            next.insert(key,Instant::now()+Duration::from_secs(30));
                            tracing::warn!(%error,%host_id,"workspace discovery incomplete; keeping registered projects");
                        }
                    },
                    Err(error)=>tracing::warn!(%error,"catalog discovery worker stopped"),
                }
            },
            _=interval.tick()=> {
                let mut config=context.config.snapshot().config.clone();
                match catalog.load(&mut config).await {
                    Ok(())=>if let Err(error)=context.config.publish_catalog(config.clone()) {tracing::warn!(%error,"catalog snapshot rejected");},
                    Err(error)=>{tracing::warn!(%error,"catalog unavailable; keeping last valid snapshot");continue;},
                }
                for host in config.hosts.clone() {
                    if jobs.len()>=4 {break;}
                    let registered=config.providers.iter().any(|p|p.host_id==host.id);
                    let key=format!("{}:{}",if registered {"workspaces"}else{"endpoint"},host.id);
                    if busy.contains(&key) || next.get(&key).is_some_and(|due|*due>Instant::now()) {continue;}
                    next.insert(key.clone(),Instant::now()+Duration::from_secs(300));
                    busy.insert(key);
                    let config=config.clone();
                    if registered {
                        let catalog=catalog.clone();
                        let runtimes=context.runtimes.clone();
                        jobs.spawn(async move {
                            let result=tokio::time::timeout(Duration::from_secs(120),catalog.refresh_host_workspaces(&config,&runtimes,&host.id)).await
                                .unwrap_or_else(|_|Err(session::ProviderError::Timeout.into()));
                            Discovery::Workspaces(host.id,result)
                        });
                    } else {
                        jobs.spawn(async move {let result=engine::runtimes::discover(&host,&config).await;Discovery::Endpoints(host.id,result)});
                    }
                }
            },
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
}
