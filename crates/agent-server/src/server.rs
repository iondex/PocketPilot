//! Private Unix RPC server. Writes continue after a proxy disconnects and are never replayed.
use crate::{
    claude::Driver,
    storage::{Store, private_directory},
};
use futures_util::{SinkExt, StreamExt};
use session::{
    RuntimeStatus, Session, SessionRef,
    provider::{Mutation, Receipt},
    server::{self, Call, Failure, Reply, Request, Response},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::{Mutex, Semaphore},
};
use tokio_util::codec::{Framed, LinesCodec};

const MAX_SESSIONS: usize = 128;

struct Runtime {
    store: Store,
    binary: PathBuf,
    roots: Vec<PathBuf>,
    uploads: PathBuf,
    drivers: Mutex<HashMap<String, Arc<Mutex<Option<Driver>>>>>,
}

pub async fn serve(
    socket: PathBuf,
    directory: PathBuf,
    binary: PathBuf,
    roots: Vec<PathBuf>,
) -> Result<(), Failure> {
    private_directory(&directory)?;
    let directory = std::fs::canonicalize(directory).map_err(|_| Failure::Invalid)?;
    let socket = if socket.is_absolute() {
        socket
    } else {
        std::env::current_dir()
            .map_err(|_| Failure::Internal)?
            .join(socket)
    };
    if socket.parent() != Some(directory.as_path()) {
        return Err(Failure::Invalid);
    }
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join("server.lock"))
        .map_err(|_| Failure::Internal)?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| Failure::Busy)?;
    if let Ok(metadata) = std::fs::symlink_metadata(&socket) {
        if !metadata.file_type().is_socket() {
            return Err(Failure::Unauthorized);
        }
        if UnixStream::connect(&socket).await.is_ok() {
            return Err(Failure::Busy);
        }
        std::fs::remove_file(&socket).map_err(|_| Failure::Internal)?;
    }
    let roots = roots
        .into_iter()
        .map(|p| std::fs::canonicalize(p).map_err(|_| Failure::Invalid))
        .collect::<Result<Vec<_>, _>>()?;
    if roots.is_empty() || roots.iter().any(|p| !p.is_dir()) {
        return Err(Failure::Invalid);
    }
    let uploads = directory.join("uploads");
    private_directory(&uploads)?;
    let store = Store::open(&directory).await?;
    let endpoint = adapters_endpoint(&socket)?;
    let manifest = directory.join("endpoint.json");
    let mut manifest_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(manifest)
        .map_err(|_| Failure::Internal)?;
    use std::io::Write;
    manifest_file
        .write_all(&serde_json::to_vec(&endpoint).map_err(|_| Failure::Internal)?)
        .and_then(|()| manifest_file.sync_all())
        .map_err(|_| Failure::Internal)?;
    let listener = UnixListener::bind(&socket).map_err(|_| Failure::Internal)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| Failure::Internal)?;
    let runtime = Arc::new(Runtime {
        store,
        binary,
        roots,
        uploads,
        drivers: Mutex::new(HashMap::new()),
    });
    let clients = Arc::new(Semaphore::new(32));
    loop {
        let (stream, _) = listener.accept().await.map_err(|_| Failure::Internal)?;
        let Ok(permit) = clients.clone().try_acquire_owned() else {
            continue;
        };
        let runtime = runtime.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = client(stream, runtime).await;
        });
    }
}

#[derive(serde::Serialize)]
struct EndpointManifest {
    binary: PathBuf,
    socket: PathBuf,
}

fn adapters_endpoint(socket: &Path) -> Result<EndpointManifest, Failure> {
    Ok(EndpointManifest {
        binary: std::env::current_exe().map_err(|_| Failure::Internal)?,
        socket: socket.into(),
    })
}

/// A byte proxy owns neither runtime nor native agent process.
pub async fn proxy(socket: &Path) -> Result<(), Failure> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|_| Failure::Internal)?;
    let (mut read, mut write) = stream.into_split();
    let input = async { tokio::io::copy(&mut tokio::io::stdin(), &mut write).await };
    let output = async { tokio::io::copy(&mut read, &mut tokio::io::stdout()).await };
    tokio::select! {result=input=>{result.map_err(|_|Failure::Internal)?;},result=output=>{result.map_err(|_|Failure::Internal)?;}}
    Ok(())
}

async fn client(stream: UnixStream, runtime: Arc<Runtime>) -> Result<(), Failure> {
    let mut framed = Framed::new(
        stream,
        LinesCodec::new_with_max_length(server::MAX_FRAME_BYTES),
    );
    let mut initialized = false;
    while let Some(line) = tokio::time::timeout(Duration::from_secs(300), framed.next())
        .await
        .map_err(|_| Failure::Invalid)?
    {
        let line = line.map_err(|_| Failure::Invalid)?;
        let request: Request = serde_json::from_str(&line).map_err(|_| Failure::Invalid)?;
        let result = match request.call {
            Call::Initialize { version, runtime }
                if !initialized && version == server::VERSION && runtime == "claude_code" =>
            {
                initialized = true;
                Ok(Reply::Initialized {
                    version: server::VERSION,
                    runtime,
                    steer: false,
                })
            }
            _ if !initialized => Err(Failure::Unsupported),
            call => runtime.call(call).await,
        };
        let response = serde_json::to_string(&Response {
            id: request.id,
            result,
        })
        .map_err(|_| Failure::Internal)?;
        if response.len() > server::MAX_FRAME_BYTES {
            return Err(Failure::Unsupported);
        }
        tokio::time::timeout(Duration::from_secs(15), framed.send(response))
            .await
            .map_err(|_| Failure::Unknown)?
            .map_err(|_| Failure::Unknown)?;
    }
    Ok(())
}

impl Runtime {
    async fn call(&self, call: Call) -> Result<Reply, Failure> {
        match call {
            Call::List { query } => Ok(Reply::Sessions {
                page: self.store.list(query).await?,
            }),
            Call::Read { thread } => Ok(Reply::Session {
                session: self.store.read(&thread).await?,
            }),
            Call::Questions { thread } => {
                self.store.read(&thread).await?;
                let slot = self.drivers.lock().await.get(&thread).cloned();
                let dialogs = if let Some(slot) = slot {
                    let driver = slot.lock().await;
                    if let Some(driver) = driver.as_ref().filter(|d| d.alive()) {
                        driver.questions().await?
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };
                Ok(Reply::Questions { dialogs })
            }
            Call::History { thread } => Ok(Reply::History {
                history: self.store.history(&thread).await?,
            }),
            Call::Mutate {
                operation,
                mutation,
            } => Ok(Reply::Receipt {
                receipt: self.mutate(&operation, mutation).await?,
            }),
            Call::Initialize { .. } => Err(Failure::Invalid),
        }
    }

    fn cwd(&self, path: &str) -> Result<PathBuf, Failure> {
        let path = std::fs::canonicalize(path).map_err(|_| Failure::Invalid)?;
        if !path.is_dir() || !self.roots.iter().any(|root| path.starts_with(root)) {
            return Err(Failure::Unauthorized);
        }
        Ok(path)
    }

    async fn slot(&self, thread: &str) -> Result<Arc<Mutex<Option<Driver>>>, Failure> {
        let mut drivers = self.drivers.lock().await;
        if drivers.len() >= MAX_SESSIONS && !drivers.contains_key(thread) {
            return Err(Failure::Busy);
        }
        Ok(drivers.entry(thread.into()).or_default().clone())
    }

    async fn mutate(&self, operation: &str, mutation: Mutation) -> Result<Receipt, Failure> {
        if operation.is_empty() || operation.len() > 200 {
            return Err(Failure::Invalid);
        }
        if matches!(
            mutation,
            Mutation::Steer { .. } | Mutation::SteerWithFiles { .. }
        ) {
            return Err(Failure::Unsupported);
        }
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&mutation).map_err(|_| Failure::Invalid)?)
        );
        // Check the durable receipt before checking current turn state: an accepted
        // operation remains accepted even after its native turn has ended.
        if let Some(receipt) = self.store.claim(operation, &digest).await? {
            return Ok(receipt);
        }
        if let Mutation::Create { cwd } = mutation {
            let cwd = self.cwd(&cwd)?;
            let thread = uuid::Uuid::new_v4().to_string();
            self.store
                .create(&Session {
                    reference: SessionRef {
                        instance: "claude_code".into(),
                        thread: thread.clone(),
                    },
                    title: String::new(),
                    native_title: None,
                    cwd: cwd.to_string_lossy().into(),
                    status: RuntimeStatus::NotLoaded,
                    active_turn: None,
                    updated_at: None,
                })
                .await?;
            let receipt = Receipt { thread, turn: None };
            self.store.accept(operation, &receipt).await?;
            return Ok(receipt);
        }
        let thread = match &mutation {
            Mutation::Answer { thread, .. }
            | Mutation::Resume { thread }
            | Mutation::SetTitleIfMissing { thread, .. }
            | Mutation::Upload { thread, .. }
            | Mutation::Start { thread, .. }
            | Mutation::StartWithFiles { thread, .. } => thread.clone(),
            _ => return Err(Failure::Unsupported),
        };
        uuid::Uuid::parse_str(&thread).map_err(|_| Failure::Invalid)?;
        let session = self.store.read(&thread).await?;
        let cwd = self.cwd(&session.cwd)?;
        let slot = self.slot(&thread).await?;
        let mut driver = slot.lock().await;
        match mutation {
            Mutation::Answer { submission, .. } => {
                let receipt = driver
                    .as_ref()
                    .filter(|d| d.alive())
                    .ok_or(Failure::Conflict)?
                    .answer(submission)
                    .await?;
                self.store.accept(operation, &receipt).await?;
                return Ok(receipt);
            }
            Mutation::SetTitleIfMissing { title, .. } => {
                if title.len() > 512 || title.contains(['\0', '\n', '\r']) {
                    return Err(Failure::Invalid);
                }
                self.store.title(&thread, &title).await?;
            }
            Mutation::Upload { path, bytes, .. } => self.upload(&thread, &path, &bytes).await?,
            Mutation::Resume { .. } => {
                if driver.as_ref().is_none_or(|d| !d.alive()) {
                    *driver =
                        Some(Driver::open(self.store.clone(), &thread, &self.binary, &cwd).await?);
                }
            }
            Mutation::Start {
                text, client_id, ..
            } => {
                return self
                    .start(
                        &mut driver,
                        operation,
                        &thread,
                        &cwd,
                        text,
                        client_id,
                        vec![],
                    )
                    .await;
            }
            Mutation::StartWithFiles {
                text,
                client_id,
                files,
                ..
            } => {
                return self
                    .start(
                        &mut driver,
                        operation,
                        &thread,
                        &cwd,
                        text,
                        client_id,
                        files,
                    )
                    .await;
            }
            _ => return Err(Failure::Unsupported),
        }
        let receipt = Receipt { thread, turn: None };
        self.store.accept(operation, &receipt).await?;
        Ok(receipt)
    }

    #[allow(clippy::too_many_arguments)]
    async fn start(
        &self,
        driver: &mut Option<Driver>,
        operation: &str,
        thread: &str,
        cwd: &Path,
        text: String,
        client: String,
        files: Vec<session::files::Uploaded>,
    ) -> Result<Receipt, Failure> {
        if text.len() > 1024 * 1024
            || client.is_empty()
            || client.len() > 200
            || files.len() > session::files::MAX_FILES
        {
            return Err(Failure::Invalid);
        }
        for file in &files {
            self.store.check_file(thread, &file.path).await?;
            let canonical = std::fs::canonicalize(&file.path).map_err(|_| Failure::Unauthorized)?;
            if canonical != Path::new(&file.path) || !canonical.starts_with(&self.uploads) {
                return Err(Failure::Unauthorized);
            }
        }
        let status = self.store.read(thread).await?.status;
        if matches!(status, RuntimeStatus::Active { .. }) {
            return Err(Failure::Busy);
        }
        if driver.as_ref().is_none_or(|d| !d.alive()) {
            *driver = Some(Driver::open(self.store.clone(), thread, &self.binary, cwd).await?);
        }
        if matches!(
            self.store.read(thread).await?.status,
            RuntimeStatus::Active { .. }
        ) {
            return Err(Failure::Busy);
        }
        driver
            .as_ref()
            .ok_or(Failure::Internal)?
            .start(operation, text, client, files)
            .await
    }

    async fn upload(&self, thread: &str, path: &str, bytes: &[u8]) -> Result<(), Failure> {
        let path = Path::new(path);
        if bytes.len() > session::files::MAX_BYTES
            || !path.starts_with(&self.uploads)
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(Failure::Unauthorized);
        }
        let relative = path
            .strip_prefix(&self.uploads)
            .map_err(|_| Failure::Unauthorized)?;
        if relative.components().count() != 3 {
            return Err(Failure::Invalid);
        }
        let mut parent = self.uploads.clone();
        for component in relative.parent().ok_or(Failure::Invalid)?.components() {
            parent.push(component);
            private_directory(&parent)?;
        }
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| Failure::Conflict)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| Failure::Internal)?;
        self.store
            .own_file(thread, path.to_str().ok_or(Failure::Invalid)?)
            .await
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
