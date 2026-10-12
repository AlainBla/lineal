//! Application-Layer von Lineal: Use-Cases, Ports (Traits), Policy und Referenzauflösung.
//! Kennt weder Datenbank noch HTTP (docs/11 §2).
#![forbid(unsafe_code)]

pub mod app;
pub mod collab;
pub mod context;
pub mod error;
pub mod policy;
pub mod ports;
pub mod resolve;
pub mod usecases;

pub use app::{App, AppConfig, WorkspaceCreation};
pub use context::{Actor, RequestCtx, Scopes};
pub use error::{AppError, AppResult, ErrorKind};
