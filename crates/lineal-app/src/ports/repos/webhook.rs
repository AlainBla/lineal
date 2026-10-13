//! Repository-Port `WebhookRepo` (WP16).
//!
//! Der Port bündelt die Persistenz des Pakets WP16: Webhooks und Zustellprotokoll, OAuth-
//! Autorisierungsserver, Einladungen und Team-Lebenszyklus. Alle Methoden haben
//! Default-Implementierungen (`not implemented`), damit Fakes ohne Datenbank kompilieren.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, TeamId, UserId, WebhookId};
use serde_json::Value;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

// ---------------------------------------------------------------------------
// Webhooks
// ---------------------------------------------------------------------------

/// Gespeicherter Webhook. Die Secrets sind **verschlüsselt** (siehe `usecases::webhook::seal`).
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookRecord {
    pub id: WebhookId,
    pub org_id: OrgId,
    pub label: Option<String>,
    pub url: String,
    pub secret_enc: String,
    pub previous_secret_enc: Option<String>,
    pub secret_rotated_at: Option<DateTime<Utc>>,
    pub enabled: bool,
    pub team_id: Option<TeamId>,
    pub all_public_teams: bool,
    pub resource_types: Vec<String>,
    pub creator_id: UserId,
    pub oauth_application_id: Option<Uuid>,
    pub consecutive_failures: i32,
    pub disabled_reason: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Neuer Webhook.
#[derive(Debug, Clone, PartialEq)]
pub struct NewWebhook {
    pub id: WebhookId,
    pub label: Option<String>,
    pub url: String,
    pub secret_enc: String,
    pub team_id: Option<TeamId>,
    pub all_public_teams: bool,
    pub resource_types: Vec<String>,
    pub creator_id: UserId,
    pub oauth_application_id: Option<Uuid>,
    pub now: DateTime<Utc>,
}

/// Änderung eines Webhooks (`None` = unverändert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WebhookPatch {
    pub label: Option<Option<String>>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub resource_types: Option<Vec<String>>,
    /// Neues Secret samt Vorgänger (Rotation).
    pub rotate_secret: Option<(String, String)>,
}

/// Aufgelöste Entität eines Events für die Zustellung.
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookEntity {
    /// Teams der Entität; leer = workspace-weit.
    pub team_ids: Vec<TeamId>,
    /// Serialisierte Entität (camelCase).
    pub data: Value,
    /// Alte Werte der zuletzt geänderten Felder (nur bei Updates, soweit bekannt).
    pub updated_from: Option<Value>,
    /// Pfad der App-URL (ohne Workspace-Präfix), z. B. `issue/ENG-1`.
    pub url_path: String,
}

/// Neue Zustellung (eine je Event × Webhook).
#[derive(Debug, Clone, PartialEq)]
pub struct NewDelivery {
    pub id: Uuid,
    pub webhook_id: WebhookId,
    pub event_id: Uuid,
    pub resource_type: String,
    pub action: String,
    /// Payload ohne `webhookTimestamp` und `webhookId`.
    pub payload: Value,
    pub now: DateTime<Utc>,
}

/// Eine fällige Zustellung samt Webhook (für den Versand).
#[derive(Debug, Clone, PartialEq)]
pub struct DueDelivery {
    pub id: Uuid,
    pub org_id: OrgId,
    pub event_id: Uuid,
    pub resource_type: String,
    pub action: String,
    /// Bisherige Versuche (vor diesem).
    pub attempt: i32,
    pub payload: Value,
    pub webhook: WebhookRecord,
}

/// Ergebnis eines Zustellversuchs.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryOutcome {
    pub success: bool,
    pub response_code: Option<i32>,
    pub duration_ms: i32,
    pub error: Option<String>,
    /// Zeitpunkt des nächsten Versuchs; `None` nach Erfolg oder nach dem letzten Retry.
    pub next_attempt_at: Option<DateTime<Utc>>,
}

/// Protokolleintrag einer Zustellung (Admin-Sicht, ohne Payload).
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryRecord {
    pub id: Uuid,
    pub webhook_id: WebhookId,
    pub event_id: Uuid,
    pub resource_type: String,
    pub action: String,
    pub attempt: i32,
    pub status: String,
    pub response_code: Option<i32>,
    pub duration_ms: Option<i32>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// OAuth
// ---------------------------------------------------------------------------

/// Dynamisch registrierter Client (global, RFC 7591).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientRegistration {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

/// Neuer Autorisierungscode (nur der Hash wird gespeichert).
#[derive(Debug, Clone, PartialEq)]
pub struct NewAuthCode {
    pub code_hash: String,
    pub application_id: Uuid,
    pub user_id: UserId,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub code_challenge: String,
    pub actor: String,
    pub family_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

/// Gespeicherter Autorisierungscode samt Client.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthCodeRecord {
    pub org_id: OrgId,
    pub application_id: Uuid,
    /// Öffentliche `client_id` (die der DCR-Registrierung).
    pub client_id: String,
    pub user_id: UserId,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub code_challenge: String,
    pub actor: String,
    pub family_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

/// Neues OAuth-Token-Paar (nur Hashes).
#[derive(Debug, Clone, PartialEq)]
pub struct NewOauthToken {
    pub id: Uuid,
    pub application_id: Uuid,
    pub user_id: UserId,
    pub actor: String,
    pub scopes: Vec<String>,
    pub access_token_hash: String,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_token_hash: String,
    pub family_id: Uuid,
    pub now: DateTime<Utc>,
}

/// Token-Zeile zu einem Refresh-Token (aktuell oder als Vorgänger).
#[derive(Debug, Clone, PartialEq)]
pub struct OauthTokenRecord {
    pub id: Uuid,
    pub org_id: OrgId,
    pub application_id: Uuid,
    pub client_id: String,
    pub user_id: UserId,
    pub actor: String,
    pub scopes: Vec<String>,
    pub refresh_token_hash: Option<String>,
    pub previous_refresh_token_hash: Option<String>,
    pub rotated_at: Option<DateTime<Utc>>,
    pub family_id: Uuid,
    pub revoked_at: Option<DateTime<Utc>>,
    /// Letzte Ausstellung/Rotation (Basis der Refresh-Lebensdauer).
    pub updated_at: DateTime<Utc>,
}

/// Rotation eines Token-Paars.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenRotation {
    pub access_token_hash: String,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_token_hash: String,
    pub now: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Einladungen
// ---------------------------------------------------------------------------

/// Einladung eines Workspace (ohne Token-Hash).
#[derive(Debug, Clone, PartialEq)]
pub struct InvitationRecord {
    pub id: Uuid,
    pub org_id: OrgId,
    pub email: String,
    pub role: UserRole,
    pub team_ids: Vec<TeamId>,
    pub inviter_id: UserId,
    /// `pending`, `accepted`, `revoked`, `expired`.
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub last_sent_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub accepted_user_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

/// Neue Einladung.
#[derive(Debug, Clone, PartialEq)]
pub struct NewInvitation {
    pub id: Uuid,
    pub email: String,
    pub role: UserRole,
    pub team_ids: Vec<TeamId>,
    pub inviter_id: UserId,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub now: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Persistenzzugriff des Pakets WP16.
#[async_trait]
pub trait WebhookRepo: Send {
    // --- Webhook-CRUD -----------------------------------------------------
    async fn insert_webhook(&mut self, _w: &NewWebhook) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Nicht archivierter Webhook des Workspace.
    async fn get_webhook(&mut self, _id: WebhookId) -> AppResult<Option<WebhookRecord>> {
        unimplemented_repo()
    }
    /// Alle nicht archivierten Webhooks (neueste zuerst).
    async fn list_webhooks(&mut self) -> AppResult<Vec<WebhookRecord>> {
        unimplemented_repo()
    }
    async fn update_webhook(
        &mut self,
        _id: WebhookId,
        _patch: &WebhookPatch,
        _now: DateTime<Utc>,
    ) -> AppResult<WebhookRecord> {
        unimplemented_repo()
    }
    /// Archiviert den Webhook (`archivedAt`); `false`, wenn er nicht existiert.
    async fn archive_webhook(&mut self, _id: WebhookId, _now: DateTime<Utc>) -> AppResult<bool> {
        unimplemented_repo()
    }

    // --- Zustellung -------------------------------------------------------
    /// Aktive Webhooks des Workspace, die `resource_type` abonniert haben.
    async fn list_dispatch_webhooks(
        &mut self,
        _resource_type: &str,
    ) -> AppResult<Vec<WebhookRecord>> {
        unimplemented_repo()
    }
    /// Lädt die Entität eines Events (`kind` = Ressourcentyp, z. B. `Issue`, `Comment`);
    /// `None`, wenn sie nicht (mehr) existiert. `hint_fields` sind die geänderten Felder (für
    /// `updatedFrom`).
    async fn load_webhook_entity(
        &mut self,
        _kind: &str,
        _id: Uuid,
        _hint_fields: &[String],
    ) -> AppResult<Option<WebhookEntity>> {
        unimplemented_repo()
    }
    /// Legt die Zustellung an; `false`, wenn es für (Webhook, Event) schon eine gibt.
    async fn insert_delivery(&mut self, _d: &NewDelivery) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Beansprucht fällige Zustellungen mandantenübergreifend (`SKIP LOCKED`, nur `begin_system`)
    /// und verschiebt `next_attempt_at` um `lease`, damit parallele Worker sie nicht doppelt senden.
    async fn claim_due_deliveries(
        &mut self,
        _now: DateTime<Utc>,
        _limit: i64,
        _lease: chrono::Duration,
    ) -> AppResult<Vec<DueDelivery>> {
        unimplemented_repo()
    }
    /// Protokolliert das Ergebnis eines Versuchs (Transaktion im Workspace der Zustellung).
    /// Pflegt Fehlerzähler und Deaktivierung; `give_up` markiert die Zustellung als aufgegeben.
    /// Liefert `true`, wenn der Webhook dabei deaktiviert wurde.
    async fn record_delivery_attempt(
        &mut self,
        _delivery: Uuid,
        _webhook: WebhookId,
        _outcome: &DeliveryOutcome,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Die letzten Zustellungen eines Webhooks (neueste zuerst).
    async fn list_deliveries(
        &mut self,
        _webhook: WebhookId,
        _limit: i64,
    ) -> AppResult<Vec<DeliveryRecord>> {
        unimplemented_repo()
    }
    /// Setzt eine Zustellung auf sofort fällig (erneut senden).
    async fn requeue_delivery(&mut self, _id: Uuid, _now: DateTime<Utc>) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Löscht Zustellprotokolle, die älter als `before` sind (30 Tage Aufbewahrung).
    async fn purge_deliveries(&mut self, _before: DateTime<Utc>) -> AppResult<u64> {
        unimplemented_repo()
    }
    /// Deaktiviert (`disabledReason = oauth_revoked`) alle Webhooks einer OAuth-Anwendung.
    async fn disable_webhooks_of_application(
        &mut self,
        _application: Uuid,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        unimplemented_repo()
    }

    // --- OAuth ------------------------------------------------------------
    async fn oauth_register_client(
        &mut self,
        _client: &ClientRegistration,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Registrierter Client (global, ohne Workspace-Bezug).
    async fn oauth_find_client(
        &mut self,
        _client_id: &str,
    ) -> AppResult<Option<ClientRegistration>> {
        unimplemented_repo()
    }
    /// Liefert die Workspace-Anwendung zu einem DCR-Client; legt sie beim ersten Consent an.
    async fn oauth_ensure_application(
        &mut self,
        _client: &ClientRegistration,
        _creator: UserId,
        _now: DateTime<Utc>,
    ) -> AppResult<Uuid> {
        unimplemented_repo()
    }
    async fn oauth_insert_code(&mut self, _code: &NewAuthCode) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Code zum Hash (mandantenübergreifend; nur `begin_system`).
    async fn oauth_find_code(&mut self, _code_hash: &str) -> AppResult<Option<AuthCodeRecord>> {
        unimplemented_repo()
    }
    /// Markiert den Code atomar als verwendet; `false`, wenn er schon verwendet war.
    async fn oauth_consume_code(
        &mut self,
        _code_hash: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    async fn oauth_insert_token(&mut self, _t: &NewOauthToken) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Widerruft alle Tokens einer Familie; Anzahl der betroffenen Zeilen.
    async fn oauth_revoke_family(&mut self, _family: Uuid, _now: DateTime<Utc>) -> AppResult<u64> {
        unimplemented_repo()
    }
    /// Token-Zeile, deren aktuelles oder vorheriges Refresh-Token `hash` ist (mandantenübergreifend).
    async fn oauth_find_by_refresh_hash(
        &mut self,
        _hash: &str,
    ) -> AppResult<Option<OauthTokenRecord>> {
        unimplemented_repo()
    }
    /// Rotiert das Paar einer Zeile (altes Refresh-Token wird `previous`, `rotatedAt = now`).
    /// Nur wirksam, wenn das aktuelle Refresh-Token noch `expected_current` ist; sonst `false`.
    async fn oauth_rotate_token(
        &mut self,
        _id: Uuid,
        _expected_current: &str,
        _rotation: &TokenRotation,
    ) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Widerruft die Token-Familie, zu der Access- oder Refresh-Hash gehört (mandantenübergreifend).
    /// Liefert Workspace und `client_id`, wenn etwas widerrufen wurde.
    async fn oauth_revoke_by_token_hash(
        &mut self,
        _hash: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<Option<(OrgId, String)>> {
        unimplemented_repo()
    }

    // --- Einladungen ------------------------------------------------------
    async fn insert_invitation(&mut self, _i: &NewInvitation) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get_invitation(&mut self, _id: Uuid) -> AppResult<Option<InvitationRecord>> {
        unimplemented_repo()
    }
    /// Einladungen des Workspace (neueste zuerst); `pending_only` blendet den Rest aus.
    async fn list_invitations(&mut self, _pending_only: bool) -> AppResult<Vec<InvitationRecord>> {
        unimplemented_repo()
    }
    /// Offene (`pending`, nicht abgelaufene) Einladung zu einer E-Mail.
    async fn find_pending_invitation(
        &mut self,
        _email: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<Option<InvitationRecord>> {
        unimplemented_repo()
    }
    /// Einladung zum Token-Hash (mandantenübergreifend; nur `begin_system`).
    async fn find_invitation_by_token_hash(
        &mut self,
        _hash: &str,
    ) -> AppResult<Option<InvitationRecord>> {
        unimplemented_repo()
    }
    /// Neues Token und neue Frist (resend).
    async fn renew_invitation(
        &mut self,
        _id: Uuid,
        _token_hash: &str,
        _expires_at: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn set_invitation_status(
        &mut self,
        _id: Uuid,
        _status: &str,
        _accepted_user: Option<UserId>,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Macht einen importierten Platzhalter-User zum aktiven User (Name, Passwort, Rolle).
    async fn activate_imported_user(
        &mut self,
        _user: UserId,
        _name: &str,
        _password_hash: Option<&str>,
        _role: UserRole,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Passwort-Hash eines aktiven Users mit dieser E-Mail in einem beliebigen Workspace
    /// (Konto-Session, nur `begin_system`).
    async fn password_hash_of_account(&mut self, _user: UserId) -> AppResult<Option<String>> {
        unimplemented_repo()
    }

    // --- Team-Lebenszyklus ------------------------------------------------
    /// Setzt `retiredAt` bzw. leert es.
    async fn set_team_retired(
        &mut self,
        _team: TeamId,
        _retired_at: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Legt das Team in den Papierkorb bzw. stellt es wieder her.
    async fn set_team_trashed(
        &mut self,
        _team: TeamId,
        _trashed: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Legt alle nicht gelöschten Projekte, an denen nur dieses Team beteiligt ist, in den
    /// Papierkorb (`trashedAt = now`); liefert ihre IDs.
    async fn trash_sole_projects(
        &mut self,
        _team: TeamId,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        unimplemented_repo()
    }
    /// Stellt die Projekte wieder her, die mit dem Team (`trashedAt == trashed_at`) gelöscht wurden.
    async fn restore_sole_projects(
        &mut self,
        _team: TeamId,
        _trashed_at: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        unimplemented_repo()
    }
    /// Nicht gelöschte Projekte mit mehreren Teams, an denen dieses Team beteiligt ist:
    /// `(project, lead_team)`.
    async fn shared_team_projects(&mut self, _team: TeamId) -> AppResult<Vec<(Uuid, TeamId)>> {
        unimplemented_repo()
    }
    /// Anzahl nicht gelöschter Issues je Team in einem Projekt.
    async fn project_issue_counts(&mut self, _project: Uuid) -> AppResult<Vec<(TeamId, i64)>> {
        unimplemented_repo()
    }
    /// Gelöschte Teams, deren Frist abgelaufen ist (mandantenübergreifend; nur `begin_system`).
    async fn list_teams_due_for_purge(
        &mut self,
        _before: DateTime<Utc>,
    ) -> AppResult<Vec<(OrgId, TeamId)>> {
        unimplemented_repo()
    }
    /// Löscht das Team samt Issues, Status, Labels, Cycles, Templates, Mitgliedschaften endgültig.
    async fn purge_team(&mut self, _team: TeamId) -> AppResult<()> {
        unimplemented_repo()
    }
}
