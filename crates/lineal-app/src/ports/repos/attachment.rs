//! Repository-Port `AttachmentRepo`: Issue-Anhänge und Upload-Reservierungen (WP09).

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::ids::*;
use serde_json::Value;
use uuid::Uuid;

/// Kurzform eines Issues für Zugriffsprüfung und Anzeige.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    pub id: IssueId,
    pub team_id: TeamId,
    pub identifier: String,
}

/// Ein Anhang.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub id: AttachmentId,
    pub issue_id: IssueId,
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// `url` | `upload`
    pub source_type: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    pub storage_key: Option<String>,
    pub metadata: Value,
    pub creator_id: Option<UserId>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

/// Link-Anhang (Upsert pro Issue und URL).
#[derive(Debug, Clone, PartialEq)]
pub struct NewLink {
    pub id: AttachmentId,
    pub issue_id: IssueId,
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub metadata: Option<Value>,
    pub creator_id: Option<UserId>,
    pub now: DateTime<Utc>,
}

/// Hochgeladene Datei als Anhang.
#[derive(Debug, Clone, PartialEq)]
pub struct NewUpload {
    pub id: AttachmentId,
    pub issue_id: IssueId,
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub content_type: String,
    pub size: i64,
    pub storage_key: String,
    pub creator_id: Option<UserId>,
    pub now: DateTime<Utc>,
}

/// Reservierung eines Uploads (`prepare`).
#[derive(Debug, Clone, PartialEq)]
pub struct Reservation {
    pub id: Uuid,
    pub issue_id: Option<IssueId>,
    pub creator_id: Option<UserId>,
    pub storage_key: String,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub finalized_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

fn not_implemented<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Persistenzzugriff des Bereichs `attachment`.
#[async_trait]
#[allow(unused_variables)]
pub trait AttachmentRepo: Send {
    /// Issue per UUID, Kennung (auch frühere) auflösen.
    async fn find_issue(&mut self, reference: &str) -> AppResult<Option<IssueRef>> {
        not_implemented()
    }
    async fn issue_ref(&mut self, id: IssueId) -> AppResult<Option<IssueRef>> {
        not_implemented()
    }

    /// Legt den Link an oder aktualisiert ihn (`true` = neu angelegt bzw. wiederhergestellt).
    async fn upsert_link(&mut self, l: &NewLink) -> AppResult<(Attachment, bool)> {
        not_implemented()
    }
    async fn insert_upload(&mut self, u: &NewUpload) -> AppResult<Attachment> {
        not_implemented()
    }
    /// Nicht archivierter Anhang.
    async fn get(&mut self, id: AttachmentId) -> AppResult<Option<Attachment>> {
        not_implemented()
    }
    /// Nicht archivierter Anhang zu diesem Objektschlüssel.
    async fn find_by_storage_key(&mut self, key: &str) -> AppResult<Option<Attachment>> {
        not_implemented()
    }
    async fn list_for_issue(&mut self, issue: IssueId) -> AppResult<Vec<Attachment>> {
        not_implemented()
    }
    async fn archive(&mut self, id: AttachmentId, now: DateTime<Utc>) -> AppResult<()> {
        not_implemented()
    }
    /// Eintrag im Issue-Verlauf (`attachmentAdded` / `attachmentRemoved`).
    async fn add_history(
        &mut self,
        issue: IssueId,
        actor: Option<UserId>,
        attachment: AttachmentId,
        flag: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        not_implemented()
    }

    async fn insert_reservation(&mut self, r: &Reservation) -> AppResult<()> {
        not_implemented()
    }
    async fn reservation_by_key(&mut self, key: &str) -> AppResult<Option<Reservation>> {
        not_implemented()
    }
    /// Markiert die Reservierung als finalisiert; `false`, wenn sie es schon war.
    async fn finalize_reservation(&mut self, id: Uuid, now: DateTime<Utc>) -> AppResult<bool> {
        not_implemented()
    }

    /// Objektschlüssel aller Anhänge und Reservierungen dieses Issues (auch archivierte).
    async fn storage_keys_for_issue(&mut self, issue: IssueId) -> AppResult<Vec<String>> {
        not_implemented()
    }
    /// Objektschlüssel aller Anhänge und Reservierungen der Issues dieses Teams.
    async fn storage_keys_for_team(&mut self, team: TeamId) -> AppResult<Vec<String>> {
        not_implemented()
    }
    /// Teilmenge von `keys`, die weder ein Anhang noch eine Reservierung (irgendeines
    /// Workspace) noch referenziert; nach dem Löschen der Zeilen aufrufen.
    async fn unreferenced_storage_keys(&mut self, keys: &[String]) -> AppResult<Vec<String>> {
        not_implemented()
    }
    /// Nicht finalisierte Reservierungen, die vor `cutoff` abgelaufen sind (alle Workspaces;
    /// nur in System-Transaktionen).
    async fn list_expired_reservations(
        &mut self,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<(OrgId, Reservation)>> {
        not_implemented()
    }
    /// Löscht Reservierungen dieses Workspace.
    async fn delete_reservations(&mut self, ids: &[Uuid]) -> AppResult<()> {
        not_implemented()
    }
}
