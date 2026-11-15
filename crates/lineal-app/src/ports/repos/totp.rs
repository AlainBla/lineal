//! Repository-Port `TotpRepo` (LIN-70): TOTP-Secret, Recovery-Codes und
//! Schonfrist je Nutzer. Default-Bodies liefern „not implemented", damit
//! Fake/PG nur die tatsächlich genutzten Methoden überschreiben müssen.

use crate::error::{AppError, AppResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::ids::{OrgId, UserId};
use uuid::Uuid;

fn unimplemented_repo<T>() -> AppResult<T> {
    Err(AppError::internal("not implemented"))
}

/// Gespeicherter TOTP-Zustand eines Nutzers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTotp {
    pub user_id: UserId,
    pub secret_enc: Vec<u8>,
    pub dek_wrapped: Vec<u8>,
    pub key_backend: String,
    pub key_version: i32,
    /// `None` = angefangen, aber noch nicht aktiviert.
    pub confirmed_at: Option<DateTime<Utc>>,
    /// Zuletzt akzeptierter Zeitschritt (Replay-Schutz).
    pub last_used_step: Option<i64>,
}

impl UserTotp {
    /// Aktiv = Enrollment bestätigt.
    pub fn is_active(&self) -> bool {
        self.confirmed_at.is_some()
    }
}

/// Neues (zunächst unbestätigtes) TOTP-Secret.
#[derive(Debug, Clone)]
pub struct NewUserTotp {
    pub user_id: UserId,
    pub org_id: OrgId,
    pub secret_enc: Vec<u8>,
    pub dek_wrapped: Vec<u8>,
    pub key_backend: String,
    pub key_version: i32,
    pub now: DateTime<Utc>,
}

/// Ein noch nicht verbrauchter Recovery-Code (nur der Hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCodeRow {
    pub id: Uuid,
    pub code_hash: Vec<u8>,
}

/// Schonfrist-Zustand eines Owners ohne bestätigtes TOTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraceRow {
    pub started_at: DateTime<Utc>,
    pub logins_since: i32,
}

#[async_trait]
pub trait TotpRepo: Send {
    /// TOTP-Zustand eines Nutzers (auch unbestätigt).
    async fn get(&mut self, _user: UserId) -> AppResult<Option<UserTotp>> {
        unimplemented_repo()
    }
    /// Legt ein unbestätigtes Secret an bzw. ersetzt ein vorhandenes (Neu-Enrollment).
    async fn upsert_pending(&mut self, _rec: &NewUserTotp) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Markiert das Secret als bestätigt (aktiv).
    async fn confirm(&mut self, _user: UserId, _now: DateTime<Utc>) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Hält den zuletzt benutzten Zeitschritt fest (Replay-Schutz).
    async fn set_last_used_step(&mut self, _user: UserId, _step: i64) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Entfernt TOTP, Recovery-Codes und Schonfrist eines Nutzers (Disable/Reset).
    async fn delete_all(&mut self, _user: UserId) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Ersetzt den kompletten Recovery-Code-Satz.
    async fn replace_recovery_codes(
        &mut self,
        _user: UserId,
        _org: OrgId,
        _hashes: &[Vec<u8>],
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Noch unbenutzte Recovery-Codes eines Nutzers.
    async fn unused_recovery_codes(&mut self, _user: UserId) -> AppResult<Vec<RecoveryCodeRow>> {
        unimplemented_repo()
    }
    /// Zählt noch unbenutzte Recovery-Codes.
    async fn unused_recovery_count(&mut self, user: UserId) -> AppResult<i64> {
        Ok(self.unused_recovery_codes(user).await?.len() as i64)
    }
    /// Markiert einen Recovery-Code als verbraucht.
    async fn consume_recovery_code(&mut self, _id: Uuid, _now: DateTime<Utc>) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Schonfrist-Zeile eines Nutzers.
    async fn grace_get(&mut self, _user: UserId) -> AppResult<Option<GraceRow>> {
        unimplemented_repo()
    }
    /// Legt die Schonfrist bei Bedarf an und erhöht `logins_since`; gibt den neuen Stand zurück.
    async fn grace_bump(
        &mut self,
        _user: UserId,
        _org: OrgId,
        _now: DateTime<Utc>,
    ) -> AppResult<GraceRow> {
        unimplemented_repo()
    }
    /// Löscht die Schonfrist (nach erfolgreichem Enrollment).
    async fn grace_clear(&mut self, _user: UserId) -> AppResult<()> {
        unimplemented_repo()
    }
    /// Owner (system-weit), deren Schonfrist abgelaufen ist und die kein bestätigtes
    /// TOTP haben — für den Enforcement-Job (Session-Widerruf).
    async fn owners_past_deadline(
        &mut self,
        _now: DateTime<Utc>,
        _max_age_days: i64,
        _max_logins: i32,
    ) -> AppResult<Vec<UserId>> {
        unimplemented_repo()
    }
}
