//! Versioned protocol for PocketPilot-owned runtime servers, independent of native agent wire formats.
use crate::{
    History, Page, Session, SessionQuery,
    provider::{Mutation, Receipt},
};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 2;

pub const MAX_FRAME_BYTES: usize = 128 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub call: Call,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Call {
    Initialize {
        version: u32,
        runtime: String,
    },
    List {
        query: SessionQuery,
    },
    Read {
        thread: String,
    },
    Questions {
        thread: String,
    },
    History {
        thread: String,
    },
    Mutate {
        operation: String,
        mutation: Mutation,
    },
}

#[derive(Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub result: Result<Reply, Failure>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Initialized {
        version: u32,
        runtime: String,
        steer: bool,
    },
    Sessions {
        page: Page<Session>,
    },
    Session {
        session: Session,
    },
    Questions {
        dialogs: Vec<crate::questions::Dialog>,
    },
    History {
        history: History,
    },
    Receipt {
        receipt: Receipt,
    },
}

/// Fixed categories only; native stderr, permission prompts and model output never become errors.
#[derive(Clone, Copy, Debug, thiserror::Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    #[error("invalid runtime request")]
    Invalid,
    #[error("session was not found")]
    NotFound,
    #[error("runtime is busy")]
    Busy,
    #[error("runtime capability is unsupported")]
    Unsupported,
    #[error("session state changed")]
    Conflict,
    #[error("runtime path or operation is unauthorized")]
    Unauthorized,
    #[error("runtime write outcome is unknown")]
    Unknown,
    #[error("runtime state is unavailable")]
    Internal,
}
