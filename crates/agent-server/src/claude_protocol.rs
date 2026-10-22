//! Native stream-json DTOs and bounded input encoding, isolated from lifecycle state.
//!
//! Unknown protocol extensions are ignored except tool-request RawValue fields,
//! which must survive decoding unchanged so answers preserve the original input.
use crate::questions::{Request as ControlRequest, Snapshot};
use base64::Engine;
use serde::{Deserialize, Serialize};
use session::{files::Uploaded, server::Failure};

/// A single native line may include image data; both reader and writer enforce this budget.
pub(super) const MAX_NATIVE_FRAME: usize = 32 * 1024 * 1024;

// Read one extra byte to distinguish a valid image from a silently truncated upload.
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Input<'a> {
    ControlResponse {
        response: crate::questions::Response,
    },
    ControlRequest {
        request_id: &'a str,
        request: Initialize,
    },
    User {
        uuid: &'a str,
        session_id: &'a str,
        parent_tool_use_id: Option<&'a str>,
        message: Message<'a>,
    },
}

#[derive(Serialize)]
pub(super) struct Initialize {
    pub(super) subtype: &'static str,
}

#[derive(Serialize)]
pub(super) struct Message<'a> {
    pub(super) role: &'static str,
    pub(super) content: Content<'a>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum Content<'a> {
    Text(&'a str),
    Blocks(Vec<InputBlock<'a>>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum InputBlock<'a> {
    Text { text: &'a str },
    Image { source: ImageSource },
}

#[derive(Serialize)]
pub(super) struct ImageSource {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'static str,
    data: String,
}

pub(super) async fn content<'a>(body: &'a str, files: &[Uploaded]) -> Result<Content<'a>, Failure> {
    use tokio::io::AsyncReadExt;
    if !files.iter().any(|f| f.image) {
        return Ok(Content::Text(body));
    }
    let mut blocks = vec![InputBlock::Text { text: body }];
    for file in files.iter().filter(|f| f.image) {
        let mut bytes = Vec::new();
        tokio::fs::File::open(&file.path)
            .await
            .map_err(|_| Failure::Unauthorized)?
            .take(MAX_IMAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Failure::Internal)?;
        if bytes.len() as u64 > MAX_IMAGE_BYTES {
            return Err(Failure::Unsupported);
        }
        let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            "image/gif"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
            "image/webp"
        } else {
            return Err(Failure::Unsupported);
        };
        blocks.push(InputBlock::Image {
            source: ImageSource {
                kind: "base64",
                media_type: mime,
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            },
        });
    }
    Ok(Content::Blocks(blocks))
}

// Unknown extensions are ignored at this boundary, never inspected as business JSON.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Output {
    ControlResponse {
        response: ControlResponse,
    },
    ControlRequest {
        request_id: String,
        request: ControlRequest,
    },
    ControlCancelRequest {
        request_id: String,
    },
    User {
        #[serde(default)]
        uuid: Option<String>,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default, rename = "isReplay")]
        replay: bool,
    },
    Assistant {
        session_id: String,
        uuid: String,
        message: Assistant,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
    },
    Result {
        session_id: String,
        subtype: String,
        #[serde(default)]
        is_error: bool,
        #[serde(default)]
        result: Option<String>,
    },
    System {
        subtype: String,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        state: Option<NativeState>,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum NativeState {
    Idle,
    Running,
    RequiresAction,
}

#[derive(Deserialize)]
pub(super) struct ControlResponse {
    pub(super) subtype: String,
    pub(super) request_id: String,
    #[serde(default)]
    pub(super) pending_permission_requests: Option<Vec<Snapshot>>,
    #[serde(default)]
    pub(super) pending_user_dialog_requests: Option<Vec<Snapshot>>,
}

#[derive(Deserialize)]
pub(super) struct Assistant {
    pub(super) content: Vec<Block>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Block {
    Text {
        text: String,
    },
    #[serde(other)]
    Other,
}

/// Decode control payloads directly from JSON. Serde's internally tagged enum
/// buffering cannot preserve RawValue extensions in nested tool input/snapshots.
pub(super) fn decode(line: &str) -> Result<Output, serde_json::Error> {
    #[derive(Deserialize)]
    struct Kind {
        #[serde(rename = "type")]
        kind: String,
    }

    #[derive(Deserialize)]
    struct RequestFrame {
        request_id: String,
        request: ControlRequest,
    }

    #[derive(Deserialize)]
    struct ResponseFrame {
        response: ControlResponse,
    }

    match serde_json::from_str::<Kind>(line)?.kind.as_str() {
        "control_request" => {
            let frame: RequestFrame = serde_json::from_str(line)?;
            Ok(Output::ControlRequest {
                request_id: frame.request_id,
                request: frame.request,
            })
        }
        "control_response" => {
            let frame: ResponseFrame = serde_json::from_str(line)?;
            Ok(Output::ControlResponse {
                response: frame.response,
            })
        }
        _ => serde_json::from_str(line),
    }
}
