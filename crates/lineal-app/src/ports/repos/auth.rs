//! Repository-Port `AuthRepo`: Sessions, API-Keys, Token-Lookup.

use crate::context::{Actor, Scopes};
use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{ApiKeyId, OrgId, TeamId, UserId};
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Art des Tokens, bestimmt durch die Tabelle, in der der Hash gefunden wurde.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    Session,
    ApiKey,
    OAuth,
}

/// Aufgelöster Aufrufer eines Tokens (Session, API-Key oder OAuth-Token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Tatsächliche Token-Art (nicht die Transportart Cookie/Header).
    pub kind: CredentialKind,
    pub org_id: OrgId,
    pub actor: Actor,
    pub scopes: Scopes,
    /// Team-Beschränkung eines API-Keys (`api_key.team_ids`); `None` = unbeschränkt.
    pub team_scope: Option<Vec<TeamId>>,
}

/// Kandidat für den Passwort-Login (ein User in einem Workspace).
#[derive(Debug, Clone, PartialEq)]
pub struct LoginCandidate {
    pub org_id: OrgId,
    pub url_key: String,
    pub user_id: UserId,
    pub role: UserRole,
    pub active: bool,
    pub password_hash: Option<String>,
}

/// Neue Web-Session (nur der Hash des Tokens wird gespeichert).
#[derive(Debug, Clone, PartialEq)]
pub struct NewSession {
    pub id: Uuid,
    pub user_id: UserId,
    pub token_hash: String,
    pub csrf_token_hash: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

/// Aktive Web-Session eines Users (für „Active sessions“).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub id: Uuid,
    /// SHA-256 des Tokens; nur intern zum Erkennen der aktuellen Session, nie ausgeben.
    pub token_hash: String,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

/// Neuer persönlicher API-Key.
#[derive(Debug, Clone, PartialEq)]
pub struct NewApiKey {
    pub id: ApiKeyId,
    pub user_id: UserId,
    pub label: String,
    /// Die ersten 12 Zeichen des Klartexts (Anzeige).
    pub key_prefix: String,
    /// SHA-256 (hex) des vollständigen Keys.
    pub key_hash: String,
    /// `read` (immer), `write`, `admin`, …
    pub scopes: Vec<String>,
    pub team_ids: Option<Vec<TeamId>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub now: DateTime<Utc>,
}

/// Gespeicherter API-Key (ohne Hash).
#[derive(Debug, Clone, PartialEq)]
pub struct ApiKeyRecord {
    pub id: ApiKeyId,
    pub user_id: UserId,
    pub label: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub team_ids: Option<Vec<TeamId>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait AuthRepo: Send {
    /// Sucht den Aufrufer zu einem SHA-256-Token-Hash (hex) in Sessions, API-Keys und
    /// OAuth-Access-Tokens. Läuft mandantenübergreifend (typisch unter `begin_system`).
    /// Abgelaufene/widerrufene Tokens und deaktivierte User liefern `None`.
    async fn find_principal_by_token_hash(&mut self, hash: &str) -> AppResult<Option<Principal>>;

    /// Login-Kandidaten zu einer E-Mail (optional eingegrenzt auf einen Workspace-`urlKey`).
    /// Mandantenübergreifend; nur unter `begin_system` aufrufen.
    async fn find_login_candidates(
        &mut self,
        _email: &str,
        _url_key: Option<&str>,
    ) -> AppResult<Vec<LoginCandidate>> {
        unimplemented_repo()
    }

    async fn insert_session(&mut self, _session: &NewSession) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Widerruft die Session zum Token-Hash; `true`, wenn eine aktive Session betroffen war.
    async fn revoke_session(&mut self, _token_hash: &str, _now: DateTime<Utc>) -> AppResult<bool> {
        unimplemented_repo()
    }

    /// Aktive (nicht widerrufene, nicht abgelaufene) Sessions des Users, neueste zuerst.
    async fn list_sessions(
        &mut self,
        _user_id: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<SessionRecord>> {
        unimplemented_repo()
    }
    /// Widerruft die Session `id` **des Users**; `false`, wenn sie nicht existiert, einem anderen
    /// User gehört oder schon widerrufen ist.
    async fn revoke_session_by_id(
        &mut self,
        _user_id: UserId,
        _id: Uuid,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Widerruft alle aktiven Sessions des Users außer der mit `keep_token_hash`; liefert die Zahl.
    async fn revoke_other_sessions(
        &mut self,
        _user_id: UserId,
        _keep_token_hash: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        unimplemented_repo()
    }

    async fn insert_api_key(&mut self, _key: &NewApiKey) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get_api_key(&mut self, _id: ApiKeyId) -> AppResult<Option<ApiKeyRecord>> {
        unimplemented_repo()
    }
    /// Keys eines Users (neueste zuerst, inkl. widerrufener).
    async fn list_api_keys(&mut self, _user_id: UserId) -> AppResult<Vec<ApiKeyRecord>> {
        unimplemented_repo()
    }
    /// Setzt `revokedAt`; `false`, wenn der Key nicht existiert oder schon widerrufen ist.
    async fn revoke_api_key(&mut self, _id: ApiKeyId, _now: DateTime<Utc>) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Widerruft alle Web-Sessions, API-Keys und OAuth-Tokens des Users (beim Sperren, damit eine
    /// Entsperrung keine alten Zugangsdaten wiederbelebt).
    async fn revoke_all_credentials_of_user(
        &mut self,
        _user_id: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
}
