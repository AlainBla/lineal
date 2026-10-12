//! Repository-Port `UserRepo`: Benutzer eines Workspace.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, TeamId, UserId};
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Sortierung von Listen (docs/00 §5), immer absteigend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderBy {
    CreatedAt,
    #[default]
    UpdatedAt,
}

/// Position in einer Liste: Sortschlüssel (Zeitpunkt) und ID des letzten Eintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCursor {
    pub key: DateTime<Utc>,
    pub id: Uuid,
}

/// Seitenanfrage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    /// 1..=250 (der Aufrufer validiert).
    pub limit: i64,
    pub cursor: Option<PageCursor>,
    pub order_by: OrderBy,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            limit: 50,
            cursor: None,
            order_by: OrderBy::UpdatedAt,
        }
    }
}

/// Eine Seite; `next` ist der Cursor des letzten Eintrags, falls es weitere gibt.
#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<PageCursor>,
}

impl<T> Page<T> {
    pub fn has_next_page(&self) -> bool {
        self.next.is_some()
    }
}

/// Ein Benutzer (ohne Passwort-Hash).
#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: UserId,
    pub org_id: OrgId,
    pub name: String,
    /// Handle für `@`-Erwähnungen (`^[a-z0-9._-]{1,40}$`).
    pub display_name: String,
    pub email: String,
    pub role: UserRole,
    pub active: bool,
    /// `active` oder `imported` (docs/13).
    pub status: String,
    pub avatar_url: Option<String>,
    pub timezone: Option<String>,
    pub last_seen: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Neuer Benutzer.
#[derive(Debug, Clone, PartialEq)]
pub struct NewUser {
    pub id: UserId,
    pub name: String,
    pub display_name: String,
    pub email: String,
    pub role: UserRole,
    /// Argon2-PHC-Hash; `None` = kein Passwort-Login.
    pub password_hash: Option<String>,
    pub now: DateTime<Utc>,
}

/// Filter der Benutzerliste.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserFilter {
    /// Teilstring in Name, Anzeigename oder E-Mail (case-insensitiv).
    pub query: Option<String>,
    /// Nur Mitglieder dieses Teams.
    pub team_id: Option<TeamId>,
    /// Nur User, die in mindestens einem dieser Teams Mitglied sind (Gast-Sicht).
    pub in_any_team: Option<Vec<TeamId>>,
    /// Auch suspendierte User liefern (nur für die Admin-Mitgliederliste).
    pub include_suspended: bool,
}

/// Persistenzzugriff des Bereichs `user`.
#[async_trait]
pub trait UserRepo: Send {
    async fn insert(&mut self, _user: &NewUser) -> AppResult<()> {
        unimplemented_repo()
    }
    async fn get(&mut self, _id: UserId) -> AppResult<Option<User>> {
        unimplemented_repo()
    }
    /// Exakte E-Mail (case-insensitiv), auch deaktivierte User.
    async fn find_by_email(&mut self, _email: &str) -> AppResult<Option<User>> {
        unimplemented_repo()
    }
    /// Alle aktiven User, deren `name` oder `displayName` case-insensitiv exakt `name` ist.
    async fn find_by_name(&mut self, _name: &str) -> AppResult<Vec<User>> {
        unimplemented_repo()
    }
    /// Ob der Anzeigename im Workspace vergeben ist.
    async fn display_name_taken(&mut self, _display_name: &str) -> AppResult<bool> {
        unimplemented_repo()
    }
    /// Aktive User, gefiltert und paginiert. Ein `query`, das exakt eine E-Mail trifft,
    /// liefert zusätzlich den deaktivierten User mit dieser E-Mail.
    async fn list(&mut self, _filter: &UserFilter, _page: &PageRequest) -> AppResult<Page<User>> {
        unimplemented_repo()
    }
    /// Setzt den Passwort-Hash.
    async fn set_password_hash(
        &mut self,
        _id: UserId,
        _hash: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Ändert die Rolle (Rechteprüfung im Use-Case).
    async fn set_role(
        &mut self,
        _id: UserId,
        _role: UserRole,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Suspendiert bzw. reaktiviert.
    async fn set_active(
        &mut self,
        _id: UserId,
        _active: bool,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
}
