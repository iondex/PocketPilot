//! Chat platforms and shared navigation, binding, rendering and network boundaries.

pub mod bindings;

pub mod discord;

pub mod feishu;

pub mod navigation;

pub mod network;

pub mod rendering;

pub mod slack;

pub mod telegram;

mod contracts;

mod questions;

pub use contracts::{AttachmentSource, Context, Integration, IntegrationError, Platform};
