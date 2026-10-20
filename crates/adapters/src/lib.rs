//! Agent runtime contracts and implementations. Runtime protocols stay behind the adapter seam.
mod contracts;

pub use contracts::{Adapter, AdapterFactory, ConnectOptions, RuntimeKind, ServerEndpoint};

pub mod codex;
