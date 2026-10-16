//! `ImportRepo` für `PgTx` (WP18, docs/13).
//!
//! Zeilen werden generisch über `jsonb_populate_record` geschrieben: Die Spaltenliste stammt aus
//! den Schlüsseln des übergebenen Objekts (nur vom Import-Code erzeugt, zusätzlich gegen
//! `^[a-z][a-z0-9_]*$` geprüft und gequotet); alle nicht genannten Spalten behalten ihren
//! Default. Jede Abfrage ist auf `organization_id` der Transaktion begrenzt (außer den
//! ausdrücklich mandantenübergreifenden Runner-Methoden).

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::ImportRepo;
use lineal_app::ports::repos::import::{
    ImportCounter, ImportRun, ImportRunUpdate, ImportSyncRow, ImportTable, MappingRow,
    NewImportRun, NewMapping, ReportEntry, UserMergeStats,
};
use lineal_app::{AppError, AppResult};
use serde_json::{Map, Value, json};
use sqlx::Row;
use uuid::Uuid;

const RUN_COLS: &str = "id, organization_id, source, status, options, checkpoint, stats, started_by, \
    started_at, finished_at, error, version, created_at, updated_at";

#[derive(sqlx::FromRow)]
struct RunRow {
    id: Uuid,
    organization_id: Uuid,
    source: String,
    status: String,
    options: Value,
    checkpoint: Value,
    stats: Value,
    started_by: Option<Uuid>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    error: Option<String>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<RunRow> for ImportRun {
    fn from(r: RunRow) -> Self {
        Self {
            id: r.id,
            org_id: r.organization_id,
            source: r.source,
            status: r.status,
            options: r.options,
            checkpoint: r.checkpoint,
            stats: r.stats,
            started_by: r.started_by,
            started_at: r.started_at,
            finished_at: r.finished_at,
            error: r.error,
            version: r.version,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct MappingDbRow {
    import_run_id: Uuid,
    source: String,
    source_type: String,
    source_id: String,
    target_id: Uuid,
    source_updated_at: Option<DateTime<Utc>>,
    checksum: Option<String>,
    imported_at: DateTime<Utc>,
}

impl From<MappingDbRow> for MappingRow {
    fn from(r: MappingDbRow) -> Self {
        Self {
            import_run_id: r.import_run_id,
            source: r.source,
            source_type: r.source_type,
            source_id: r.source_id,
            target_id: r.target_id,
            source_updated_at: r.source_updated_at,
            checksum: r.checksum,
            imported_at: r.imported_at,
        }
    }
}

const MAPPING_COLS: &str = "import_run_id, source, source_type, source_id, target_id, source_updated_at, checksum, imported_at";

fn check_ident(name: &str) -> AppResult<()> {
    let mut chars = name.chars();
    let ok = matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(AppError::internal("invalid column name in import row"))
    }
}

/// Spaltenliste (gequotet) und Objekt aus einem JSON-Objekt; ergänzt `organization_id`.
fn prepare_row(org: Uuid, row: &Value, with_org: bool) -> AppResult<(Vec<String>, Value)> {
    let Value::Object(map) = row else {
        return Err(AppError::internal("import row must be an object"));
    };
    let mut obj: Map<String, Value> = map.clone();
    if with_org {
        obj.insert("organization_id".into(), json!(org));
    }
    let mut cols = Vec::with_capacity(obj.len());
    for key in obj.keys() {
        check_ident(key)?;
        cols.push(format!("\"{key}\""));
    }
    Ok((cols, Value::Object(obj)))
}

/// Eine vom Schema abgelehnte Zeile (Eindeutigkeit, Referenz, Check) → `conflict`; Details im Log.
const SYNC_COLS: &str = "id, organization_id, interval, daily_at_minute, paused, teams, projects, \
    target_teams, user_mapping, key_mapping, created_by, last_run_id, last_run_at, last_success_at, \
    last_status, last_error, last_removal_check_at, next_run_at, created_at, updated_at";

#[derive(sqlx::FromRow)]
struct SyncRowDb {
    id: Uuid,
    organization_id: Uuid,
    interval: String,
    daily_at_minute: i32,
    paused: bool,
    teams: Vec<String>,
    projects: Vec<String>,
    target_teams: Value,
    user_mapping: Value,
    key_mapping: Value,
    created_by: Option<Uuid>,
    last_run_id: Option<Uuid>,
    last_run_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    last_status: Option<String>,
    last_error: Option<String>,
    last_removal_check_at: Option<DateTime<Utc>>,
    next_run_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<SyncRowDb> for ImportSyncRow {
    fn from(r: SyncRowDb) -> Self {
        Self {
            id: r.id,
            org_id: r.organization_id,
            interval: r.interval,
            daily_at_minute: r.daily_at_minute,
            paused: r.paused,
            teams: r.teams,
            projects: r.projects,
            target_teams: r.target_teams,
            user_mapping: r.user_mapping,
            key_mapping: r.key_mapping,
            created_by: r.created_by,
            last_run_id: r.last_run_id,
            last_run_at: r.last_run_at,
            last_success_at: r.last_success_at,
            last_status: r.last_status,
            last_error: r.last_error,
            last_removal_check_at: r.last_removal_check_at,
            next_run_at: r.next_run_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Unique- oder Check-Verletzung (23505/23514): die Zeile würde beim Ziel doppelt vorkommen.
fn is_constraint_clash(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505" || c == "23514")
}

fn row_rejected(e: &sqlx::Error) -> AppError {
    // Der Constraint-Name ist kein Nutzerinhalt und steht im Bericht; die Details nur im Debug-Log.
    tracing::debug!(error = %e, "import row rejected by the database");
    let constraint = e
        .as_database_error()
        .and_then(|d| d.constraint())
        .map(str::to_owned);
    match constraint {
        Some(c) => AppError::conflict(format!(
            "the row was rejected by the database (constraint {c})"
        )),
        None => AppError::conflict("the row was rejected by the database"),
    }
}

/// Nur die Tabellen mit `organization_id`-Spalte (alle der Whitelist).
fn run_select(extra: &str) -> String {
    format!("SELECT {RUN_COLS} FROM import_run WHERE organization_id = $1 {extra}")
}

#[async_trait]
impl ImportRepo for PgTx {
    async fn create_import_run(&mut self, run: &NewImportRun) -> AppResult<ImportRun> {
        let row: RunRow = sqlx::query_as(&format!(
            "INSERT INTO import_run (id, organization_id, source, status, options, started_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7) RETURNING {RUN_COLS}"
        ))
        .bind(run.id)
        .bind(self.org.0)
        .bind(&run.source)
        .bind(&run.status)
        .bind(&run.options)
        .bind(run.started_by)
        .bind(run.now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.into())
    }

    async fn get_import_run(&mut self, id: Uuid) -> AppResult<Option<ImportRun>> {
        let row: Option<RunRow> = sqlx::query_as(&run_select("AND id = $2"))
            .bind(self.org.0)
            .bind(id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn list_import_runs(&mut self, limit: i64) -> AppResult<Vec<ImportRun>> {
        let rows: Vec<RunRow> = sqlx::query_as(&run_select(
            "AND archived_at IS NULL ORDER BY created_at DESC, id LIMIT $2",
        ))
        .bind(self.org.0)
        .bind(limit.clamp(1, 200))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_import_run(&mut self, id: Uuid, u: &ImportRunUpdate) -> AppResult<()> {
        let now = u.now.unwrap_or_else(Utc::now);
        sqlx::query(
            "UPDATE import_run SET \
               status = COALESCE($3, status), \
               options = COALESCE($4, options), \
               checkpoint = COALESCE($5, checkpoint), \
               stats = COALESCE($6, stats), \
               started_at = CASE WHEN $7 THEN $8 ELSE started_at END, \
               finished_at = CASE WHEN $9 THEN $10 ELSE finished_at END, \
               error = CASE WHEN $11 THEN $12 ELSE error END, \
               version = version + 1, updated_at = $13 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(u.status.as_deref())
        .bind(u.options.as_ref())
        .bind(u.checkpoint.as_ref())
        .bind(u.stats.as_ref())
        .bind(u.started_at.is_some())
        .bind(u.started_at.flatten())
        .bind(u.finished_at.is_some())
        .bind(u.finished_at.flatten())
        .bind(u.error.is_some())
        .bind(u.error.clone().flatten())
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn latest_completed_import_run(&mut self, source: &str) -> AppResult<Option<ImportRun>> {
        let row: Option<RunRow> = sqlx::query_as(&run_select(
            "AND source = $2 AND status = 'completed' AND COALESCE((options->>'dryRun')::boolean, false) = false \
             ORDER BY started_at DESC NULLS LAST LIMIT 1",
        ))
        .bind(self.org.0)
        .bind(source)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn claim_runnable_import(
        &mut self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> AppResult<Option<ImportRun>> {
        let row: Option<RunRow> = sqlx::query_as(&format!(
            "UPDATE import_run SET lease_until = $2 WHERE id = ( \
               SELECT id FROM import_run \
               WHERE status IN ('scanning', 'running') AND archived_at IS NULL AND source = 'linear' \
                 AND (lease_until IS NULL OR lease_until < $1) \
               ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED) \
             RETURNING {RUN_COLS}"
        ))
        .bind(now)
        .bind(lease_until)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn fail_stale_import_runs(
        &mut self,
        now: DateTime<Utc>,
        stale_before: DateTime<Utc>,
    ) -> AppResult<u64> {
        let res = sqlx::query(
            "UPDATE import_run SET status = 'failed', lease_until = NULL, finished_at = $1, \
               error = 'the run was interrupted (process ended); resume or cancel it', \
               updated_at = $1, version = version + 1 \
             WHERE status = 'running' AND archived_at IS NULL AND source <> 'linear' \
               AND COALESCE(lease_until, updated_at) < $2",
        )
        .bind(now)
        .bind(stale_before)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn set_import_lease(
        &mut self,
        id: Uuid,
        lease_until: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE import_run SET lease_until = $3 WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(lease_until)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn put_import_credential(
        &mut self,
        run_id: Uuid,
        encrypted: &[u8],
        expires_at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO import_credential (organization_id, import_run_id, encrypted_token, expires_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (import_run_id) DO UPDATE SET encrypted_token = EXCLUDED.encrypted_token, \
               expires_at = EXCLUDED.expires_at",
        )
        .bind(self.org.0)
        .bind(run_id)
        .bind(encrypted)
        .bind(expires_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_import_credential(&mut self, run_id: Uuid) -> AppResult<Option<Vec<u8>>> {
        sqlx::query_scalar(
            "SELECT encrypted_token FROM import_credential WHERE organization_id = $1 AND import_run_id = $2",
        )
        .bind(self.org.0)
        .bind(run_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn delete_import_credential(&mut self, run_id: Uuid) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM import_credential WHERE organization_id = $1 AND import_run_id = $2",
        )
        .bind(self.org.0)
        .bind(run_id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_expired_import_credentials(&mut self, now: DateTime<Utc>) -> AppResult<u64> {
        let r = sqlx::query("DELETE FROM import_credential WHERE expires_at < $1")
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(r.rows_affected())
    }

    async fn get_mappings(
        &mut self,
        source: &str,
        source_type: &str,
        source_ids: &[String],
    ) -> AppResult<Vec<MappingRow>> {
        if source_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows: Vec<MappingDbRow> = sqlx::query_as(&format!(
            "SELECT {MAPPING_COLS} FROM import_mapping \
             WHERE organization_id = $1 AND source = $2 AND source_type = $3 AND source_id = ANY($4)"
        ))
        .bind(self.org.0)
        .bind(source)
        .bind(source_type)
        .bind(source_ids)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_mappings(
        &mut self,
        source: &str,
        source_type: &str,
        after: Option<&str>,
        limit: i64,
    ) -> AppResult<Vec<MappingRow>> {
        let rows: Vec<MappingDbRow> = sqlx::query_as(&format!(
            "SELECT {MAPPING_COLS} FROM import_mapping \
             WHERE organization_id = $1 AND source = $2 AND source_type = $3 \
               AND ($4::text IS NULL OR source_id > $4) \
             ORDER BY source_id LIMIT $5"
        ))
        .bind(self.org.0)
        .bind(source)
        .bind(source_type)
        .bind(after)
        .bind(limit.clamp(1, 1000))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn upsert_mapping(&mut self, m: &NewMapping) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO import_mapping (organization_id, import_run_id, source, source_type, source_id, \
               target_id, source_updated_at, checksum, imported_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (organization_id, source, source_type, source_id) DO UPDATE SET \
               import_run_id = EXCLUDED.import_run_id, target_id = EXCLUDED.target_id, \
               source_updated_at = EXCLUDED.source_updated_at, checksum = EXCLUDED.checksum, \
               imported_at = EXCLUDED.imported_at, version = import_mapping.version + 1, \
               updated_at = EXCLUDED.imported_at",
        )
        .bind(self.org.0)
        .bind(m.import_run_id)
        .bind(&m.source)
        .bind(&m.source_type)
        .bind(&m.source_id)
        .bind(m.target_id)
        .bind(m.source_updated_at)
        .bind(&m.checksum)
        .bind(m.imported_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_mapping(
        &mut self,
        source: &str,
        source_type: &str,
        source_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM import_mapping WHERE organization_id = $1 AND source = $2 \
             AND source_type = $3 AND source_id = $4",
        )
        .bind(self.org.0)
        .bind(source)
        .bind(source_type)
        .bind(source_id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn count_mappings(&mut self, source: &str) -> AppResult<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT source_type, count(*) AS n FROM import_mapping \
             WHERE organization_id = $1 AND source = $2 GROUP BY source_type ORDER BY source_type",
        )
        .bind(self.org.0)
        .bind(source)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("source_type"), r.get::<i64, _>("n")))
            .collect())
    }

    async fn add_report_entries(&mut self, run_id: Uuid, entries: &[ReportEntry]) -> AppResult<()> {
        for e in entries {
            sqlx::query(
                "INSERT INTO import_issue (organization_id, import_run_id, severity, source_type, source_id, message, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, COALESCE($7, now()))",
            )
            .bind(self.org.0)
            .bind(run_id)
            .bind(&e.severity)
            .bind(&e.source_type)
            .bind(&e.source_id)
            .bind(&e.message)
            .bind(e.created_at)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn delete_report_entries(&mut self, run_id: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM import_issue WHERE organization_id = $1 AND import_run_id = $2")
            .bind(self.org.0)
            .bind(run_id)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn list_report_entries(
        &mut self,
        run_id: Uuid,
        severity: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ReportEntry>> {
        let rows = sqlx::query(
            "SELECT severity, source_type, source_id, message, created_at FROM import_issue \
             WHERE organization_id = $1 AND import_run_id = $2 AND ($3::text IS NULL OR severity = $3) \
             ORDER BY created_at, id LIMIT $4 OFFSET $5",
        )
        .bind(self.org.0)
        .bind(run_id)
        .bind(severity)
        .bind(limit.clamp(1, 1000))
        .bind(offset.max(0))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| ReportEntry {
                severity: r.get("severity"),
                source_type: r.get("source_type"),
                source_id: r.get("source_id"),
                message: r.get("message"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn report_counts(&mut self, run_id: Uuid) -> AppResult<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT severity, count(*) AS n FROM import_issue \
             WHERE organization_id = $1 AND import_run_id = $2 GROUP BY severity",
        )
        .bind(self.org.0)
        .bind(run_id)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("severity"), r.get::<i64, _>("n")))
            .collect())
    }

    async fn insert_row(&mut self, table: ImportTable, row: &Value) -> AppResult<()> {
        let (cols, obj) = prepare_row(self.org.0, row, true)?;
        let t = table.name();
        let list = cols.join(", ");
        // Savepoint: eine verletzte Eindeutigkeit/Referenz verwirft nur diese Zeile, nicht die Transaktion.
        let mut sp = sqlx::Acquire::begin(&mut *self.tx).await.map_err(db_err)?;
        let r = sqlx::query(&format!(
            "INSERT INTO {t} ({list}) SELECT {list} FROM jsonb_populate_record(NULL::{t}, $1::jsonb)"
        ))
        .bind(&obj)
        .execute(&mut *sp)
        .await;
        match r {
            Ok(_) => sp.commit().await.map_err(db_err),
            Err(e) => {
                sp.rollback().await.map_err(db_err)?;
                Err(row_rejected(&e))
            }
        }
    }

    async fn insert_row_ignore(&mut self, table: ImportTable, row: &Value) -> AppResult<bool> {
        let (cols, obj) = prepare_row(self.org.0, row, true)?;
        let t = table.name();
        let list = cols.join(", ");
        // Savepoint, damit auch Verletzungen anderer Eindeutigkeiten (Teil-Indizes) nicht die
        // ganze Transaktion abbrechen.
        let mut sp = sqlx::Acquire::begin(&mut *self.tx).await.map_err(db_err)?;
        let r = sqlx::query(&format!(
            "INSERT INTO {t} ({list}) SELECT {list} FROM jsonb_populate_record(NULL::{t}, $1::jsonb) \
             ON CONFLICT DO NOTHING"
        ))
        .bind(&obj)
        .execute(&mut *sp)
        .await;
        match r {
            Ok(r) => {
                sp.commit().await.map_err(db_err)?;
                Ok(r.rows_affected() > 0)
            }
            Err(_) => {
                sp.rollback().await.map_err(db_err)?;
                Ok(false)
            }
        }
    }

    async fn update_row(&mut self, table: ImportTable, id: Uuid, row: &Value) -> AppResult<bool> {
        let (cols, obj) = prepare_row(self.org.0, row, false)?;
        let t = table.name();
        let sets = cols
            .iter()
            .map(|c| format!("{c} = r.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut sp = sqlx::Acquire::begin(&mut *self.tx).await.map_err(db_err)?;
        let r = sqlx::query(&format!(
            "UPDATE {t} AS x SET {sets}, version = x.version + 1 \
             FROM jsonb_populate_record(NULL::{t}, $1::jsonb) AS r \
             WHERE x.id = $2 AND x.organization_id = $3"
        ))
        .bind(&obj)
        .bind(id)
        .bind(self.org.0)
        .execute(&mut *sp)
        .await;
        match r {
            Ok(r) => {
                sp.commit().await.map_err(db_err)?;
                Ok(r.rows_affected() > 0)
            }
            Err(e) => {
                sp.rollback().await.map_err(db_err)?;
                Err(row_rejected(&e))
            }
        }
    }

    async fn delete_rows_where(
        &mut self,
        table: ImportTable,
        column: &str,
        id: Uuid,
    ) -> AppResult<u64> {
        check_ident(column)?;
        let r = sqlx::query(&format!(
            "DELETE FROM {} WHERE organization_id = $1 AND \"{column}\" = $2",
            table.name()
        ))
        .bind(self.org.0)
        .bind(id)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(r.rows_affected())
    }

    async fn row_updated_at(
        &mut self,
        table: ImportTable,
        id: Uuid,
    ) -> AppResult<Option<DateTime<Utc>>> {
        sqlx::query_scalar(&format!(
            "SELECT updated_at FROM {} WHERE organization_id = $1 AND id = $2",
            table.name()
        ))
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn find_user_id_by_email(&mut self, email: &str) -> AppResult<Option<Uuid>> {
        sqlx::query_scalar(
            "SELECT id FROM \"user\" WHERE organization_id = $1 AND lower(email) = lower($2) \
             AND archived_at IS NULL AND (active OR status = 'imported')",
        )
        .bind(self.org.0)
        .bind(email)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn user_display_name_taken(&mut self, display_name: &str) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM \"user\" WHERE organization_id = $1 AND lower(display_name) = lower($2))",
        )
        .bind(self.org.0)
        .bind(display_name)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn find_team_by_key(&mut self, key: &str) -> AppResult<Option<Uuid>> {
        sqlx::query_scalar(
            "SELECT id FROM team WHERE organization_id = $1 AND key = $2 AND NOT trashed \
             UNION SELECT team_id FROM team_key WHERE organization_id = $1 AND key = $2 LIMIT 1",
        )
        .bind(self.org.0)
        .bind(key)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn all_team_keys(&mut self) -> AppResult<Vec<String>> {
        sqlx::query_scalar(
            "SELECT key FROM team WHERE organization_id = $1 AND NOT trashed \
             UNION SELECT key FROM team_key WHERE organization_id = $1",
        )
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn next_import_number(
        &mut self,
        counter: ImportCounter,
        scope: Option<Uuid>,
    ) -> AppResult<i32> {
        match counter {
            ImportCounter::Project => {
                let team =
                    scope.ok_or_else(|| AppError::internal("project counter needs a team"))?;
                sqlx::query_scalar(
                    "UPDATE team SET last_project_number = last_project_number + 1 \
                     WHERE organization_id = $1 AND id = $2 RETURNING last_project_number",
                )
                .bind(self.org.0)
                .bind(team)
                .fetch_one(&mut *self.tx)
                .await
                .map_err(db_err)
            }
            ImportCounter::Initiative => sqlx::query_scalar(
                "UPDATE organization SET last_initiative_number = last_initiative_number + 1 \
                 WHERE id = $1 RETURNING last_initiative_number",
            )
            .bind(self.org.0)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(db_err),
        }
    }

    async fn finalize_import_counters(&mut self, team_ids: &[Uuid]) -> AppResult<()> {
        sqlx::query(
            "UPDATE team t SET \
               last_issue_number = GREATEST(t.last_issue_number, \
                 COALESCE((SELECT max(number) FROM issue i WHERE i.team_id = t.id), 0)), \
               last_cycle_number = GREATEST(t.last_cycle_number, \
                 COALESCE((SELECT max(number) FROM cycle c WHERE c.team_id = t.id), 0)), \
               last_project_number = GREATEST(t.last_project_number, \
                 COALESCE((SELECT max(number) FROM project p WHERE p.lead_team_id = t.id), 0)) \
             WHERE t.organization_id = $1 AND t.id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(team_ids)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "UPDATE organization SET last_initiative_number = GREATEST(last_initiative_number, \
               COALESCE((SELECT max(number) FROM initiative i WHERE i.organization_id = organization.id), 0)) \
             WHERE id = $1",
        )
        .bind(self.org.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn set_team_import_lock(
        &mut self,
        team_ids: &[Uuid],
        until: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE team SET import_locked_until = $3 WHERE organization_id = $1 AND id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(team_ids)
        .bind(until)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn extend_team_import_lock(
        &mut self,
        team_ids: &[Uuid],
        until: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE team SET import_locked_until = $3 WHERE id IN ( \
               SELECT id FROM team WHERE organization_id = $1 AND id = ANY($2) \
                 AND import_locked_until IS NOT NULL FOR UPDATE SKIP LOCKED)",
        )
        .bind(self.org.0)
        .bind(team_ids)
        .bind(until)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn set_team_states(
        &mut self,
        team_id: Uuid,
        default_state: Option<Uuid>,
        triage_state: Option<Uuid>,
        auto_close_state: Option<Uuid>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE team SET default_issue_state_id = COALESCE($3, default_issue_state_id), \
               triage_issue_state_id = COALESCE($4, triage_issue_state_id), \
               auto_close_state_id = COALESCE($5, auto_close_state_id) \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(team_id)
        .bind(default_state)
        .bind(triage_state)
        .bind(auto_close_state)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn first_state_of_types(
        &mut self,
        team_id: Uuid,
        types: &[&str],
    ) -> AppResult<Option<Uuid>> {
        let types: Vec<String> = types.iter().map(|s| (*s).to_owned()).collect();
        for t in &types {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM workflow_state WHERE organization_id = $1 AND team_id = $2 \
                 AND type = $3 AND archived_at IS NULL ORDER BY position, created_at LIMIT 1",
            )
            .bind(self.org.0)
            .bind(team_id)
            .bind(t)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
            if id.is_some() {
                return Ok(id);
            }
        }
        Ok(None)
    }

    async fn find_rows_containing(
        &mut self,
        table: ImportTable,
        column: &str,
        owner_column: &str,
        needle: &str,
        after: Option<Uuid>,
        limit: i64,
    ) -> AppResult<Vec<(Uuid, Uuid, String)>> {
        check_ident(column)?;
        check_ident(owner_column)?;
        let rows = sqlx::query(&format!(
            "SELECT id, \"{owner_column}\" AS owner, \"{column}\" AS body FROM {} WHERE organization_id = $1 \
             AND position($2 in \"{column}\") > 0 AND \"{owner_column}\" IS NOT NULL \
             AND ($3::uuid IS NULL OR id > $3) ORDER BY id LIMIT $4",
            table.name()
        ))
        .bind(self.org.0)
        .bind(needle)
        .bind(after)
        .bind(limit.clamp(1, 500))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<Uuid, _>("id"),
                    r.get::<Uuid, _>("owner"),
                    r.get::<String, _>("body"),
                )
            })
            .collect())
    }

    async fn set_row_uuid(
        &mut self,
        table: ImportTable,
        column: &str,
        id: Uuid,
        value: Option<Uuid>,
    ) -> AppResult<()> {
        check_ident(column)?;
        sqlx::query(&format!(
            "UPDATE {} SET \"{column}\" = $3 WHERE organization_id = $1 AND id = $2",
            table.name()
        ))
        .bind(self.org.0)
        .bind(id)
        .bind(value)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn issue_parent_would_cycle(&mut self, child: Uuid, parent: Uuid) -> AppResult<bool> {
        if child == parent {
            return Ok(true);
        }
        // UNION (nicht UNION ALL) und Tiefengrenze: terminiert auch auf bereits zyklischen Daten.
        sqlx::query_scalar(
            "WITH RECURSIVE anc(id, depth) AS ( \
               SELECT parent_id, 1 FROM issue WHERE organization_id = $1 AND id = $3 \
                 AND parent_id IS NOT NULL \
               UNION \
               SELECT i.parent_id, a.depth + 1 FROM issue i JOIN anc a ON i.id = a.id \
               WHERE i.organization_id = $1 AND i.parent_id IS NOT NULL AND a.depth < 1000) \
             SELECT EXISTS (SELECT 1 FROM anc WHERE id = $2)",
        )
        .bind(self.org.0)
        .bind(child)
        .bind(parent)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn set_row_text(
        &mut self,
        table: ImportTable,
        column: &str,
        id: Uuid,
        value: &str,
    ) -> AppResult<()> {
        check_ident(column)?;
        sqlx::query(&format!(
            "UPDATE {} SET \"{column}\" = $3 WHERE organization_id = $1 AND id = $2",
            table.name()
        ))
        .bind(self.org.0)
        .bind(id)
        .bind(value)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_import_sync(&mut self) -> AppResult<Option<ImportSyncRow>> {
        let row: Option<SyncRowDb> = sqlx::query_as(&format!(
            "SELECT {SYNC_COLS} FROM import_sync WHERE organization_id = $1"
        ))
        .bind(self.org.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn put_import_sync(&mut self, r: &ImportSyncRow, token: Option<&[u8]>) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO import_sync (id, organization_id, interval, daily_at_minute, paused, teams, projects, \
               target_teams, user_mapping, key_mapping, created_by, next_run_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13) \
             ON CONFLICT (organization_id) DO UPDATE SET interval = $3, daily_at_minute = $4, paused = $5, \
               teams = $6, projects = $7, target_teams = $8, user_mapping = $9, key_mapping = $10, \
               next_run_at = $12, updated_at = $13, version = import_sync.version + 1",
        )
        .bind(r.id)
        .bind(self.org.0)
        .bind(&r.interval)
        .bind(r.daily_at_minute)
        .bind(r.paused)
        .bind(&r.teams)
        .bind(&r.projects)
        .bind(&r.target_teams)
        .bind(&r.user_mapping)
        .bind(&r.key_mapping)
        .bind(r.created_by)
        .bind(r.next_run_at)
        .bind(r.updated_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if let Some(t) = token {
            sqlx::query(
                "INSERT INTO import_sync_credential (sync_id, organization_id, encrypted_token) \
                 SELECT id, organization_id, $2 FROM import_sync WHERE organization_id = $1 \
                 ON CONFLICT (sync_id) DO UPDATE SET encrypted_token = $2, updated_at = now()",
            )
            .bind(self.org.0)
            .bind(t)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn get_import_sync_token(&mut self, sync_id: Uuid) -> AppResult<Option<Vec<u8>>> {
        sqlx::query_scalar(
            "SELECT encrypted_token FROM import_sync_credential WHERE organization_id = $1 AND sync_id = $2",
        )
        .bind(self.org.0)
        .bind(sync_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn delete_import_sync(&mut self, sync_id: Uuid) -> AppResult<bool> {
        let r = sqlx::query("DELETE FROM import_sync WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(sync_id)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(r.rows_affected() > 0)
    }

    async fn due_import_syncs(&mut self, now: DateTime<Utc>) -> AppResult<Vec<ImportSyncRow>> {
        let rows: Vec<SyncRowDb> = sqlx::query_as(&format!(
            "SELECT {SYNC_COLS} FROM import_sync WHERE NOT paused AND interval <> 'off' \
             AND next_run_at IS NOT NULL AND next_run_at <= $1 ORDER BY next_run_at LIMIT 50"
        ))
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn active_sync_run(&mut self, sync_id: Uuid) -> AppResult<Option<Uuid>> {
        sqlx::query_scalar(
            "SELECT id FROM import_run WHERE organization_id = $1 AND options->>'syncId' = $2 \
             AND status IN ('scanning', 'planned', 'running', 'paused') AND archived_at IS NULL LIMIT 1",
        )
        .bind(self.org.0)
        .bind(sync_id.to_string())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn sync_mark_started(
        &mut self,
        sync_id: Uuid,
        run_id: Uuid,
        started: DateTime<Utc>,
        next_run_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE import_sync SET last_run_id = $3, last_run_at = $4, last_status = 'running', \
               last_error = NULL, next_run_at = $5, updated_at = $4, version = version + 1 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(sync_id)
        .bind(run_id)
        .bind(started)
        .bind(next_run_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn sync_mark_result(
        &mut self,
        sync_id: Uuid,
        run_id: Option<Uuid>,
        status: &str,
        error: Option<&str>,
        success_started_at: Option<DateTime<Utc>>,
        removal_checked_at: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE import_sync SET last_status = $4, last_error = $5, \
               last_success_at = COALESCE($6, last_success_at), \
               last_removal_check_at = COALESCE($7, last_removal_check_at), \
               updated_at = now(), version = version + 1 \
             WHERE organization_id = $1 AND id = $2 AND ($3::uuid IS NULL OR last_run_id = $3)",
        )
        .bind(self.org.0)
        .bind(sync_id)
        .bind(run_id)
        .bind(status)
        .bind(error)
        .bind(success_started_at)
        .bind(removal_checked_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn merge_user_references(
        &mut self,
        from: Uuid,
        to: Uuid,
        from_handle: &str,
        to_handle: &str,
    ) -> AppResult<UserMergeStats> {
        let org = self.org.0;
        let mut stats = UserMergeStats::default();
        // Rollen: Eigentümer-Flag der Mitgliedschaften gilt auch beim Ziel (ODER).
        sqlx::query(
            "UPDATE team_membership t SET owner = true, version = t.version + 1 \
             FROM team_membership p \
             WHERE t.organization_id = $1 AND p.organization_id = $1 AND p.user_id = $2 AND t.user_id = $3 \
               AND t.team_id = p.team_id AND t.archived_at IS NULL AND p.archived_at IS NULL \
               AND p.owner AND NOT t.owner",
        )
        .bind(org)
        .bind(from)
        .bind(to)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        // Alle Fremdschlüssel auf "user"(id) aus dem Katalog (Spalte, ggf. zugehörige Mandantenspalte).
        let refs: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT c.conrelid::regclass::text, a.attname::text, o.attname::text \
             FROM pg_constraint c \
             JOIN LATERAL unnest(c.conkey, c.confkey) AS k(col, fcol) ON true \
             JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.col \
             JOIN pg_attribute fa ON fa.attrelid = c.confrelid AND fa.attnum = k.fcol AND fa.attname = 'id' \
             LEFT JOIN LATERAL ( \
               SELECT oa.attname FROM unnest(c.conkey, c.confkey) AS k2(col, fcol) \
               JOIN pg_attribute oa ON oa.attrelid = c.conrelid AND oa.attnum = k2.col \
               JOIN pg_attribute ofa ON ofa.attrelid = c.confrelid AND ofa.attnum = k2.fcol \
                 AND ofa.attname = 'organization_id' LIMIT 1) o ON true \
             WHERE c.contype = 'f' AND c.confrelid = '\"user\"'::regclass \
             ORDER BY 1, 2",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        for (table, col, org_col) in refs {
            let ident_ok = |s: &str| {
                s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '"' | '.'))
            };
            if !ident_ok(&table)
                || !ident_ok(&col)
                || org_col.as_deref().is_some_and(|c| !ident_ok(c))
            {
                return Err(AppError::internal(
                    "unexpected identifier in the schema catalog",
                ));
            }
            let scope = match &org_col {
                Some(c) => format!(" AND \"{c}\" = $3"),
                None => String::new(),
            };
            let bulk = format!("UPDATE {table} SET \"{col}\" = $2 WHERE \"{col}\" = $1{scope}");
            let mut sp = sqlx::Acquire::begin(&mut *self.tx).await.map_err(db_err)?;
            let mut q = sqlx::query(&bulk).bind(from).bind(to);
            if org_col.is_some() {
                q = q.bind(org);
            }
            match q.execute(&mut *sp).await {
                Ok(r) => {
                    sp.commit().await.map_err(db_err)?;
                    stats.moved += r.rows_affected();
                }
                Err(e) if is_constraint_clash(&e) => {
                    sp.rollback().await.map_err(db_err)?;
                    // Zeilenweise: kollidierende Zeilen (beim Ziel gleichwertig vorhanden) entfallen.
                    let sel = format!(
                        "SELECT ctid::text FROM {table} WHERE \"{col}\" = $1{}",
                        scope.replace("$3", "$2")
                    );
                    let mut q = sqlx::query_scalar::<_, String>(&sel).bind(from);
                    if org_col.is_some() {
                        q = q.bind(org);
                    }
                    let ctids = q.fetch_all(&mut *self.tx).await.map_err(db_err)?;
                    for ctid in ctids {
                        let mut sp = sqlx::Acquire::begin(&mut *self.tx).await.map_err(db_err)?;
                        let upd =
                            format!("UPDATE {table} SET \"{col}\" = $2 WHERE ctid = $1::text::tid");
                        match sqlx::query(&upd)
                            .bind(&ctid)
                            .bind(to)
                            .execute(&mut *sp)
                            .await
                        {
                            Ok(r) => {
                                sp.commit().await.map_err(db_err)?;
                                stats.moved += r.rows_affected();
                            }
                            Err(e) if is_constraint_clash(&e) => {
                                sp.rollback().await.map_err(db_err)?;
                                let del = format!("DELETE FROM {table} WHERE ctid = $1::text::tid");
                                let r = sqlx::query(&del)
                                    .bind(&ctid)
                                    .execute(&mut *self.tx)
                                    .await
                                    .map_err(db_err)?;
                                stats.dropped += r.rows_affected();
                            }
                            Err(e) => return Err(db_err(e)),
                        }
                    }
                }
                Err(e) => return Err(db_err(e)),
            }
        }
        // Arrays von User-IDs (ohne Fremdschlüssel).
        for (table, col) in [
            ("issue_history", "notified_user_ids"),
            ("document_content_history", "actor_ids"),
            ("triage_responsibility", "manual_user_ids"),
        ] {
            sqlx::query(&format!(
                "UPDATE {table} SET {col} = array_replace({col}, $1, $2) \
                 WHERE organization_id = $3 AND {col} @> ARRAY[$1]::uuid[]"
            ))
            .bind(from)
            .bind(to)
            .bind(org)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        // Erwähnungen im Text: `@alt` -> `@neu` (Namensgrenzen, ohne Übergriff auf längere Namen).
        let esc = from_handle.replace('.', "\\.");
        let pattern = format!("(^|[^a-z0-9._@-])@{esc}(?![a-z0-9_-]|\\.[a-z0-9_-])");
        let replacement = format!("\\1@{to_handle}");
        for (table, col) in [
            ("issue", "description"),
            ("comment", "body"),
            ("document", "content"),
            ("document_content", "markdown"),
            ("document_content_history", "markdown"),
            ("project", "content"),
            ("project", "description"),
            ("initiative", "content"),
            ("initiative", "description"),
            ("project_update", "body"),
            ("initiative_update", "body"),
            ("project_milestone", "description"),
        ] {
            // Der CRDT-Zustand eines Dokuments wird verworfen und aus dem Markdown neu aufgebaut.
            let extra = if table == "document_content" {
                ", content_state = NULL"
            } else {
                ""
            };
            let r = sqlx::query(&format!(
                "UPDATE {table} SET {col} = regexp_replace({col}, $1, $2, 'gi'){extra} \
                 WHERE organization_id = $3 AND {col} ~* $1"
            ))
            .bind(&pattern)
            .bind(&replacement)
            .bind(org)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
            stats.mentions += r.rows_affected();
        }
        sqlx::query(
            "UPDATE import_mapping SET target_id = $2, updated_at = now() \
             WHERE organization_id = $3 AND source_type = 'User' AND target_id = $1",
        )
        .bind(from)
        .bind(to)
        .bind(org)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let del = sqlx::query(
            "DELETE FROM \"user\" WHERE id = $1 AND organization_id = $2 AND status = 'imported'",
        )
        .bind(from)
        .bind(org)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "placeholder user is still referenced");
            AppError::conflict("the placeholder is still referenced and cannot be removed")
        })?;
        if del.rows_affected() != 1 {
            return Err(AppError::not_found("Placeholder user not found."));
        }
        Ok(stats)
    }

    async fn repoint_mappings(
        &mut self,
        source_type: &str,
        from: Uuid,
        to: Uuid,
    ) -> AppResult<u64> {
        let r = sqlx::query(
            "UPDATE import_mapping SET target_id = $2, updated_at = now() \
             WHERE organization_id = $3 AND source_type = $4 AND target_id = $1",
        )
        .bind(from)
        .bind(to)
        .bind(self.org.0)
        .bind(source_type)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(r.rows_affected())
    }

    async fn row_team_id(&mut self, table: ImportTable, id: Uuid) -> AppResult<Option<Uuid>> {
        if table != ImportTable::Issue {
            return Ok(None);
        }
        sqlx::query_scalar("SELECT team_id FROM issue WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)
    }

    async fn archive_row(
        &mut self,
        table: ImportTable,
        id: Uuid,
        at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let r = sqlx::query(&format!(
            "UPDATE {} SET archived_at = $3, version = version + 1 \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
            table.name()
        ))
        .bind(self.org.0)
        .bind(id)
        .bind(at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(r.rows_affected() > 0)
    }

    async fn list_linear_suffixed_labels(
        &mut self,
    ) -> AppResult<Vec<(Uuid, String, Option<Uuid>, Option<Uuid>)>> {
        let rows: Vec<(Uuid, String, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
            "SELECT l.id, left(l.name, char_length(l.name) - 9), l.team_id, l.parent_id FROM issue_label l \
             WHERE l.organization_id = $1 AND l.name LIKE '% (Linear)' AND NOT l.is_group \
               AND EXISTS (SELECT 1 FROM import_mapping m WHERE m.organization_id = l.organization_id \
                           AND m.source_type = 'IssueLabel' AND m.target_id = l.id) \
             ORDER BY l.name",
        )
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows)
    }

    async fn find_label_by_name_at(
        &mut self,
        table: ImportTable,
        name: &str,
        team_id: Option<Uuid>,
        parent_id: Option<Uuid>,
    ) -> AppResult<Option<Uuid>> {
        sqlx::query_scalar(&format!(
            "SELECT id FROM {} WHERE organization_id = $1 AND lower(name) = lower($2) \
             AND team_id IS NOT DISTINCT FROM $3 AND parent_id IS NOT DISTINCT FROM $4 LIMIT 1",
            table.name()
        ))
        .bind(self.org.0)
        .bind(name)
        .bind(team_id)
        .bind(parent_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn find_row_id_by_name(
        &mut self,
        table: ImportTable,
        name: &str,
        team_id: Option<Uuid>,
    ) -> AppResult<Option<Uuid>> {
        sqlx::query_scalar(&format!(
            "SELECT id FROM {} WHERE organization_id = $1 AND lower(name) = lower($2) \
             AND team_id IS NOT DISTINCT FROM $3 AND archived_at IS NULL LIMIT 1",
            table.name()
        ))
        .bind(self.org.0)
        .bind(name)
        .bind(team_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn find_state_by_type_and_name(
        &mut self,
        team_id: Uuid,
        state_type: &str,
        name: &str,
    ) -> AppResult<Option<Uuid>> {
        sqlx::query_scalar(
            "SELECT id FROM workflow_state WHERE organization_id = $1 AND team_id = $2 \
             AND type = $3 AND lower(name) = lower($4) AND archived_at IS NULL LIMIT 1",
        )
        .bind(self.org.0)
        .bind(team_id)
        .bind(state_type)
        .bind(name)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn count_mapped_rows(
        &mut self,
        source: &str,
        source_type: &str,
        table: ImportTable,
    ) -> AppResult<i64> {
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM import_mapping m JOIN {} t ON t.id = m.target_id \
             WHERE m.organization_id = $1 AND m.source = $2 AND m.source_type = $3",
            table.name()
        ))
        .bind(self.org.0)
        .bind(source)
        .bind(source_type)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn count_rows(&mut self, table: ImportTable) -> AppResult<i64> {
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {} WHERE organization_id = $1",
            table.name()
        ))
        .bind(self.org.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }
}
