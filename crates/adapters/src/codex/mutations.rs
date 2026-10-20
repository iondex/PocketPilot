//! Explicit user-authorized writes and native input encoding.
use super::Codex;
use crate::codex::protocol as p;
use session::{
    AgentProvider, ProviderError, RuntimeStatus,
    provider::{Mutation, Receipt},
};

impl Codex {
    pub(super) async fn execute_mutation(
        &self,
        mutation: Mutation,
    ) -> Result<Receipt, ProviderError> {
        if !self.control {
            return Err(ProviderError::Unsupported);
        }
        let (mutation, files) = match mutation {
            Mutation::StartWithFiles {
                thread,
                text,
                client_id,
                files,
            } => (
                Mutation::Start {
                    thread,
                    text: session::files::envelope(&text, &files),
                    client_id,
                },
                files,
            ),
            Mutation::SteerWithFiles {
                thread,
                expected_turn,
                text,
                client_id,
                files,
            } => (
                Mutation::Steer {
                    thread,
                    expected_turn,
                    text: session::files::envelope(&text, &files),
                    client_id,
                },
                files,
            ),
            mutation => (mutation, Vec::new()),
        };
        match mutation {
            Mutation::Upload {
                thread,
                path,
                bytes,
            } => {
                use base64::Engine;
                if bytes.len() > session::files::MAX_BYTES
                    || !std::path::Path::new(&path).is_absolute()
                {
                    return Err(ProviderError::Conflict);
                }
                let directory = std::path::Path::new(&path)
                    .parent()
                    .ok_or(ProviderError::Conflict)?
                    .to_string_lossy()
                    .into_owned();
                self.client
                    .write(&p::FsCreateDirectoryParams {
                        path: p::AbsolutePathBuf(directory),
                        recursive: Some(true),
                    })
                    .await?;
                self.client
                    .write(&p::FsWriteFileParams {
                        path: p::AbsolutePathBuf(path),
                        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                    })
                    .await?;
                Ok(Receipt { thread, turn: None })
            }
            Mutation::StartWithFiles { .. } | Mutation::SteerWithFiles { .. } => {
                Err(ProviderError::Incompatible)
            }
            Mutation::SetTitleIfMissing { thread, title } => {
                if title.trim().is_empty() {
                    return Err(ProviderError::Conflict);
                }
                let current = self
                    .client
                    .call(&p::ThreadReadParams {
                        thread_id: thread.clone(),
                        include_turns: Some(false),
                    })
                    .await?;
                if current
                    .thread
                    .name
                    .as_deref()
                    .is_none_or(|name| name.trim().is_empty())
                {
                    self.client
                        .write(&p::ThreadSetNameParams {
                            thread_id: thread.clone(),
                            name: title,
                        })
                        .await?;
                }
                Ok(Receipt { thread, turn: None })
            }
            Mutation::Create { cwd } => {
                let params: p::ThreadStartParams = p::ThreadStartParams::builder()
                    .cwd(Some(cwd))
                    .try_into()
                    .map_err(|_| ProviderError::Incompatible)?;
                let result = self.client.write(&params).await?;
                Ok(Receipt {
                    thread: result.thread.id,
                    turn: None,
                })
            }
            Mutation::Resume { thread } => {
                let params: p::ThreadResumeParams = p::ThreadResumeParams::builder()
                    .thread_id(thread.clone())
                    .exclude_turns(Some(true))
                    .try_into()
                    .map_err(|_| ProviderError::Incompatible)?;
                let result = self.client.write(&params).await?;
                if result.thread.id != thread {
                    return Err(ProviderError::Incompatible);
                }
                Ok(Receipt { thread, turn: None })
            }
            Mutation::Start {
                thread,
                text,
                client_id,
            } => {
                // Never convert a detected active turn into an unguarded turn/start.
                let current = self.read(&thread).await?;
                if current.status != RuntimeStatus::Idle {
                    return Err(ProviderError::Conflict);
                }
                let params: p::TurnStartParams = p::TurnStartParams::builder()
                    .thread_id(thread.clone())
                    .input(file_input(text, &files)?)
                    .client_user_message_id(Some(client_id))
                    .try_into()
                    .map_err(|_| ProviderError::Incompatible)?;
                let result = self.client.write(&params).await?;
                Ok(Receipt {
                    thread,
                    turn: Some(result.turn.id),
                })
            }
            Mutation::Steer {
                thread,
                expected_turn,
                text,
                client_id,
            } => {
                let params = p::TurnSteerParams {
                    thread_id: thread.clone(),
                    expected_turn_id: expected_turn,
                    input: text_input(text)?,
                    client_user_message_id: Some(client_id),
                };
                let result = self.client.write(&params).await?;
                Ok(Receipt {
                    thread,
                    turn: Some(result.turn_id),
                })
            }
        }
    }
}

fn file_input(
    text: String,
    files: &[session::files::Uploaded],
) -> Result<Vec<p::UserInput>, ProviderError> {
    let mut input = text_input(text)?;
    for file in files.iter().filter(|f| f.image) {
        input.push(p::UserInput::LocalImageUserInput {
            detail: None,
            path: file.path.clone(),
            type_: p::LocalImageUserInputType::LocalImage,
        });
    }
    Ok(input)
}

fn text_input(text: String) -> Result<Vec<p::UserInput>, ProviderError> {
    if text.trim().is_empty() || text.len() > 1024 * 1024 {
        return Err(ProviderError::Conflict);
    }
    Ok(vec![p::UserInput::TextUserInput {
        text,
        text_elements: vec![],
        type_: p::TextUserInputType::Text,
    }])
}

#[cfg(test)]
#[path = "mutations_tests.rs"]
mod tests;
