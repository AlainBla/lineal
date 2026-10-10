//! Import aus Linear (WP18, docs/13): Port `ImportSource`, Adapter (Linear-GraphQL, CSV, Fake),
//! Phasen-Engine mit Checkpoints, Markdown-Umschreibung und abgesicherte Downloads.
//!
//! Lauf-Verwaltung (Scan/Start/Pause/Abbruch, Token-Verschlüsselung) steht in
//! `lineal_app::usecases::import`; dieses Crate führt die Läufe aus (Job, CLI).
#![forbid(unsafe_code)]

pub mod adapter;
pub mod blob;
pub mod engine;
pub mod fixtures;
pub mod markdown;
pub mod net;
pub mod port;
pub mod runner;

pub use adapter::csv::CsvSource;
pub use adapter::fake::FakeSource;
pub use adapter::linear::{LinearConfig, LinearSource};
pub use blob::{AppBlobSink, BlobSink, MemoryBlobSink};
pub use engine::{EngineConfig, ImportEngine, RunOutcome};
pub use net::UrlPolicy;
pub use port::{ImportSource, SourceEntity, SourceError};
