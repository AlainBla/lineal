//! Wer handelt in welchem Workspace (docs/11 §3.2).

use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, UserId};

/// Handelnder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// Benutzer mit Rolle.
    User { id: UserId, role: UserRole },
    /// App/Integration (OAuth-Client).
    App { id: UserId },
    /// Systemintern (Jobs, Automationen).
    System,
}

/// Berechtigungs-Scopes eines Tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scopes {
    pub read: bool,
    pub write: bool,
    pub admin: bool,
}

impl Scopes {
    /// Nur lesen.
    pub const fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            admin: false,
        }
    }
    /// Lesen und schreiben.
    pub const fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            admin: false,
        }
    }
    /// Alle Scopes.
    pub const fn all() -> Self {
        Self {
            read: true,
            write: true,
            admin: true,
        }
    }
}

/// Kontext jeder Anfrage; von jedem Adapter aus Session, API-Key oder OAuth-Token gebaut.
#[derive(Debug, Clone)]
pub struct RequestCtx {
    pub org_id: OrgId,
    pub actor: Actor,
    pub request_id: String,
    pub scopes: Scopes,
}

impl RequestCtx {
    /// Systemkontext für Jobs und Konsumenten.
    pub fn system(org_id: OrgId, request_id: impl Into<String>) -> Self {
        Self {
            org_id,
            actor: Actor::System,
            request_id: request_id.into(),
            scopes: Scopes::all(),
        }
    }
}
