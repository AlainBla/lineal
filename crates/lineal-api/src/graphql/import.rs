//! GraphQL-Teilobjekte des Bereichs `import` (WP18, docs/13 §6): Import aus Linear.
//!
//! Alle Operationen laufen durch die Use-Cases in `lineal_app::usecases::import`
//! (`Action::ManageImport`: nur Workspace-Admins/Owner mit Scope `admin`). Der Scan selbst und
//! der Lauf passieren asynchron im Import-Job; hier werden nur Läufe angelegt und gesteuert.
//! Das Token wird nie zurückgegeben und im Schema als `secret` markiert.

use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::types::Json;
use async_graphql::{Context, InputObject, Object, SimpleObject};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::AppError;
use lineal_app::ports::repos::import::{ImportRun, ReportEntry};
use lineal_app::usecases::import::{ImportScanInput, ImportStartInput};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_id(id: &str) -> async_graphql::Result<Uuid> {
    Uuid::parse_str(id.trim()).map_err(|_| gql_err(AppError::invalid_request("invalid id")))
}

/// Ein Importlauf (`ImportRun`). `scan` und `stats` sind JSON (Plan bzw. Zähler je Entität).
#[derive(SimpleObject)]
#[graphql(name = "ImportRun")]
pub struct ImportRunGql {
    pub id: String,
    /// `linear` oder `linearCsv`.
    pub source: String,
    /// `scanning`, `planned`, `running`, `paused`, `failed`, `completed`, `canceled`.
    pub status: String,
    /// Aktuelle Phase (1–8) laut Checkpoint, `null` vor dem Start.
    pub phase: Option<i32>,
    /// Aktueller Schritt laut Checkpoint (z. B. `Issues`).
    pub step: Option<String>,
    pub dry_run: bool,
    /// Ergebnis des Scans: Workspace, Teams (mit Mengen), User, Zählungen, Key-Konflikte.
    pub scan: Option<Json<Value>>,
    /// Zähler je Entität (`entities`), Verifikation (`verification`).
    pub stats: Json<Value>,
    pub options: Json<Value>,
    pub checkpoint: Json<Value>,
    pub error: Option<String>,
    pub started_by: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ImportRun> for ImportRunGql {
    fn from(r: ImportRun) -> Self {
        let scan = r.stats.get("scan").cloned().map(Json);
        let mut stats = r.stats.clone();
        if let Some(o) = stats.as_object_mut() {
            o.remove("scan");
        }
        Self {
            id: r.id.to_string(),
            source: r.source,
            status: r.status,
            phase: r.checkpoint["phase"].as_i64().map(|p| p as i32),
            step: r.checkpoint["step"].as_str().map(str::to_owned),
            dry_run: r.options["dryRun"].as_bool().unwrap_or(false),
            scan,
            stats: Json(stats),
            options: Json(r.options),
            checkpoint: Json(r.checkpoint),
            error: r.error,
            started_by: r.started_by.map(|u| u.to_string()),
            started_at: r.started_at.map(ts),
            finished_at: r.finished_at.map(ts),
            created_at: ts(r.created_at),
            updated_at: ts(r.updated_at),
        }
    }
}

/// Ein Eintrag des Berichts (`ImportIssue`).
#[derive(SimpleObject)]
#[graphql(name = "ImportIssue")]
pub struct ImportIssueGql {
    /// `info`, `warning`, `error`.
    pub severity: String,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub message: String,
    pub created_at: Option<String>,
}

impl From<ReportEntry> for ImportIssueGql {
    fn from(e: ReportEntry) -> Self {
        Self {
            severity: e.severity,
            source_type: e.source_type,
            source_id: e.source_id,
            message: e.message,
            created_at: e.created_at.map(ts),
        }
    }
}

/// Bericht eines Laufs.
#[derive(SimpleObject)]
#[graphql(name = "ImportReport")]
pub struct ImportReportGql {
    pub entries: Vec<ImportIssueGql>,
    pub info_count: i32,
    pub warning_count: i32,
    pub error_count: i32,
}

/// Eingabe `importScan`. Das Token ist `secret`: es erscheint in keinem Fehler- oder Log-Output.
#[derive(InputObject)]
#[graphql(name = "ImportScanInput")]
pub struct ImportScanInputGql {
    /// `linear`.
    pub source: String,
    #[graphql(secret)]
    pub token: String,
}

/// Eingabe `importStart`.
#[derive(InputObject)]
#[graphql(name = "ImportStartInput")]
pub struct ImportStartInputGql {
    pub run_id: String,
    /// Linear-Team-Keys; leer = alle gescannten Teams.
    #[graphql(default)]
    pub teams: Vec<String>,
    /// Objekt `{ "<Linear-User-ID>": "<Lineal-User-ID>" | null }`.
    pub user_mapping: Option<Json<Value>>,
    /// Objekt `{ "<Linear-Team-Key>": "<Ziel-Key in Lineal>" }`.
    pub key_mapping: Option<Json<Value>>,
    /// Probelauf: Mapping und Validierung ohne Schreiben.
    #[graphql(default)]
    pub dry_run: bool,
    /// Nur Änderungen seit dem letzten erfolgreichen Lauf übernehmen.
    #[graphql(default)]
    pub delta: bool,
}

fn user_mapping(v: Option<Json<Value>>) -> async_graphql::Result<BTreeMap<String, Option<Uuid>>> {
    let Some(Json(v)) = v else {
        return Ok(BTreeMap::new());
    };
    let Some(m) = v.as_object() else {
        return Err(gql_err(AppError::invalid_request(
            "userMapping must be an object",
        )));
    };
    if m.len() > 10_000 {
        return Err(gql_err(AppError::invalid_request(
            "userMapping is too large",
        )));
    }
    let mut out = BTreeMap::new();
    for (k, val) in m {
        let target = match val {
            Value::Null => None,
            Value::String(s) => Some(parse_id(s)?),
            _ => {
                return Err(gql_err(AppError::invalid_request(
                    "userMapping values must be ids or null",
                )));
            }
        };
        out.insert(k.clone(), target);
    }
    Ok(out)
}

fn key_mapping(v: Option<Json<Value>>) -> async_graphql::Result<BTreeMap<String, String>> {
    let Some(Json(v)) = v else {
        return Ok(BTreeMap::new());
    };
    let Some(m) = v.as_object() else {
        return Err(gql_err(AppError::invalid_request(
            "keyMapping must be an object",
        )));
    };
    if m.len() > 1_000 {
        return Err(gql_err(AppError::invalid_request(
            "keyMapping is too large",
        )));
    }
    let mut out = BTreeMap::new();
    for (k, val) in m {
        let Some(target) = val.as_str() else {
            return Err(gql_err(AppError::invalid_request(
                "keyMapping values must be strings",
            )));
        };
        out.insert(k.clone(), target.to_owned());
    }
    Ok(out)
}

#[derive(Default)]
pub struct ImportQuery;

#[Object]
impl ImportQuery {
    /// Ein Importlauf mit Status, Fortschritt und Plan (nur Admins).
    async fn import_run(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<ImportRunGql> {
        let run = app(ctx)?
            .import_get_run(request_ctx(ctx)?, parse_id(&id)?)
            .await
            .map_err(gql_err)?;
        Ok(run.into())
    }

    /// Alle Importläufe, neueste zuerst (nur Admins).
    async fn import_runs(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<ImportRunGql>> {
        let runs = app(ctx)?
            .import_list_runs(request_ctx(ctx)?, i64::from(first.unwrap_or(20)))
            .await
            .map_err(gql_err)?;
        Ok(runs.into_iter().map(Into::into).collect())
    }

    /// Bericht eines Laufs (übernommen, übersprungen, Warnungen, Konflikte).
    async fn import_report(
        &self,
        ctx: &Context<'_>,
        run_id: String,
        severity: Option<String>,
        first: Option<i32>,
        offset: Option<i32>,
    ) -> async_graphql::Result<ImportReportGql> {
        let (entries, counts) = app(ctx)?
            .import_report(
                request_ctx(ctx)?,
                parse_id(&run_id)?,
                severity.as_deref(),
                i64::from(first.unwrap_or(100)),
                i64::from(offset.unwrap_or(0)),
            )
            .await
            .map_err(gql_err)?;
        let n = |s: &str| {
            counts
                .iter()
                .find(|(k, _)| k == s)
                .map_or(0, |(_, n)| *n as i32)
        };
        Ok(ImportReportGql {
            info_count: n("info"),
            warning_count: n("warning"),
            error_count: n("error"),
            entries: entries.into_iter().map(Into::into).collect(),
        })
    }
}

#[derive(Default)]
pub struct ImportMutation;

#[Object]
impl ImportMutation {
    /// Verbindung prüfen und Plan vorbereiten: legt einen Lauf (`scanning`) an und speichert das
    /// Token verschlüsselt; der Import-Job liest Teams, User und Mengen (Status danach `planned`).
    async fn import_scan(
        &self,
        ctx: &Context<'_>,
        input: ImportScanInputGql,
    ) -> async_graphql::Result<ImportRunGql> {
        let run = app(ctx)?
            .import_scan(
                request_ctx(ctx)?,
                ImportScanInput {
                    source: input.source,
                    token: input.token,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(run.into())
    }

    /// Lauf starten (asynchron als Job). Key-Konflikte mit bestehenden Teams brechen vorher ab.
    async fn import_start(
        &self,
        ctx: &Context<'_>,
        input: ImportStartInputGql,
    ) -> async_graphql::Result<ImportRunGql> {
        let run = app(ctx)?
            .import_start(
                request_ctx(ctx)?,
                ImportStartInput {
                    run_id: parse_id(&input.run_id)?,
                    teams: input.teams,
                    user_mapping: user_mapping(input.user_mapping)?,
                    key_mapping: key_mapping(input.key_mapping)?,
                    dry_run: input.dry_run,
                    delta: input.delta,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(run.into())
    }

    async fn import_pause(
        &self,
        ctx: &Context<'_>,
        run_id: String,
    ) -> async_graphql::Result<ImportRunGql> {
        let run = app(ctx)?
            .import_pause(request_ctx(ctx)?, parse_id(&run_id)?)
            .await
            .map_err(gql_err)?;
        Ok(run.into())
    }

    async fn import_resume(
        &self,
        ctx: &Context<'_>,
        run_id: String,
    ) -> async_graphql::Result<ImportRunGql> {
        let run = app(ctx)?
            .import_resume(request_ctx(ctx)?, parse_id(&run_id)?)
            .await
            .map_err(gql_err)?;
        Ok(run.into())
    }

    async fn import_cancel(
        &self,
        ctx: &Context<'_>,
        run_id: String,
    ) -> async_graphql::Result<ImportRunGql> {
        let run = app(ctx)?
            .import_cancel(request_ctx(ctx)?, parse_id(&run_id)?)
            .await
            .map_err(gql_err)?;
        Ok(run.into())
    }
}
