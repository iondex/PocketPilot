//! Chat integrations and their shared lifecycle, rendering and transport contracts.
mod contracts;

pub use contracts::{AttachmentSource, Context, Integration, IntegrationError, Platform};

pub mod navigation;

pub mod bindings;

pub mod rendering;

pub mod network;

pub mod telegram;

pub mod discord;

pub mod slack;
