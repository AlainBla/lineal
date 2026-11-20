//! Seed-Daten (acme, ENG, WEB, DES, Alice, Bob, Carol), Fakes und Test-Helfer.
//! Ausschließlich `dev-dependency` (Ausnahme: optionales Feature `seed` in `lineal-server`).
#![forbid(unsafe_code)]

pub mod fake;
pub mod seed;

pub use fake::{
    FakeStore, FakeTx, FixedClock, NeverMailer, RecordingMailer, fake_app, test_app, test_app_with,
    test_app_with_mailer,
};
pub use seed::{SeedData, SeedUser, ctx_for};
