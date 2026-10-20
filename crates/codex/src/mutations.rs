//! Explicit user-authorized writes and native input encoding.
use super::Codex;
use crate::protocol as p;
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
mod tests {
    use super::*;
    use crate::Client;

    #[test]
    fn uploaded_images_use_native_image_input_and_documents_keep_their_paths_in_context() {
        let files = vec![session::files::Uploaded {
            name: "image.png".into(),
            path: "/private/uploads/image.png".into(),
            image: true,
        }];
        let input = file_input(
            session::files::envelope("Describe the file", &files),
            &files,
        )
        .unwrap();
        assert!(
            matches!(&input[0], p::UserInput::TextUserInput { text, .. } if text.contains("## image.png: /private/uploads/image.png") && text.contains("Describe the file"))
        );
        assert!(
            matches!(&input[1], p::UserInput::LocalImageUserInput { path, .. } if path == "/private/uploads/image.png")
        );
    }

    #[derive(serde::Deserialize)]
    struct Frame {
        id: Option<p::RequestId>,
        method: String,
        params: Option<Box<serde_json::value::RawValue>>,
    }

    const THREAD: &str = r#"{"id":"thread","sessionId":"thread","cliVersion":"0.162.1","projectId":null,"preview":"Title","cwd":"/workspace","ephemeral":false,"source":"appServer","modelProvider":"openai","createdAt":100,"updatedAt":100,"status":{"type":"idle"},"turns":[]}"#;

    #[tokio::test]
    async fn user_input_preserves_execution_policy_and_drop_sends_no_lifecycle_action() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_util::codec::{Framed, LinesCodec};
        let (local, remote) = tokio::io::duplex(16384);
        let (read, write) = tokio::io::split(local);
        let provider = Codex::new(
            "instance".into(),
            Client::spawn(
                Box::new(crate::transport::JsonLines::new(read, write, None)),
                std::time::Duration::from_secs(2),
            ),
        )
        .with_control(true);
        let server = tokio::spawn(async move {
            let mut server = Framed::new(remote, LinesCodec::new());
            let read: Frame = serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(read.method, "thread/read");
            let params: p::ThreadReadParams =
                serde_json::from_str(read.params.unwrap().get()).unwrap();
            assert_eq!(params.include_turns, Some(false));
            server
                .send(format!(
                    "{{\"id\":{},\"result\":{{\"thread\":{THREAD}}}}}",
                    read.id.unwrap()
                ))
                .await
                .unwrap();
            let start: Frame =
                serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(start.method, "turn/start");
            let params: p::TurnStartParams =
                serde_json::from_str(start.params.unwrap().get()).unwrap();
            assert_eq!(params.thread_id, "thread");
            assert_eq!(params.client_user_message_id.as_deref(), Some("message-1"));
            assert!(
                params.model.is_none()
                    && params.cwd.is_none()
                    && params.approval_policy.is_none()
                    && params.approvals_reviewer.is_none()
                    && params.sandbox_policy.is_none()
            );
            server.send(format!("{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"active-turn\",\"status\":\"inProgress\",\"items\":[]}}}}}}",start.id.unwrap())).await.unwrap();
            let steer: Frame =
                serde_json::from_str(&server.next().await.unwrap().unwrap()).unwrap();
            assert_eq!(steer.method, "turn/steer");
            let params: p::TurnSteerParams =
                serde_json::from_str(steer.params.unwrap().get()).unwrap();
            assert_eq!(params.expected_turn_id, "active-turn");
            assert_eq!(params.client_user_message_id.as_deref(), Some("message-2"));
            server
                .send(format!(
                    "{{\"id\":{},\"result\":{{\"turnId\":\"active-turn\"}}}}",
                    steer.id.unwrap()
                ))
                .await
                .unwrap();
            assert!(
                server.next().await.is_none(),
                "disconnect emitted an unexpected lifecycle request"
            );
        });
        let first = provider
            .execute(Mutation::Start {
                thread: "thread".into(),
                text: "first".into(),
                client_id: "message-1".into(),
            })
            .await
            .unwrap();
        assert_eq!(first.turn.as_deref(), Some("active-turn"));
        provider
            .execute(Mutation::Steer {
                thread: "thread".into(),
                expected_turn: "active-turn".into(),
                text: "second".into(),
                client_id: "message-2".into(),
            })
            .await
            .unwrap();
        drop(provider);
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn unverified_provider_refuses_mutation_before_sending_any_request() {
        use futures_util::StreamExt;
        use tokio_util::codec::{Framed, LinesCodec};
        let (local, remote) = tokio::io::duplex(1024);
        let (read, write) = tokio::io::split(local);
        let provider = Codex::new(
            "instance".into(),
            Client::spawn(
                Box::new(crate::transport::JsonLines::new(read, write, None)),
                std::time::Duration::from_secs(1),
            ),
        );
        assert!(matches!(
            provider
                .execute(Mutation::Create {
                    cwd: "/workspace".into()
                })
                .await,
            Err(ProviderError::Unsupported)
        ));
        drop(provider);
        assert!(
            Framed::new(remote, LinesCodec::new())
                .next()
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn missing_title_is_written_without_starting_a_turn_and_existing_titles_are_preserved() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_util::codec::{Framed, LinesCodec};
        for existing in [None, Some("Native title")] {
            let (local, remote) = tokio::io::duplex(16384);
            let (read, write) = tokio::io::split(local);
            let provider = Codex::new(
                "instance".into(),
                Client::spawn(
                    Box::new(crate::transport::JsonLines::new(read, write, None)),
                    std::time::Duration::from_secs(2),
                ),
            )
            .with_control(true);
            let server = tokio::spawn(async move {
                let mut stream = Framed::new(remote, LinesCodec::new());
                let frame: Frame =
                    serde_json::from_str(&stream.next().await.unwrap().unwrap()).unwrap();
                assert_eq!(frame.method, "thread/read");
                let mut thread: p::Thread = serde_json::from_str(THREAD).unwrap();
                thread.name = existing.map(str::to_owned);
                let result = p::ThreadReadResponse { thread };
                stream
                    .send(format!(
                        "{{\"id\":{},\"result\":{}}}",
                        frame.id.unwrap(),
                        serde_json::to_string(&result).unwrap()
                    ))
                    .await
                    .unwrap();
                if existing.is_none() {
                    let frame: Frame =
                        serde_json::from_str(&stream.next().await.unwrap().unwrap()).unwrap();
                    assert_eq!(frame.method, "thread/name/set");
                    let params: p::ThreadSetNameParams =
                        serde_json::from_str(frame.params.unwrap().get()).unwrap();
                    assert_eq!(params.name, "Bridge title");
                    assert_eq!(params.thread_id, "thread");
                    stream
                        .send(format!("{{\"id\":{},\"result\":{{}}}}", frame.id.unwrap()))
                        .await
                        .unwrap();
                }
                assert!(
                    stream.next().await.is_none(),
                    "metadata must not start, resume or stop a turn"
                );
            });
            let receipt = provider
                .execute(Mutation::SetTitleIfMissing {
                    thread: "thread".into(),
                    title: "Bridge title".into(),
                })
                .await
                .unwrap();
            assert!(receipt.turn.is_none());
            drop(provider);
            server.await.unwrap();
        }
    }
}
