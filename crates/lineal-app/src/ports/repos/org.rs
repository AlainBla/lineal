//! Repository-Port `OrgRepo`: Workspace (`Organization`), Settings und Seed-Daten beim Anlegen.
//!
//! Alle Methoden haben Default-Implementierungen (`not implemented`), damit Fakes ohne
//! Datenbank (`lineal-testkit::FakeTx`) kompilieren.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::ProjectStatusType;
use lineal_domain::ids::OrgId;
use serde_json::Value;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Ein Workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Org {
    pub id: OrgId,
    pub name: String,
    pub url_key: String,
    pub previous_url_keys: Vec<String>,
    pub logo_url: Option<String>,
    pub fiscal_year_start_month: i32,
    pub initiatives_enabled: bool,
    /// `securitySettings` (Mindestrollen, docs/02 §6.3).
    pub security_settings: Value,
    /// `limits` (docs/02 §1.3).
    pub limits: Value,
    pub deletion_requested_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Neuer Workspace (die Transaktion muss mit `begin(org.id)` geöffnet sein).
#[derive(Debug, Clone, PartialEq)]
pub struct NewOrg {
    pub id: OrgId,
    pub name: String,
    pub url_key: String,
    pub security_settings: Value,
    pub limits: Value,
    pub now: DateTime<Utc>,
}

/// Änderung der Workspace-Settings (`None` = unverändert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OrgPatch {
    pub name: Option<String>,
    /// Neuer `urlKey`; der alte wandert nach `previousUrlKeys` (max. 3).
    pub url_key: Option<String>,
    pub logo_url: Option<Option<String>>,
    pub fiscal_year_start_month: Option<i32>,
    pub initiatives_enabled: Option<bool>,
    pub security_settings: Option<Value>,
    pub limits: Option<Value>,
    pub now: Option<DateTime<Utc>>,
}

/// Workspace-weiter Projektstatus (docs/04 §2.3).
#[derive(Debug, Clone, PartialEq)]
pub struct NewProjectStatus {
    pub id: Uuid,
    pub name: String,
    pub status_type: ProjectStatusType,
    pub color: String,
    pub position: f64,
    pub indefinite: bool,
    pub now: DateTime<Utc>,
}

/// Workspace-Label (`team_id = null`).
#[derive(Debug, Clone, PartialEq)]
pub struct NewWorkspaceLabel {
    pub id: Uuid,
    pub name: String,
    pub color: String,
    pub creator_id: Option<Uuid>,
    pub now: DateTime<Utc>,
}

/// Persistenzzugriff des Bereichs `org`.
#[async_trait]
pub trait OrgRepo: Send {
    /// Legt den Workspace an (`tx.org_id()` muss `org.id` sein).
    async fn insert(&mut self, _org: &NewOrg) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Der Workspace dieser Transaktion.
    async fn get(&mut self) -> AppResult<Option<Org>> {
        unimplemented_repo()
    }
    /// Ob `url_key` global (über alle Workspaces, aktuelle und frühere Keys) vergeben ist.
    async fn url_key_taken(&mut self, _url_key: &str) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Aktualisiert die Settings; liefert den neuen Stand.
    async fn update(&mut self, _patch: &OrgPatch) -> AppResult<Org> {
        unimplemented_repo()
    }
    /// Volltextkonfiguration des Workspace (`simple`, `german`, `english`).
    async fn fulltext_config(&mut self) -> AppResult<String> {
        unimplemented_repo()
    }
    /// Setzt die Volltextkonfiguration; ein Trigger baut die Suchvektoren des Workspace neu auf.
    async fn set_fulltext_config(&mut self, _config: &str, _now: DateTime<Utc>) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Legt einen Workspace-Projektstatus an.
    async fn insert_project_status(&mut self, _status: &NewProjectStatus) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Legt ein Workspace-Label an.
    async fn insert_workspace_label(&mut self, _label: &NewWorkspaceLabel) -> AppResult<()> {
        unimplemented_repo()
    }
}
