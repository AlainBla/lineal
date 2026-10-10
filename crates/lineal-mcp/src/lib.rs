//! MCP-Adapter: Tool-Dispatch und Handler. Transport und Validierung folgen in WP04
//! (bewusst noch kein MCP-Crate wie `rmcp`).
#![forbid(unsafe_code)]

pub mod auth;
pub mod dispatch;
pub mod envelope;
pub mod error;
pub mod fields;
pub mod handlers;
pub mod server;
pub mod transport;
pub mod validate;
pub mod wellknown;

pub use server::router;
