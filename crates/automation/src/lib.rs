//! # texturecraft-automation
//!
//! Agent-facing automation, L4 (no UI toolkit):
//!
//! * [`TexturecraftMcp`]: an MCP server (official `rmcp` SDK). Headless mode drives an in-process
//!   [`Headless`] session; bridge mode forwards to a running desktop app over the JSON-lines
//!   control channel (`docs/control-protocol.md`).
//! * [`Headless`]: a [`texturecraft_engine::Session`] plus file I/O through an
//!   [`AuthorizedWorkspace`] and windowless GPU renders. Shared by the MCP server, the RPC server
//!   and `texturecraft-cli`.
//! * [`rpc`]: the control channel's methods (plus `render`) over stdio or loopback TCP, used by
//!   `texturecraft-cli serve`.
//! * [`BridgeClient`]: the client side of the app's control channel.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bridge;
pub mod budgets;
pub mod headless;
pub mod render;
pub mod rpc;
pub mod security;
pub mod server;
pub mod turntable;
pub mod workspace;

pub use bridge::BridgeClient;
pub use headless::{Filesystem, Headless, RenderOptions};
pub use server::{Backend, TexturecraftMcp};
pub use turntable::{TurntableParams, TurntableReport};
pub use workspace::AuthorizedWorkspace;

#[derive(Debug, thiserror::Error)]
pub enum AutomationError {
    #[error("{0}")]
    BadRequest(String),
    #[error("I/O: {0}")]
    Io(String),
    #[error(transparent)]
    Engine(#[from] texturecraft_engine::EngineError),
    #[error("bridge: {0}")]
    Bridge(String),
    #[error("app: {0}")]
    App(String),
    #[error("{0}")]
    Other(String),
    #[error("cancelled")]
    Cancelled,
}

pub(crate) fn bad(msg: impl Into<String>) -> AutomationError {
    AutomationError::BadRequest(msg.into())
}
