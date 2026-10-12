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
    ImportCounter, ImportRun, ImportRunUpdate, ImportTable, MappingRow, NewImportRun, NewMapping,
    ReportEntry,
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
fn row_rejected(e: &sqlx::Error) -> AppError {
    tracing::warn!(error = %e, "import row rejected by the database");
    AppError::conflict("the row was rejected by the database")
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
            "SELECT id FROM \"user\" WHERE organization_id = $1 AND lower(email) = lower($2)",
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
