use super::*;
use session::{
    Capabilities, History, Page, ProviderError, SessionQuery, SessionRef, provider::Receipt,
    retry::RetryReason,
};
use std::sync::Mutex;

struct Provider {
    listed: Mutex<Vec<Session>>,
    current: Mutex<Session>,
    failure: Mutex<Option<RetryableFailure>>,
    sent: Mutex<Vec<Mutation>>,
    timeout: bool,
}

#[async_trait::async_trait]
impl AgentProvider for Provider {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            control_verified: true,
            ..Default::default()
        }
    }

    async fn list(&self, _: SessionQuery) -> Result<Page<Session>, ProviderError> {
        Ok(Page {
            data: self.listed.lock().unwrap().clone(),
            next_cursor: None,
        })
    }

    async fn read(&self, _: &str) -> Result<Session, ProviderError> {
        Ok(self.current.lock().unwrap().clone())
    }

    async fn history(&self, _: &str) -> Result<History, ProviderError> {
        panic!("retry must not scan conversation bodies")
    }

    async fn retryable_failure(&self, _: &str) -> Result<Option<RetryableFailure>, ProviderError> {
        Ok(self.failure.lock().unwrap().clone())
    }

    async fn execute(&self, mutation: Mutation) -> Result<Receipt, ProviderError> {
        self.sent.lock().unwrap().push(mutation);
        if self.timeout {
            return Err(ProviderError::Timeout);
        }
        Ok(Receipt {
            thread: "thread".into(),
            turn: Some("continued-turn".into()),
        })
    }
}

struct Fixture {
    _clock: ClockGuard,
    _directory: tempfile::TempDir,
    path: std::path::PathBuf,
    config: ConfigFile,
    store: Store,
    provider: Provider,
    session: Session,
}

// SQLite uses a real worker thread. Keep virtual time from automatically jumping
// to SQLx's acquisition timeout while that thread is opening or querying the DB.
struct ClockGuard(tokio::task::JoinHandle<()>);

impl Drop for ClockGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Fixture {
    async fn new() -> Self {
        let clock = ClockGuard(tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        }));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("retry.toml");
        std::fs::write(&path, "[[hosts]]\nid='host'\nname='host'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']\n").unwrap();
        let config = ConfigFile::open(path.clone(), |path, text| {
            let mut config: Config = toml::from_str(text).unwrap();
            config.prepare(path, text)?;
            config.providers.push(crate::config::Provider {
                id: "instance".into(),
                host_id: "host".into(),
                kind: "codex".into(),
                connection_mode: "existing_proxy".into(),
                compatibility_profile: "fixture".into(),
                control_verified: true,
                endpoint: None,
            });
            Ok(crate::reload::ParsedConfig {
                config,
                restart_key: String::new(),
            })
        })
        .unwrap();
        let store = Store::open(&directory.path().join("data")).await.unwrap();
        let session = Session {
            reference: SessionRef {
                instance: "instance".into(),
                thread: "thread".into(),
            },
            title: "fixture".into(),
            native_title: None,
            cwd: "/projects/example".into(),
            status: RuntimeStatus::Idle,
            active_turn: None,
            updated_at: Some(100),
        };
        let provider = Provider {
            listed: Mutex::new(vec![]),
            current: Mutex::new(session.clone()),
            failure: Mutex::new(Some(RetryableFailure {
                turn_id: "failed-turn".into(),
                reason: RetryReason::ConcurrencyLimit,
            })),
            sent: Mutex::new(vec![]),
            timeout: false,
        };
        Self {
            _clock: clock,
            _directory: directory,
            path,
            config,
            store,
            provider,
            session,
        }
    }

    async fn inspect(&self, monitor: &mut Monitor) {
        monitor
            .inspect(
                &self.config,
                &self.store,
                &self.provider,
                &self.session,
                &self.config.snapshot().hash,
            )
            .await
            .unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn discovers_new_sessions_without_bindings_and_tracks_start_and_disappearance() {
    let f = Fixture::new().await;
    let mut monitor = Monitor::default();
    let connections = Connections::new(f.store.clone());
    monitor
        .scan_provider(&f.config, &connections, &f.store, "instance", &f.provider)
        .await
        .unwrap();
    assert!(monitor.pending.is_empty());
    f.provider.listed.lock().unwrap().push(f.session.clone());
    monitor
        .scan_provider(&f.config, &connections, &f.store, "instance", &f.provider)
        .await
        .unwrap();
    assert_eq!(monitor.pending.len(), 1);
    f.provider.listed.lock().unwrap()[0].status = RuntimeStatus::Active {
        waiting_on_approval: false,
        waiting_on_user_input: false,
    };
    monitor
        .scan_provider(&f.config, &connections, &f.store, "instance", &f.provider)
        .await
        .unwrap();
    assert!(monitor.pending.is_empty());
    f.provider.failure.lock().unwrap().as_mut().unwrap().turn_id = "new-failure".into();
    f.provider.listed.lock().unwrap()[0].status = RuntimeStatus::Idle;
    monitor
        .scan_provider(&f.config, &connections, &f.store, "instance", &f.provider)
        .await
        .unwrap();
    assert_eq!(monitor.pending.len(), 1);
    f.provider.listed.lock().unwrap().clear();
    monitor
        .scan_provider(&f.config, &connections, &f.store, "instance", &f.provider)
        .await
        .unwrap();
    tokio::time::advance(DELAY).await;
    assert!(monitor.pending.is_empty());
    assert!(f.provider.sent.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn waits_a_full_minute_and_sends_exactly_one_continue_per_failed_turn() {
    let f = Fixture::new().await;
    let mut monitor = Monitor::default();
    f.inspect(&mut monitor).await;
    tokio::time::advance(Duration::from_secs(59)).await;
    f.inspect(&mut monitor).await;
    assert!(f.provider.sent.lock().unwrap().is_empty());
    tokio::time::advance(Duration::from_secs(1)).await;
    f.inspect(&mut monitor).await;
    let sent = f.provider.sent.lock().unwrap().clone();
    assert!(
        matches!(&sent[..], [Mutation::Start { thread, text, client_id }] if thread == "thread" && text == "继续" && !client_id.is_empty())
    );
    // Stale history and duplicate notifications cannot schedule a second send.
    for _ in 0..3 {
        tokio::time::advance(DELAY).await;
        f.inspect(&mut monitor).await;
    }
    assert_eq!(f.provider.sent.lock().unwrap().len(), 1);
    // A new failed turn gets its own minute and operation identity.
    f.provider.failure.lock().unwrap().as_mut().unwrap().turn_id = "next-failed-turn".into();
    f.inspect(&mut monitor).await;
    tokio::time::advance(DELAY).await;
    f.inspect(&mut monitor).await;
    assert_eq!(f.provider.sent.lock().unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn manual_progress_or_a_closed_session_cancels_pending_retry() {
    let f = Fixture::new().await;
    let mut monitor = Monitor::default();
    f.inspect(&mut monitor).await;
    *f.provider.failure.lock().unwrap() = None;
    tokio::time::advance(DELAY).await;
    f.inspect(&mut monitor).await;
    assert!(monitor.pending.is_empty());
    assert!(f.provider.sent.lock().unwrap().is_empty());

    f.provider
        .failure
        .lock()
        .unwrap()
        .replace(RetryableFailure {
            turn_id: "another-turn".into(),
            reason: RetryReason::ServerOverloaded,
        });
    f.inspect(&mut monitor).await;
    // Even if the list is stale, the final native read catches a closed session.
    f.provider.current.lock().unwrap().status = RuntimeStatus::NotLoaded;
    tokio::time::advance(DELAY).await;
    f.inspect(&mut monitor).await;
    assert!(monitor.pending.is_empty());
    assert!(f.provider.sent.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn configuration_revocation_and_cwd_changes_block_dispatch() {
    let f = Fixture::new().await;
    let mut monitor = Monitor::default();
    f.inspect(&mut monitor).await;
    std::fs::write(
        &f.path,
        "[[hosts]]\nid='host'\nname='host'\ntransport='local_proxy'\nallowed_workspace_roots=[]\n",
    )
    .unwrap();
    assert!(f.config.reload().unwrap());
    tokio::time::advance(DELAY).await;
    f.inspect(&mut monitor).await;
    assert!(f.provider.sent.lock().unwrap().is_empty());

    let f = Fixture::new().await;
    let mut monitor = Monitor::default();
    f.inspect(&mut monitor).await;
    f.provider.current.lock().unwrap().cwd = "/elsewhere/project".into();
    tokio::time::advance(DELAY).await;
    f.inspect(&mut monitor).await;
    assert!(f.provider.sent.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn unrelated_config_changes_preserve_the_original_retry_deadline() {
    let f = Fixture::new().await;
    let mut monitor = Monitor::default();
    f.inspect(&mut monitor).await;
    tokio::time::advance(Duration::from_secs(30)).await;
    std::fs::write(&f.path, "[[hosts]]\nid='host'\nname='renamed host'\ntransport='local_proxy'\nallowed_workspace_roots=['/projects']\n").unwrap();
    assert!(f.config.reload().unwrap());
    f.inspect(&mut monitor).await;
    tokio::time::advance(Duration::from_secs(30)).await;
    f.inspect(&mut monitor).await;
    assert_eq!(f.provider.sent.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn unknown_write_outcome_is_durable_and_never_replayed_after_restart() {
    let mut f = Fixture::new().await;
    f.provider.timeout = true;
    let mut monitor = Monitor::default();
    f.inspect(&mut monitor).await;
    let operation = monitor.pending["thread"].operation.clone();
    tokio::time::advance(DELAY).await;
    assert!(matches!(
        monitor
            .inspect(
                &f.config,
                &f.store,
                &f.provider,
                &f.session,
                &f.config.snapshot().hash
            )
            .await,
        Err(Error::Operation(operations::Error::Unknown))
    ));
    assert_eq!(
        f.store.operation_state(&operation).await.unwrap(),
        State::OutcomeUnknown
    );
    let mut restarted = Monitor::default();
    f.inspect(&mut restarted).await;
    tokio::time::advance(DELAY).await;
    f.inspect(&mut restarted).await;
    assert!(restarted.pending.is_empty());
    assert_eq!(f.provider.sent.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn restart_requires_fresh_observation_and_a_new_full_delay() {
    let f = Fixture::new().await;
    let mut monitor = Monitor::default();
    f.inspect(&mut monitor).await;
    tokio::time::advance(Duration::from_secs(59)).await;
    drop(monitor);
    let mut restarted = Monitor::default();
    f.inspect(&mut restarted).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    f.inspect(&mut restarted).await;
    assert!(f.provider.sent.lock().unwrap().is_empty());
    tokio::time::advance(Duration::from_secs(59)).await;
    f.inspect(&mut restarted).await;
    assert_eq!(f.provider.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn scope_requires_the_verified_instance_and_a_path_inside_allowed_roots() {
    let f = Fixture::new().await;
    let mut config = f.config.snapshot().config.clone();
    assert!(allowed(&config, "instance", &f.session));
    for cwd in [
        "/projects-other/test",
        "/projects/../private",
        "relative/path",
    ] {
        let mut session = f.session.clone();
        session.cwd = cwd.into();
        assert!(!allowed(&config, "instance", &session));
    }
    config.providers[0].control_verified = false;
    assert!(!allowed(&config, "instance", &f.session));
}
