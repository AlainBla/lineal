//! Reiner Domain-Core von Lineal: Werte, IDs, Enums, Regeln und Domain-Events.
//! Keine I/O-Abhängigkeiten (siehe docs/11 §2).
#![forbid(unsafe_code)]

pub mod enums;
pub mod error;
pub mod events;
pub mod identifier;
pub mod ids;
pub mod rules;
pub mod time;

pub use error::DomainError;
