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
use lineal_app::ports::repos::import::{ImportRun, ImportSyncRow, ReportEntry};
use lineal_app::usecases::import::{ImportScanInput, ImportStartInput};
use lineal_app::usecases::import_sync::{ImportSyncInput, SyncRunOptions};
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
            // Interner Zwischenstand des Importers (Elternkommentare), kein Teil der API.
            o.remove("deferredParents");
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
    /// Linear-Projekt-IDs (aus dem Scan); leer = alle Projekte und Issues der gewählten Teams.
    #[graphql(default)]
    pub projects: Vec<String>,
    /// Objekt `{ "<Linear-Team-Key>": "<ID eines bestehenden Lineal-Teams>" }`: dieses Linear-Team
    /// wird in das Lineal-Team importiert statt ein neues anzulegen (Sichtbarkeit und Schreibrecht
    /// werden geprüft).
    pub target_teams: Option<Json<Value>>,
}

/// Einrichtung des regelmäßigen Abgleichs (`ImportSync`). Der Linear-API-Schlüssel wird nie
/// ausgegeben; `hasKey` zeigt nur, dass einer gespeichert ist.
#[derive(SimpleObject)]
#[graphql(name = "ImportSync")]
pub struct ImportSyncGql {
    pub id: String,
    /// `off`, `hourly`, `every6h`, `daily`.
    pub interval: String,
    /// Minuten seit 00:00 UTC (nur `daily`).
    pub daily_at_minute: i32,
    pub paused: bool,
    pub teams: Vec<String>,
    pub projects: Vec<String>,
    pub target_teams: Json<Value>,
    pub user_mapping: Json<Value>,
    pub key_mapping: Json<Value>,
    pub has_key: bool,
    pub last_run_id: Option<String>,
    pub last_run_at: Option<String>,
    pub last_success_at: Option<String>,
    /// `running`, `completed`, `failed`.
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub next_run_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ImportSyncRow> for ImportSyncGql {
    fn from(r: ImportSyncRow) -> Self {
        Self {
            id: r.id.to_string(),
            interval: r.interval,
            daily_at_minute: r.daily_at_minute,
            paused: r.paused,
            teams: r.teams,
            projects: r.projects,
            target_teams: Json(r.target_teams),
            user_mapping: Json(r.user_mapping),
            key_mapping: Json(r.key_mapping),
            has_key: true,
            last_run_id: r.last_run_id.map(|i| i.to_string()),
            last_run_at: r.last_run_at.map(ts),
            last_success_at: r.last_success_at.map(ts),
            last_status: r.last_status,
            last_error: r.last_error,
            next_run_at: r.next_run_at.map(ts),
            created_at: ts(r.created_at),
            updated_at: ts(r.updated_at),
        }
    }
}

/// Eingabe `importSyncSave`. Der Schlüssel ist `secret` (nie in Fehlern oder Logs).
#[derive(InputObject)]
#[graphql(name = "ImportSyncInput")]
pub struct ImportSyncInputGql {
    /// `off`, `hourly`, `every6h`, `daily`.
    pub interval: String,
    /// Minuten seit 00:00 UTC (nur `daily`).
    pub daily_at_minute: Option<i32>,
    /// Linear-Team-Keys; leer = Teams des letzten abgeschlossenen Imports.
    #[graphql(default)]
    pub teams: Vec<String>,
    #[graphql(default)]
    pub projects: Vec<String>,
    pub target_teams: Option<Json<Value>>,
    pub user_mapping: Option<Json<Value>>,
    pub key_mapping: Option<Json<Value>>,
    /// Pflicht beim Einrichten; später nur zum Ersetzen.
    #[graphql(secret)]
    pub token: Option<String>,
}

/// Ergebnis von `importMergeLinearLabels`.
#[derive(SimpleObject)]
#[graphql(name = "ImportLabelMerge")]
pub struct ImportLabelMergeGql {
    pub merged: i32,
    pub skipped: Vec<String>,
}

/// Ergebnis von `importPlaceholderMerge`.
#[derive(SimpleObject)]
#[graphql(name = "ImportPlaceholderMerge")]
pub struct ImportPlaceholderMergeGql {
    pub success: bool,
    /// Umgehängte Verweise.
    pub moved: i32,
    /// Beim Ziel schon vorhandene gleichwertige Zeilen (entfallen).
    pub dropped: i32,
    /// Texte mit umgeschriebener `@`-Erwähnung.
    pub mentions: i32,
}

fn target_teams(v: Option<Json<Value>>) -> async_graphql::Result<BTreeMap<String, Uuid>> {
    let Some(Json(v)) = v else {
        return Ok(BTreeMap::new());
    };
    let Some(m) = v.as_object() else {
        return Err(gql_err(AppError::invalid_request(
            "targetTeams must be an object",
        )));
    };
    if m.len() > 1_000 {
        return Err(gql_err(AppError::invalid_request(
            "targetTeams is too large",
        )));
    }
    let mut out = BTreeMap::new();
    for (k, val) in m {
        let Some(id) = val.as_str() else {
            return Err(gql_err(AppError::invalid_request(
                "targetTeams values must be team ids",
            )));
        };
        out.insert(k.clone(), parse_id(id)?);
    }
    Ok(out)
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
            .import_list_runs(
                request_ctx(ctx)?,
                i64::from(crate::error::GqlResultExt::gql(
                    crate::pagination::first_arg(first, 20, 250),
                    ctx,
                )?),
            )
            .await
            .map_err(gql_err)?;
        Ok(runs.into_iter().map(Into::into).collect())
    }

    /// Die Einrichtung des regelmäßigen Abgleichs (nur Admins); `null`, wenn keine existiert.
    async fn import_sync(&self, ctx: &Context<'_>) -> async_graphql::Result<Option<ImportSyncGql>> {
        let row = app(ctx)?
            .import_sync_get(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        Ok(row.map(Into::into))
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
                i64::from(crate::error::GqlResultExt::gql(
                    crate::pagination::first_arg(first, 100, 250),
                    ctx,
                )?),
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
                    projects: input.projects,
                    target_teams: target_teams(input.target_teams)?,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(run.into())
    }

    /// Abgleich einrichten oder ändern (Intervall, Teams/Projekte, Schlüssel).
    async fn import_sync_save(
        &self,
        ctx: &Context<'_>,
        input: ImportSyncInputGql,
    ) -> async_graphql::Result<ImportSyncGql> {
        let row = app(ctx)?
            .import_sync_save(
                request_ctx(ctx)?,
                ImportSyncInput {
                    interval: input.interval,
                    daily_at_minute: input.daily_at_minute,
                    teams: input.teams,
                    projects: input.projects,
                    target_teams: target_teams(input.target_teams)?,
                    user_mapping: user_mapping(input.user_mapping)?,
                    key_mapping: key_mapping(input.key_mapping)?,
                    token: input.token,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(row.into())
    }

    /// Abgleich pausieren (`true`) oder fortsetzen (`false`).
    async fn import_sync_pause(
        &self,
        ctx: &Context<'_>,
        paused: bool,
    ) -> async_graphql::Result<ImportSyncGql> {
        let row = app(ctx)?
            .import_sync_set_paused(request_ctx(ctx)?, paused)
            .await
            .map_err(gql_err)?;
        Ok(row.into())
    }

    /// Abgleich samt gespeichertem Schlüssel löschen.
    async fn import_sync_delete(&self, ctx: &Context<'_>) -> async_graphql::Result<bool> {
        app(ctx)?
            .import_sync_delete(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        Ok(true)
    }

    /// „Jetzt abgleichen“. `reloadContent`: Texte mit Tag-artigem Inhalt neu aus Linear holen
    /// (nur lokal unveränderte Objekte); `mergeLinearLabels`: frühere `Name (Linear)`-Duplikate
    /// zusammenführen.
    async fn import_sync_run(
        &self,
        ctx: &Context<'_>,
        reload_content: Option<bool>,
        merge_linear_labels: Option<bool>,
    ) -> async_graphql::Result<ImportRunGql> {
        let run = app(ctx)?
            .import_sync_run_now(
                request_ctx(ctx)?,
                SyncRunOptions {
                    reload_content: reload_content.unwrap_or(false),
                    merge_linear_labels: merge_linear_labels.unwrap_or(false),
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(run.into())
    }

    /// Einmalige Aufräumfunktion: führt frühere `Name (Linear)`-Duplikat-Labels zusammen.
    async fn import_merge_linear_labels(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<ImportLabelMergeGql> {
        let r = app(ctx)?
            .import_merge_linear_labels(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        Ok(ImportLabelMergeGql {
            merged: i32::try_from(r.merged).unwrap_or(i32::MAX),
            skipped: r.skipped,
        })
    }

    /// Ordnet einen Import-Platzhalter einem bestehenden aktiven User zu (nur Owner/Admin).
    async fn import_placeholder_merge(
        &self,
        ctx: &Context<'_>,
        placeholder_id: String,
        target_user_id: String,
    ) -> async_graphql::Result<ImportPlaceholderMergeGql> {
        let r = app(ctx)?
            .import_placeholder_merge(
                request_ctx(ctx)?,
                parse_id(&placeholder_id)?,
                parse_id(&target_user_id)?,
            )
            .await
            .map_err(gql_err)?;
        let n = |v: u64| i32::try_from(v).unwrap_or(i32::MAX);
        Ok(ImportPlaceholderMergeGql {
            success: true,
            moved: n(r.moved),
            dropped: n(r.dropped),
            mentions: n(r.mentions),
        })
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
