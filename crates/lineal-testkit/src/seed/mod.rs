//! Seed-Workspace `acme`. Gemeinsame Datei: nur S1 / Orchestrator.
//!
//! `run` ruft die Bereichsfunktionen in fester Reihenfolge auf; jedes Arbeitspaket füllt nur
//! seine eigene Datei (`identity` WP02, `issues` WP06, `planning` WP07, `content` WP10).

pub mod content;
pub mod identity;
pub mod issues;
pub mod planning;

use lineal_app::ports::Tx;
use lineal_app::{Actor, AppResult, RequestCtx, Scopes};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, TeamId, UserId};
use uuid::Uuid;

/// Wer aus den Beispieldaten handelt (docs/02 §1: Alice owner, Bob user, Carol guest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedUser {
    Alice,
    Bob,
    Carol,
}

/// Feste IDs der Beispieldaten (deterministisch, damit Tests sie kennen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedData {
    pub org: OrgId,
    pub alice: UserId,
    pub bob: UserId,
    pub carol: UserId,
    /// Engineering.
    pub eng: TeamId,
    /// Web, Sub-Team von ENG.
    pub web: TeamId,
    /// Design, privat.
    pub des: TeamId,
}

impl SeedData {
    /// Die festen IDs des Workspace `acme`.
    pub const fn acme() -> Self {
        Self {
            org: OrgId(Uuid::from_u128(0xAC3E_0000_0000_7000_8000_0000_0000_0001)),
            alice: UserId(Uuid::from_u128(0xA11C_E000_0000_7000_8000_0000_0000_0001)),
            bob: UserId(Uuid::from_u128(0xB0B0_0000_0000_7000_8000_0000_0000_0002)),
            carol: UserId(Uuid::from_u128(0xCA20_1000_0000_7000_8000_0000_0000_0003)),
            eng: TeamId(Uuid::from_u128(0x7EA4_0000_0000_7000_8000_0000_0000_0001)),
            web: TeamId(Uuid::from_u128(0x7EA4_0000_0000_7000_8000_0000_0000_0002)),
            des: TeamId(Uuid::from_u128(0x7EA4_0000_0000_7000_8000_0000_0000_0003)),
        }
    }

    /// ID und Rolle eines Seed-Users.
    pub const fn user(&self, user: SeedUser) -> (UserId, UserRole) {
        match user {
            SeedUser::Alice => (self.alice, UserRole::Owner),
            SeedUser::Bob => (self.bob, UserRole::User),
            SeedUser::Carol => (self.carol, UserRole::Guest),
        }
    }
}

/// `RequestCtx` für einen Seed-User (alle Scopes).
pub fn ctx_for(user: SeedUser) -> RequestCtx {
    let data = SeedData::acme();
    let (id, role) = data.user(user);
    RequestCtx {
        org_id: data.org,
        actor: Actor::User { id, role },
        request_id: format!("req_test_{user:?}").to_lowercase(),
        scopes: Scopes::all(),
    }
}

/// Legt den kompletten Beispiel-Workspace in der Transaktion an.
pub async fn run(tx: &mut dyn Tx) -> AppResult<SeedData> {
    let data = SeedData::acme();
    identity::seed(tx, &data).await?;
    issues::seed(tx, &data).await?;
    planning::seed(tx, &data).await?;
    content::seed(tx, &data).await?;
    Ok(data)
}
