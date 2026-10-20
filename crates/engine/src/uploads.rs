//! Build paths under the configured Codex account's private home, never a user-supplied path.
use crate::config::Provider;
use session::{
    ProviderError,
    files::{Downloaded, Uploaded},
    provider::Mutation,
};

pub fn prepare(
    provider: &Provider,
    source: &str,
    thread: &str,
    downloaded: Vec<Downloaded>,
) -> Result<(Vec<Mutation>, Vec<Uploaded>), ProviderError> {
    if downloaded.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    if downloaded.len() > session::files::MAX_FILES {
        return Err(ProviderError::Unsupported);
    }
    let endpoint = provider
        .endpoint
        .as_ref()
        .ok_or(ProviderError::Unsupported)?;
    let socket = std::path::Path::new(&endpoint.socket);
    let root = socket
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "app-server-control"))
        .and_then(|p| p.parent())
        .ok_or(ProviderError::Unsupported)?;
    let directory = root
        .join("pocketpilot")
        .join("uploads")
        .join(crate::digest(source));
    let mut mutations = Vec::new();
    let mut files = Vec::new();
    for (index, file) in downloaded.into_iter().enumerate() {
        if file.bytes.len() > session::files::MAX_BYTES {
            return Err(ProviderError::Unsupported);
        }
        let name = session::files::name(&file.name);
        let path = directory
            .join(index.to_string())
            .join(&name)
            .to_string_lossy()
            .into_owned();
        files.push(Uploaded {
            name,
            path: path.clone(),
            image: file.image,
        });
        mutations.push(Mutation::Upload {
            thread: thread.into(),
            path,
            bytes: file.bytes,
        });
    }
    Ok((mutations, files))
}

pub fn attach(mutation: Mutation, files: Vec<Uploaded>) -> Mutation {
    if files.is_empty() {
        return mutation;
    }
    match mutation {
        Mutation::Start {
            thread,
            text,
            client_id,
        } => Mutation::StartWithFiles {
            thread,
            text,
            client_id,
            files,
        },
        Mutation::Steer {
            thread,
            expected_turn,
            text,
            client_id,
        } => Mutation::SteerWithFiles {
            thread,
            expected_turn,
            text,
            client_id,
            files,
        },
        other => other,
    }
}

#[cfg(test)]
#[path = "uploads_tests.rs"]
mod tests;
