//! Refresh local catalog state and discover newly allowed hosts in the background.
use engine::{catalog::Catalog, integrations::Context};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::{sync::watch, task::JoinSet, time::Instant};

pub async fn watch(context: Context, mut stop: watch::Receiver<bool>) {
    let catalog = Catalog::new(context.store.pool().clone());
    let mut next = HashMap::<String, Instant>::new();
    let mut busy = HashSet::<String>::new();
    let mut jobs = JoinSet::<(
        String,
        Result<Vec<codex::discovery::Endpoint>, codex::Error>,
    )>::new();
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            Some(result)=jobs.join_next(),if !jobs.is_empty()=> {
                match result {
                    Ok((host_id,endpoints))=> {
                        busy.remove(&host_id);
                        if context.config.snapshot().config.hosts.iter().all(|h|h.id!=host_id) { continue; }
                        match endpoints {
                            Ok(endpoints)=>for endpoint in endpoints { if let Err(error)=catalog.discover(&host_id,endpoint).await { tracing::warn!(%error,%host_id,"provider catalog update failed"); } },
                            Err(error)=>tracing::warn!(%error,%host_id,"endpoint discovery unavailable; no server started"),
                        }
                    }
                    Err(error)=>tracing::warn!(%error,"endpoint discovery worker stopped"),
                }
            },
            _=interval.tick()=> {
                let mut config=context.config.snapshot().config.clone();
                match catalog.load(&mut config).await {
                    Ok(())=>if let Err(error)=context.config.publish_catalog(config.clone()) { tracing::warn!(%error,"catalog snapshot rejected"); },
                    Err(error)=>{tracing::warn!(%error,"catalog unavailable; keeping last valid snapshot");continue;},
                }
                for host in config.hosts.clone() {
                    if jobs.len()>=4 { break; }
                    if config.providers.iter().any(|p|p.host_id==host.id) || busy.contains(&host.id) || next.get(&host.id).is_some_and(|due|*due>Instant::now()) {continue;}
                    next.insert(host.id.clone(),Instant::now()+Duration::from_secs(300));
                    busy.insert(host.id.clone());
                    let config=config.clone();
                    jobs.spawn(async move { let result=engine::connections::discover(&host,&config).await; (host.id,result) });
                }
            }
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
}
