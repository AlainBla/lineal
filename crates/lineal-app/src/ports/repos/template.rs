//! Repository-Port `TemplateRepo`: Vorlagen für Issues, Projekte und Dokumente (WP10, docs/06 §2).

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::TemplateType;
use lineal_domain::ids::*;
use serde_json::Value;
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Eine Vorlage. `team_id = None` ist ein Workspace-Template.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub id: TemplateId,
    pub template_type: TemplateType,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub team_id: Option<TeamId>,
    pub template_data: Value,
    pub sort_order: f64,
    pub inherited_from_id: Option<TemplateId>,
    pub creator_id: Option<UserId>,
    pub last_updated_by_id: Option<UserId>,
    pub last_applied_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

/// Neue Vorlage.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTemplate {
    pub id: TemplateId,
    pub template_type: TemplateType,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub team_id: Option<TeamId>,
    pub template_data: Value,
    pub sort_order: f64,
    pub inherited_from_id: Option<TemplateId>,
    pub creator_id: Option<UserId>,
    pub now: DateTime<Utc>,
}

/// Welches Default-Template eines Teams gemeint ist (docs/06 §2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultSlot {
    /// `defaultTemplateForMembers` (Issue).
    Members,
    /// `defaultTemplateForNonMembers` (Issue).
    NonMembers,
    /// `defaultProjectTemplate` (Projekt).
    Project,
}

/// Die drei Default-Templates eines Teams.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TeamDefaults {
    pub members: Option<TemplateId>,
    pub non_members: Option<TemplateId>,
    pub project: Option<TemplateId>,
}

/// Persistenzzugriff des Bereichs `template`. Alle Abfragen laufen im Workspace der Transaktion.
#[async_trait]
pub trait TemplateRepo: Send {
    async fn insert(&mut self, t: &NewTemplate) -> AppResult<Template> {
        let _ = t;
        unimplemented_repo()
    }

    /// Per UUID, auch archiviert.
    async fn get(&mut self, id: TemplateId) -> AppResult<Option<Template>> {
        let _ = id;
        unimplemented_repo()
    }

    /// Schreibt Name, Beschreibung, Icon, Farbe, `templateData` und Sortierung (Typ und Team sind
    /// unveränderlich); erhöht `version`.
    async fn update(
        &mut self,
        t: &Template,
        updated_by: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<Template> {
        let _ = (t, updated_by, now);
        unimplemented_repo()
    }

    /// Archiviert die Vorlage und setzt Verweise auf sie (Team-Defaults) auf `null`.
    async fn archive(&mut self, id: TemplateId, now: DateTime<Utc>) -> AppResult<()> {
        let _ = (id, now);
        unimplemented_repo()
    }

    /// Alle nicht archivierten Vorlagen, nach `sort_order`, `name`. Sichtbarkeit prüft der Use-Case.
    async fn list_active(&mut self) -> AppResult<Vec<Template>> {
        unimplemented_repo()
    }

    /// Nicht archivierte Vorlage gleichen Namens (case-insensitiv) im Geltungsbereich und Typ.
    async fn find_by_name(
        &mut self,
        team: Option<TeamId>,
        template_type: TemplateType,
        name: &str,
    ) -> AppResult<Option<Template>> {
        let _ = (team, template_type, name);
        unimplemented_repo()
    }

    /// Alle nicht archivierten Vorlagen mit diesem Namen (über alle Geltungsbereiche und Typen).
    async fn find_all_by_name(&mut self, name: &str) -> AppResult<Vec<Template>> {
        let _ = name;
        unimplemented_repo()
    }

    /// Vermerkt `lastAppliedAt`.
    async fn mark_applied(&mut self, id: TemplateId, now: DateTime<Utc>) -> AppResult<()> {
        let _ = (id, now);
        unimplemented_repo()
    }

    /// Setzt `lastAppliedTemplate` am erzeugten Objekt (Issue, Projekt oder Dokument).
    async fn mark_object(
        &mut self,
        template_type: TemplateType,
        object: Uuid,
        template: TemplateId,
    ) -> AppResult<()> {
        let _ = (template_type, object, template);
        unimplemented_repo()
    }

    async fn team_defaults(&mut self, team: TeamId) -> AppResult<TeamDefaults> {
        let _ = team;
        unimplemented_repo()
    }

    async fn set_team_default(
        &mut self,
        team: TeamId,
        slot: DefaultSlot,
        template: Option<TemplateId>,
    ) -> AppResult<()> {
        let _ = (team, slot, template);
        unimplemented_repo()
    }
}
