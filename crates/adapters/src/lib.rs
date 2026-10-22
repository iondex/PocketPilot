//! Runtime implementations behind session contracts. Native protocol DTOs remain in each adapter.

pub mod claude;

pub mod codex;

mod contracts;

pub use contracts::{Adapter, AdapterFactory, ConnectOptions, RuntimeKind, ServerEndpoint};
