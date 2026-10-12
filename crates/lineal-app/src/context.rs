//! Wer handelt in welchem Workspace (docs/11 §3.2).

use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, TeamId, UserId};

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
    /// Team-Beschränkung eines API-Keys (`api_key.team_ids`): nur diese Teams sind sichtbar und
    /// änderbar. `None` = keine Beschränkung (Sessions, OAuth, System).
    pub team_scope: Option<Vec<TeamId>>,
    /// `true`, wenn die Anfrage mit einer Web-Session (Cookie) authentifiziert wurde – nicht mit
    /// API-Key oder OAuth-Token. Nur dann dürfen z. B. neue API-Keys angelegt werden.
    pub via_session: bool,
    /// Client-IP des Requests (Audit-Log). `None` ohne Netzwerkkontext (Jobs, CLI, MCP-Transport).
    pub client_ip: Option<String>,
    /// Art der Zugangsdaten für den Audit-Log: `session`, `api_key`, `oauth`, `cli`; `None` ohne.
    pub credential_kind: Option<&'static str>,
}

impl RequestCtx {
    /// Systemkontext für Jobs und Konsumenten.
    pub fn system(org_id: OrgId, request_id: impl Into<String>) -> Self {
        Self {
            org_id,
            actor: Actor::System,
            request_id: request_id.into(),
            scopes: Scopes::all(),
            team_scope: None,
            via_session: false,
            client_ip: None,
            credential_kind: None,
        }
    }
}

/// Bezeichnung einer Token-Art im Audit-Log (`credential_kind`).
pub const fn credential_label(kind: crate::ports::repos::auth::CredentialKind) -> &'static str {
    use crate::ports::repos::auth::CredentialKind;
    match kind {
        CredentialKind::Session => "session",
        CredentialKind::ApiKey => "api_key",
        CredentialKind::OAuth => "oauth",
    }
}
