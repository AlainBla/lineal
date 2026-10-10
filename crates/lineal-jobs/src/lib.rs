//! Job-Runner: Outbox-Konsumenten, periodische Jobs und die Worker-Schleife (docs/11 §3.6).
#![forbid(unsafe_code)]

pub mod consumers;
pub mod periodic;
pub mod registry;
pub mod traits;
pub mod worker;

pub use traits::{EventConsumer, PeriodicJob, Schedule};
pub use worker::{TickStats, Worker, WorkerConfig};
