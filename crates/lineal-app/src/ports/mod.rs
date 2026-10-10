//! Ports (Traits), die Adapter implementieren. Gemeinsame Datei: nur S1 / Orchestrator.

pub mod repos;
pub mod services;
pub mod tx;

pub use services::{BlobStore, Clock, IdGen, Mail, Mailer, PasswordHasher, SearchHit, SearchIndex};
pub use tx::{Store, Tx};
