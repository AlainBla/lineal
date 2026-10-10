//! Seed-Daten (acme, ENG, WEB, DES, Alice, Bob, Carol), Fakes und Test-Helfer.
//! Ausschließlich `dev-dependency` (Ausnahme: optionales Feature `seed` in `lineal-server`).
#![forbid(unsafe_code)]

pub mod fake;
pub mod seed;

pub use fake::{FakeStore, FakeTx, fake_app, test_app};
pub use seed::{SeedData, SeedUser, ctx_for};
