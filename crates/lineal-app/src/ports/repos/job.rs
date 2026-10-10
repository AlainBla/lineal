//! Repository-Port `JobRepo` (Tabelle `job`, `FOR UPDATE SKIP LOCKED`).

use crate::error::AppResult;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_domain::ids::{JobId, OrgId};
use serde_json::Value;

/// Neuer Job.
#[derive(Debug, Clone, PartialEq)]
pub struct NewJob {
    pub kind: String,
    pub payload: Value,
    pub org_id: Option<OrgId>,
    pub run_at: DateTime<Utc>,
    pub max_attempts: i32,
    /// Idempotenzschlüssel: ein zweiter Job mit demselben Schlüssel wird nicht angelegt.
    pub unique_key: Option<String>,
}

impl NewJob {
    /// Sofort fälliger Job ohne Schlüssel.
    pub fn now(kind: impl Into<String>, payload: Value, now: DateTime<Utc>) -> Self {
        Self { kind: kind.into(), payload, org_id: None, run_at: now, max_attempts: 5, unique_key: None }
    }
}

/// Beanspruchter Job.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: JobId,
    pub org_id: Option<OrgId>,
    pub kind: String,
    pub payload: Value,
    pub attempts: i32,
    pub max_attempts: i32,
}

#[async_trait]
pub trait JobRepo: Send {
    /// Legt einen Job an; `None`, wenn `unique_key` bereits existiert.
    async fn enqueue(&mut self, job: NewJob) -> AppResult<Option<JobId>>;
    /// Beansprucht bis zu `limit` fällige Jobs (`SKIP LOCKED`), setzt sie auf `running`
    /// und erhöht `attempts`. Hängende `running`-Jobs (> 15 min) werden erneut vergeben.
    async fn claim(&mut self, worker: &str, limit: i64, now: DateTime<Utc>) -> AppResult<Vec<Job>>;
    /// Markiert einen Job als erledigt.
    async fn complete(&mut self, id: JobId, now: DateTime<Utc>) -> AppResult<()>;
    /// Meldet einen Fehler. Mit `retry_at` (und `attempts < max_attempts`) wird der Job neu
    /// eingeplant, sonst endgültig `failed`.
    async fn fail(&mut self, id: JobId, error: &str, retry_at: Option<DateTime<Utc>>) -> AppResult<()>;
}
