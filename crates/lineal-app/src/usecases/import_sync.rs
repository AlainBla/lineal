//! usecases – regelmäßiger Abgleich Linear -> Lineal (docs/13 §12).
//!
//! Ein Owner/Admin richtet für einen abgeschlossenen Linear-Import einen Abgleich ein
//! (Intervall, Teams/Projekte, API-Schlüssel). Der Schlüssel wird mit dem Verschlüsselungsweg der
//! Import-Tokens (`encrypt_token`, AES-GCM, Bindung an die Sync-ID) gespeichert und verlässt diese
//! Datei nur als [`Secret`]. Ein periodischer Job (`lineal-import::runner::sync_tick`) legt fällige
//! Delta-Läufe an; jeder Lauf ist ein normaler `import_run` mit `options.syncId`.
//!
//! Alle Einstiege laufen durch `Action::ManageImport` (nur Workspace-Admins/Owner, Scope `admin`).

use super::audit::AuditEvent;
use super::import::{
    CREDENTIAL_TTL_DAYS, ImportStartInput, SOURCE_LINEAR, Secret, decrypt_token, encrypt_token,
    ensure_no_overlapping_run, options_json,
};
use crate::app::App;
use crate::context::{Actor, RequestCtx, Scopes};
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot, require_workspace};
use crate::ports::repos::import::{
    ImportRun, ImportRunUpdate, ImportSyncRow, ImportTable, NewImportRun,
};
use chrono::{DateTime, Duration, Timelike, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, TeamId, UserId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Erlaubte Intervalle.
pub const INTERVALS: [&str; 4] = ["off", "hourly", "every6h", "daily"];
/// Überlappung beim Delta (Uhrenabweichung zwischen Linear und Lineal).
const DELTA_MARGIN_MINUTES: i64 = 5;
/// So oft wird (spätestens) auf in Linear gelöschte/verschobene Objekte geprüft.
const REMOVAL_CHECK_EVERY_HOURS: i64 = 24;

fn invalid(field: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field }))
}

/// Eingabe zum Einrichten/Ändern.
#[derive(Debug, Clone, Default)]
pub struct ImportSyncInput {
    /// `off`, `hourly`, `every6h`, `daily`.
    pub interval: String,
    /// Minuten seit 00:00 UTC (nur `daily`).
    pub daily_at_minute: Option<i32>,
    /// Linear-Team-Keys; leer = die Teams des letzten abgeschlossenen Imports.
    pub teams: Vec<String>,
    pub projects: Vec<String>,
    pub target_teams: BTreeMap<String, Uuid>,
    pub user_mapping: BTreeMap<String, Option<Uuid>>,
    pub key_mapping: BTreeMap<String, String>,
    /// Linear-API-Schlüssel: Pflicht beim Einrichten, sonst nur zum Ersetzen.
    pub token: Option<String>,
}

/// Optionen eines einzelnen Laufs.
#[derive(Debug, Clone, Copy, Default)]
pub struct SyncRunOptions {
    /// Beschreibungen/Kommentare/Dokumente neu laden (lokal unveränderte Objekte; liest alles).
    pub reload_content: bool,
    /// Frühere `Name (Linear)`-Duplikat-Labels in das gleichnamige Label zusammenführen.
    pub merge_linear_labels: bool,
}

/// Ergebnis des Label-Aufräumens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelMergeReport {
    pub merged: u32,
    pub skipped: Vec<String>,
}

/// Nächster Termin nach `from` (`None` = kein automatischer Lauf).
pub fn next_run_after(
    interval: &str,
    daily_at_minute: i32,
    from: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match interval {
        "hourly" => Some(from + Duration::hours(1)),
        "every6h" => Some(from + Duration::hours(6)),
        "daily" => {
            let minute = u32::try_from(daily_at_minute).ok()?.min(1439);
            let today = from
                .with_hour(minute / 60)?
                .with_minute(minute % 60)?
                .with_second(0)?
                .with_nanosecond(0)?;
            Some(if today > from {
                today
            } else {
                today + Duration::days(1)
            })
        }
        _ => None,
    }
}

fn audit_details(row: &ImportSyncRow) -> Value {
    // Nie das Token: nur Einstellungen.
    json!({
        "interval": row.interval,
        "dailyAtMinute": row.daily_at_minute,
        "paused": row.paused,
        "teams": row.teams,
        "projects": row.projects.len(),
    })
}

impl App {
    /// Die Einrichtung dieses Workspace (ohne Token).
    pub async fn import_sync_get(&self, ctx: &RequestCtx) -> AppResult<Option<ImportSyncRow>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        tx.imports().get_import_sync().await
    }

    /// Richtet den Abgleich ein oder ändert ihn. Voraussetzung: ein abgeschlossener Linear-Import.
    pub async fn import_sync_save(
        &self,
        ctx: &RequestCtx,
        input: ImportSyncInput,
    ) -> AppResult<ImportSyncRow> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        if !INTERVALS.contains(&input.interval.as_str()) {
            return Err(invalid(
                "interval",
                "interval must be off, hourly, every6h or daily",
            ));
        }
        let minute = input.daily_at_minute.unwrap_or(180);
        if !(0..=1439).contains(&minute) {
            return Err(invalid(
                "dailyAtMinute",
                "dailyAtMinute must be between 0 and 1439",
            ));
        }
        if input.projects.len() > 1000 || input.teams.len() > 200 {
            return Err(invalid("teams", "too many teams or projects"));
        }
        let base = tx
            .imports()
            .latest_completed_import_run(SOURCE_LINEAR)
            .await?
            .ok_or_else(|| {
                invalid(
                    "interval",
                    "complete a Linear import first; the sync continues from it",
                )
            })?;
        let base_teams: Vec<String> = base.options["teams"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect();
        let teams = if input.teams.is_empty() {
            base_teams.clone()
        } else {
            for t in &input.teams {
                if !base_teams.is_empty() && !base_teams.contains(t) {
                    return Err(invalid(
                        "teams",
                        format!("`{t}` was not part of the completed import"),
                    ));
                }
            }
            input.teams.clone()
        };
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        for (key, team_id) in &input.target_teams {
            if !teams.contains(key) {
                return Err(invalid(
                    "targetTeams",
                    format!("`{key}` is not a synced team"),
                ));
            }
            snap.require_team(ctx, Action::ViewTeam, TeamId(*team_id))?;
            snap.require_team(ctx, Action::CreateIssue, TeamId(*team_id))?;
            snap.require_team(ctx, Action::UpdateTeamSettings, TeamId(*team_id))?;
        }
        for target in input.user_mapping.values().flatten() {
            let usable = tx
                .users()
                .get(UserId(*target))
                .await?
                .is_some_and(|u| u.active || u.status == "imported");
            if !usable {
                return Err(invalid(
                    "userMapping",
                    "a mapped Lineal user does not exist",
                ));
            }
        }
        let now = self.clock.now();
        let existing = tx.imports().get_import_sync().await?;
        let (id, created_by, created_at, is_new) = match &existing {
            Some(e) => (e.id, e.created_by, e.created_at, false),
            None => (self.ids.new_uuid(), actor_user(ctx), now, true),
        };
        let token = match input
            .token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(raw) => {
                let secret = Secret::new(raw)?;
                Some(encrypt_token(&self.config.session_secret, id, &secret)?)
            }
            None if is_new => {
                return Err(invalid(
                    "token",
                    "a Linear API key is required to set up the sync",
                ));
            }
            None => None,
        };
        let paused = existing.as_ref().is_some_and(|e| e.paused);
        let next = if paused {
            None
        } else {
            next_run_after(&input.interval, minute, now)
        };
        let row = ImportSyncRow {
            id,
            org_id: ctx.org_id.0,
            interval: input.interval,
            daily_at_minute: minute,
            paused,
            teams,
            projects: input.projects,
            target_teams: serde_json::to_value(&input.target_teams).unwrap_or_default(),
            user_mapping: serde_json::to_value(&input.user_mapping).unwrap_or_default(),
            key_mapping: serde_json::to_value(&input.key_mapping).unwrap_or_default(),
            created_by,
            last_run_id: existing.as_ref().and_then(|e| e.last_run_id),
            last_run_at: existing.as_ref().and_then(|e| e.last_run_at),
            last_success_at: existing.as_ref().and_then(|e| e.last_success_at),
            last_status: existing.as_ref().and_then(|e| e.last_status.clone()),
            last_error: existing.as_ref().and_then(|e| e.last_error.clone()),
            last_removal_check_at: existing.as_ref().and_then(|e| e.last_removal_check_at),
            next_run_at: next,
            created_at,
            updated_at: now,
        };
        tx.imports().put_import_sync(&row, token.as_deref()).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                if is_new { "import.sync.created" } else { "import.sync.updated" },
                "import_sync",
                Some(id),
                json!({ "settings": audit_details(&row), "tokenReplaced": token.is_some() && !is_new }),
            ),
        )
        .await?;
        let saved = tx
            .imports()
            .get_import_sync()
            .await?
            .ok_or_else(|| AppError::internal("sync vanished"))?;
        tx.commit().await?;
        Ok(saved)
    }

    /// Pausiert den Abgleich oder setzt ihn fort.
    pub async fn import_sync_set_paused(
        &self,
        ctx: &RequestCtx,
        paused: bool,
    ) -> AppResult<ImportSyncRow> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        let mut row = tx
            .imports()
            .get_import_sync()
            .await?
            .ok_or_else(|| AppError::not_found("No sync is set up."))?;
        let now = self.clock.now();
        row.paused = paused;
        row.next_run_at = if paused {
            None
        } else {
            next_run_after(&row.interval, row.daily_at_minute, now)
        };
        row.updated_at = now;
        tx.imports().put_import_sync(&row, None).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                if paused {
                    "import.sync.paused"
                } else {
                    "import.sync.resumed"
                },
                "import_sync",
                Some(row.id),
                audit_details(&row),
            ),
        )
        .await?;
        let saved = tx.imports().get_import_sync().await?.unwrap_or(row);
        tx.commit().await?;
        Ok(saved)
    }

    /// Löscht die Einrichtung samt gespeichertem Schlüssel. Bereits laufende Läufe enden normal.
    pub async fn import_sync_delete(&self, ctx: &RequestCtx) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        let row = tx
            .imports()
            .get_import_sync()
            .await?
            .ok_or_else(|| AppError::not_found("No sync is set up."))?;
        tx.imports().delete_import_sync(row.id).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "import.sync.deleted",
                "import_sync",
                Some(row.id),
                audit_details(&row),
            ),
        )
        .await?;
        tx.commit().await
    }

    /// „Jetzt abgleichen“: startet sofort einen Lauf (Konflikt, wenn schon einer aktiv ist).
    pub async fn import_sync_run_now(
        &self,
        ctx: &RequestCtx,
        opts: SyncRunOptions,
    ) -> AppResult<ImportRun> {
        let sync = {
            let mut tx = self.store.begin(ctx.org_id).await?;
            require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
            tx.imports()
                .get_import_sync()
                .await?
                .ok_or_else(|| AppError::not_found("No sync is set up."))?
        };
        self.import_sync_enqueue(ctx.org_id, sync.id, opts, Some(ctx))
            .await?
            .ok_or_else(|| AppError::conflict("another import run is active for the same teams"))
    }

    /// Legt einen Delta-Lauf an (Status `running`; den Rest erledigt der Runner). `None`, wenn
    /// ein geplanter Lauf übersprungen wurde (aktiver Lauf, Team-Sperre aus B7.08).
    pub async fn import_sync_enqueue(
        &self,
        org: OrgId,
        sync_id: Uuid,
        opts: SyncRunOptions,
        manual: Option<&RequestCtx>,
    ) -> AppResult<Option<ImportRun>> {
        let mut tx = self.store.begin(org).await?;
        // Gleiche Sperre wie `import_start`: Prüfung und Start sind unteilbar.
        tx.advisory_lock("import_start", None).await?;
        let sync = tx
            .imports()
            .get_import_sync()
            .await?
            .filter(|s| s.id == sync_id)
            .ok_or_else(|| AppError::not_found("No sync is set up."))?;
        let skip_or = |msg: &str| -> AppResult<Option<ImportRun>> {
            if manual.is_some() {
                Err(AppError::conflict(msg.to_owned()))
            } else {
                Ok(None)
            }
        };
        if tx.imports().active_sync_run(sync.id).await?.is_some() {
            return skip_or("a sync run is already active");
        }
        // Der Einrichtende muss noch Admin/Owner sein (der Lauf läuft in seinem Namen).
        let starter = sync
            .created_by
            .map(UserId)
            .ok_or_else(|| AppError::forbidden("the sync has no owner"))?;
        let starter_ok = tx.users().get(starter).await?.is_some_and(|u| {
            u.active && u.status == "active" && matches!(u.role, UserRole::Owner | UserRole::Admin)
        });
        if !starter_ok {
            return Err(AppError::forbidden(
                "the user who set up the sync is no longer an administrator; set the sync up again",
            ));
        }
        let blob = tx
            .imports()
            .get_import_sync_token(sync.id)
            .await?
            .ok_or_else(|| AppError::internal("the stored Linear API key is missing"))?;
        let token = decrypt_token(&self.config.session_secret, sync.id, &blob)?;
        let now = self.clock.now();
        let base = tx
            .imports()
            .latest_completed_import_run(SOURCE_LINEAR)
            .await?;
        let from = [
            sync.last_success_at,
            base.as_ref().and_then(|r| r.started_at),
        ]
        .into_iter()
        .flatten()
        .max()
        .map(|t| t - Duration::minutes(DELTA_MARGIN_MINUTES));
        if from.is_none() && !opts.reload_content {
            return Err(AppError::conflict(
                "there is no completed import to continue from",
            ));
        }
        let updated_after = if opts.reload_content { None } else { from };
        let detect_removed = manual.is_some()
            || sync
                .last_removal_check_at
                .is_none_or(|t| now - t >= Duration::hours(REMOVAL_CHECK_EVERY_HOURS));
        let teams = sync.teams.clone();
        let input = ImportStartInput {
            run_id: Uuid::nil(),
            teams: teams.clone(),
            user_mapping: serde_json::from_value(sync.user_mapping.clone()).unwrap_or_default(),
            key_mapping: serde_json::from_value(sync.key_mapping.clone()).unwrap_or_default(),
            dry_run: false,
            delta: true,
            projects: sync.projects.clone(),
            target_teams: serde_json::from_value(sync.target_teams.clone()).unwrap_or_default(),
        };
        let mut options = options_json(&input, &teams, updated_after, Some(starter.0));
        if let Some(o) = options.as_object_mut() {
            o.insert("syncId".into(), json!(sync.id));
            o.insert("reloadContent".into(), json!(opts.reload_content));
            o.insert("mergeLinearLabels".into(), json!(opts.merge_linear_labels));
            o.insert("detectRemoved".into(), json!(detect_removed));
            o.insert("manual".into(), json!(manual.is_some()));
        }
        let run_id = self.ids.new_uuid();
        let run = tx
            .imports()
            .create_import_run(&NewImportRun {
                id: run_id,
                source: SOURCE_LINEAR.to_owned(),
                status: "planned".to_owned(),
                options: json!({}),
                started_by: Some(starter.0),
                now,
            })
            .await?;
        // Nie parallel zum selben Team (B7.08).
        if let Err(e) = ensure_no_overlapping_run(&mut *tx, &run, &teams).await {
            drop(tx);
            return if manual.is_some() { Err(e) } else { Ok(None) };
        }
        let blob = encrypt_token(&self.config.session_secret, run_id, &token)?;
        tx.imports()
            .put_import_credential(run_id, &blob, now + Duration::days(CREDENTIAL_TTL_DAYS))
            .await?;
        tx.imports()
            .update_import_run(
                run_id,
                &ImportRunUpdate {
                    status: Some("running".into()),
                    options: Some(options),
                    checkpoint: Some(json!({})),
                    started_at: Some(Some(now)),
                    now: Some(now),
                    ..Default::default()
                },
            )
            .await?;
        let next = if sync.paused {
            None
        } else {
            next_run_after(&sync.interval, sync.daily_at_minute, now)
        };
        tx.imports()
            .sync_mark_started(sync.id, run_id, now, next)
            .await?;
        let audit_ctx = manual
            .cloned()
            .unwrap_or_else(|| RequestCtx::system(org, "import-sync"));
        self.record_audit(
            &mut *tx,
            &audit_ctx,
            AuditEvent::new(
                "import.sync.run_started",
                "import_run",
                Some(run_id),
                json!({
                    "syncId": sync.id,
                    "manual": manual.is_some(),
                    "reloadContent": opts.reload_content,
                    "mergeLinearLabels": opts.merge_linear_labels,
                }),
            ),
        )
        .await?;
        let run = tx
            .imports()
            .get_import_run(run_id)
            .await?
            .ok_or_else(|| AppError::internal("run vanished"))?;
        tx.commit().await?;
        Ok(Some(run))
    }

    /// Takt des Planers (Job): legt für alle fälligen Einrichtungen Läufe an. Fehler einer
    /// Einrichtung (z. B. Einrichtender kein Admin mehr) werden dort vermerkt und stoppen die
    /// anderen nicht. Liefert die Zahl gestarteter Läufe.
    pub async fn import_sync_tick(&self, now: DateTime<Utc>) -> AppResult<u32> {
        let due = {
            let mut tx = self.store.begin_system().await?;
            tx.imports().due_import_syncs(now).await?
        };
        let mut started = 0;
        for sync in due {
            let org = OrgId(sync.org_id);
            // Zuerst den nächsten Termin vormerken: ein Fehler löst sonst jeden Takt neu aus.
            let next = next_run_after(&sync.interval, sync.daily_at_minute, now);
            {
                let mut tx = self.store.begin(org).await?;
                let mut row = sync.clone();
                row.next_run_at = next;
                row.updated_at = now;
                tx.imports().put_import_sync(&row, None).await?;
                tx.commit().await?;
            }
            match self
                .import_sync_enqueue(org, sync.id, SyncRunOptions::default(), None)
                .await
            {
                Ok(Some(_)) => started += 1,
                Ok(None) => {
                    tracing::info!(sync = %sync.id, "scheduled sync skipped (run active or team busy)")
                }
                Err(e) => {
                    tracing::warn!(sync = %sync.id, error = %e.message, "scheduled sync could not start");
                    let _ = self
                        .import_sync_record_failure(org, sync.id, None, &e.message)
                        .await;
                }
            }
        }
        Ok(started)
    }

    /// Verbucht das Ende eines Sync-Laufs (`completed`/`failed`) und benachrichtigt bei Fehlern den
    /// Einrichtenden. Läufe ohne `options.syncId` bleiben unberührt.
    pub async fn import_sync_run_finished(&self, org: OrgId, run_id: Uuid) -> AppResult<()> {
        let mut tx = self.store.begin(org).await?;
        let Some(run) = tx.imports().get_import_run(run_id).await? else {
            return Ok(());
        };
        let Some(sync_id) = run.options["syncId"]
            .as_str()
            .and_then(|s| s.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        drop(tx);
        match run.status.as_str() {
            "completed" => {
                let mut tx = self.store.begin(org).await?;
                let removal = (run.options["detectRemoved"].as_bool() == Some(true))
                    .then(|| run.finished_at.unwrap_or_else(|| self.clock.now()));
                tx.imports()
                    .sync_mark_result(
                        sync_id,
                        Some(run_id),
                        "completed",
                        None,
                        run.started_at,
                        removal,
                    )
                    .await?;
                tx.commit().await
            }
            "failed" => {
                let msg = run.error.clone().unwrap_or_else(|| "the run failed".into());
                self.import_sync_record_failure(org, sync_id, Some(run_id), &msg)
                    .await
            }
            _ => Ok(()),
        }
    }

    async fn import_sync_record_failure(
        &self,
        org: OrgId,
        sync_id: Uuid,
        run_id: Option<Uuid>,
        message: &str,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(org).await?;
        let Some(sync) = tx
            .imports()
            .get_import_sync()
            .await?
            .filter(|s| s.id == sync_id)
        else {
            return Ok(());
        };
        let msg: String = message.chars().take(500).collect();
        tx.imports()
            .sync_mark_result(sync_id, run_id, "failed", Some(&msg), None, None)
            .await?;
        if let Some(user) = sync.created_by {
            let key = run_id.map_or_else(
                || format!("{}", self.clock.now().timestamp()),
                |r| r.to_string(),
            );
            super::notification::fanout::insert_direct(
                &mut *tx,
                self.ids.as_ref(),
                self.clock.now(),
                UserId(user),
                "importSyncFailed",
                None,
                None,
                format!("importSyncFailed:{sync_id}:{key}"),
            )
            .await?;
        }
        tx.commit().await
    }

    /// Führt frühere `Name (Linear)`-Duplikat-Labels in das gleichnamige Label (gleiche Ebene)
    /// zusammen (vorhandener Label-Merge: Issues, Templates, Views wandern um; Mapping folgt).
    /// Nur mit ausdrücklichem Aufruf (Admin) bzw. der Sync-Option `mergeLinearLabels`.
    pub async fn import_merge_linear_labels(
        &self,
        ctx: &RequestCtx,
    ) -> AppResult<LabelMergeReport> {
        let candidates = {
            let mut tx = self.store.begin(ctx.org_id).await?;
            require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
            tx.imports().list_linear_suffixed_labels().await?
        };
        let mut report = LabelMergeReport::default();
        for (id, base, team, parent) in candidates {
            let target = {
                let mut tx = self.store.begin(ctx.org_id).await?;
                tx.imports()
                    .find_label_by_name_at(ImportTable::IssueLabel, &base, team, parent)
                    .await?
            };
            let Some(dst) = target.filter(|d| *d != id) else {
                continue;
            };
            match self
                .merge_issue_label(ctx, &id.to_string(), &dst.to_string())
                .await
            {
                Ok(_) => {
                    let mut tx = self.store.begin(ctx.org_id).await?;
                    tx.imports().repoint_mappings("IssueLabel", id, dst).await?;
                    tx.commit().await?;
                    report.merged += 1;
                }
                Err(e) => report
                    .skipped
                    .push(format!("{base} (Linear): {}", e.message)),
            }
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "import.labels_merged",
                "import_sync",
                None,
                json!({ "merged": report.merged, "skipped": report.skipped.len() }),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(report)
    }

    /// Kontext des Einrichtenden/Starters eines Laufs (Admin/Owner, aktiv); für Teilschritte, die
    /// Use-Cases mit Rechteprüfung aufrufen (Label-Merge).
    pub async fn import_run_ctx(&self, org: OrgId, run: &ImportRun) -> AppResult<RequestCtx> {
        let starter = run.options["startedBy"]
            .as_str()
            .and_then(|s| s.parse::<Uuid>().ok())
            .or(run.started_by)
            .ok_or_else(|| AppError::forbidden("the run has no starting user"))?;
        let mut tx = self.store.begin(org).await?;
        let user = tx
            .users()
            .get(UserId(starter))
            .await?
            .filter(|u| u.active && u.status == "active")
            .ok_or_else(|| {
                AppError::forbidden("the user who started the run is no longer active")
            })?;
        Ok(RequestCtx {
            org_id: org,
            actor: Actor::User {
                id: user.id,
                role: user.role,
            },
            request_id: "import-sync".into(),
            scopes: Scopes::all(),
            team_scope: None,
            via_session: false,
            client_ip: None,
            credential_kind: None,
        })
    }
}

fn actor_user(ctx: &RequestCtx) -> Option<Uuid> {
    match ctx.actor {
        Actor::User { id, .. } | Actor::App { id } => Some(id.0),
        Actor::System => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn next_run_per_interval() {
        let now = at("2026-10-10T08:30:15Z");
        assert_eq!(next_run_after("off", 0, now), None);
        assert_eq!(
            next_run_after("hourly", 0, now),
            Some(at("2026-10-10T09:30:15Z"))
        );
        assert_eq!(
            next_run_after("every6h", 0, now),
            Some(at("2026-10-10T14:30:15Z"))
        );
        // täglich: heute, wenn die Uhrzeit noch bevorsteht, sonst morgen
        assert_eq!(
            next_run_after("daily", 9 * 60, now),
            Some(at("2026-10-10T09:00:00Z"))
        );
        assert_eq!(
            next_run_after("daily", 8 * 60 + 30, now),
            Some(at("2026-10-11T08:30:00Z"))
        );
        assert_eq!(
            next_run_after("daily", 185, now),
            Some(at("2026-10-11T03:05:00Z"))
        );
        assert_eq!(
            next_run_after("daily", 1439, now),
            Some(at("2026-10-10T23:59:00Z"))
        );
    }
}
