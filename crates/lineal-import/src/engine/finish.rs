//! Ablaufsteuerung (Phasen, Checkpoints), Phase 8 (Abschluss), Lauf-Abschluss und Scan.

use super::node::N;
use super::{HResult, Halt, ImportEngine, RunOutcome, Session, Step, src_err};
use crate::port::SourceEntity;
use chrono::Utc;
use lineal_app::RequestCtx;
use lineal_app::error::AppResult;
use lineal_app::ports::repos::import::{ImportRunUpdate, ImportTable};
use lineal_app::usecases::audit::AuditEvent;
use lineal_domain::ids::OrgId;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Höchstzahl gelesener Seiten je Entität beim Scan (Schutz vor endlosen Quellen).
const SCAN_MAX_PAGES: usize = 500;

impl Session<'_, '_> {
    pub(crate) fn cp(&self, step: Step, cursor: Option<String>) -> Value {
        json!({ "phase": step.phase(), "step": step.name(), "cursor": cursor })
    }

    /// Alle Phasen ab dem Checkpoint.
    pub(crate) async fn drive(&mut self) -> HResult<RunOutcome> {
        self.init().await?;
        let (start, start_cursor) = if self.dry {
            (0, None)
        } else {
            let name = self.run.checkpoint["step"].as_str().unwrap_or("");
            (
                Step::from_name(name).unwrap_or(0),
                self.run.checkpoint["cursor"].as_str().map(str::to_owned),
            )
        };
        for idx in start..Step::ALL.len() {
            let step = Step::ALL[idx];
            let cursor = if idx == start {
                start_cursor.clone()
            } else {
                None
            };
            if idx == start || step.phase() != Step::ALL[idx - 1].phase() {
                self.begin_unit().await?;
                self.lock_teams().await?;
                self.end_unit(None).await?;
            }
            tracing::info!(run = %self.run.id, step = step.name(), phase = step.phase(), "import step");
            self.run_step(step, cursor).await?;
            let next = Step::ALL.get(idx + 1);
            self.begin_unit().await?;
            let cp = match next {
                Some(n) => self.cp(*n, None),
                None => json!({ "phase": 8, "step": "Done", "cursor": null }),
            };
            self.end_unit(Some(cp)).await?;
            if let Some(stop) = self.eng.config.stop_after_phase
                && step.phase() == stop
                && next.is_none_or(|n| n.phase() > stop)
            {
                return Err(Halt::Stop(stop));
            }
        }
        Ok(if self.dry {
            RunOutcome::DryRunFinished
        } else {
            RunOutcome::Completed
        })
    }

    async fn run_step(&mut self, step: Step, cursor: Option<String>) -> HResult<()> {
        match step {
            Step::Users => self.step_users().await,
            Step::Teams => self.step_teams().await,
            Step::Memberships => self.step_memberships(cursor).await,
            Step::States => self.step_states(cursor).await,
            Step::Labels => self.step_labels().await,
            Step::ProjectStatuses => self.step_project_statuses(cursor).await,
            Step::ProjectLabels => self.step_project_labels().await,
            Step::Templates => self.step_templates().await,
            Step::Cycles => self.step_cycles(cursor).await,
            Step::Initiatives => self.step_initiatives().await,
            Step::Projects => self.step_projects(cursor).await,
            Step::Milestones => self.step_milestones(cursor).await,
            Step::Issues => self.step_issues(cursor).await,
            Step::IssueParents => self.step_issue_parents().await,
            Step::IssueRelations => self.step_relations(cursor).await,
            Step::Comments => self.step_comments(cursor).await,
            Step::IssueHistory => self.step_history(cursor).await,
            Step::IssueSubscriptions => self.step_subscriptions(cursor).await,
            Step::Attachments => self.step_attachments(cursor).await,
            Step::EmbeddedFiles => self.step_embedded_files(cursor).await,
            Step::Documents => self.step_documents(cursor).await,
            Step::ProjectUpdates => self.step_project_updates(cursor).await,
            Step::InitiativeUpdates => self.step_initiative_updates(cursor).await,
            Step::Views => {
                self.note(
                    "info",
                    "CustomView",
                    None,
                    "custom views and favorites are not imported by this version of the importer",
                );
                Ok(())
            }
            Step::Finalize => self.step_finalize().await,
        }
    }

    async fn mapped_team_ids(&mut self) -> AppResult<Vec<Uuid>> {
        let source = self.source().to_owned();
        let mut ids = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page = self
                .tx()?
                .imports()
                .list_mappings(&source, "Team", after.as_deref(), 500)
                .await?;
            if page.is_empty() {
                return Ok(ids);
            }
            after = page.last().map(|m| m.source_id.clone());
            ids.extend(page.iter().map(|m| m.target_id));
        }
    }

    /// Phase 8: Zähler setzen, Verifikation, Sperren aufheben (im Lauf-Abschluss).
    async fn step_finalize(&mut self) -> HResult<()> {
        self.begin_unit().await?;
        let team_ids = self.mapped_team_ids().await?;
        self.tx()?
            .imports()
            .finalize_import_counters(&team_ids)
            .await?;
        let source = self.source().to_owned();
        let mapped: BTreeMap<String, i64> = self
            .tx()?
            .imports()
            .count_mappings(&source)
            .await?
            .into_iter()
            .collect();
        let checks: [(&str, ImportTable); 11] = [
            ("User", ImportTable::User),
            ("Team", ImportTable::Team),
            ("Issue", ImportTable::Issue),
            ("IssueLabel", ImportTable::IssueLabel),
            ("Cycle", ImportTable::Cycle),
            ("Project", ImportTable::Project),
            ("ProjectMilestone", ImportTable::ProjectMilestone),
            ("Initiative", ImportTable::Initiative),
            ("Comment", ImportTable::Comment),
            ("Attachment", ImportTable::Attachment),
            ("Document", ImportTable::Document),
        ];
        let mut verification = serde_json::Map::new();
        for (ty, table) in checks {
            let m = mapped.get(ty).copied().unwrap_or(0);
            let present = self
                .tx()?
                .imports()
                .count_mapped_rows(&source, ty, table)
                .await?;
            let scanned = self.run.stats["scan"]["counts"][ty].as_i64();
            if present < m {
                self.note(
                    "warning",
                    ty,
                    None,
                    format!(
                        "verification: {m} objects are mapped but only {present} exist in Lineal"
                    ),
                );
            }
            verification.insert(
                ty.to_owned(),
                json!({ "mapped": m, "present": present, "scanned": scanned }),
            );
        }
        if let Some(stats) = self.run.stats.as_object_mut() {
            stats.insert("verification".into(), Value::Object(verification));
        } else {
            self.run.stats = json!({ "verification": verification });
        }
        self.end_unit(None).await?;
        Ok(())
    }

    /// Lauf beenden: Status, Zugangsdaten, Sperren und Lease je nach Ausgang.
    pub(crate) async fn finish(mut self, result: HResult<RunOutcome>) -> AppResult<RunOutcome> {
        // Eine angefangene Einheit wird verworfen (Rollback beim Drop).
        let dry = self.dry;
        *self.tx.slot() = None;
        let now = self.eng.app.clock.now();
        let run_id = self.run.id;
        let stats = self.stats_json();
        let report = std::mem::take(&mut self.report);
        let mut tx = self.eng.app.store.begin(self.org).await?;
        let outcome = match result {
            Ok(done) => {
                if !report.is_empty() {
                    tx.imports().add_report_entries(run_id, &report).await?;
                }
                if dry {
                    tx.imports()
                        .update_import_run(
                            run_id,
                            &ImportRunUpdate {
                                status: Some("planned".into()),
                                stats: Some(stats),
                                checkpoint: Some(json!({})),
                                now: Some(now),
                                ..Default::default()
                            },
                        )
                        .await?;
                } else {
                    tx.imports()
                        .update_import_run(
                            run_id,
                            &ImportRunUpdate {
                                status: Some("completed".into()),
                                stats: Some(stats),
                                finished_at: Some(Some(now)),
                                error: Some(None),
                                now: Some(now),
                                ..Default::default()
                            },
                        )
                        .await?;
                    tx.imports().delete_import_credential(run_id).await?;
                    unlock_teams(&mut *tx, &self.run.source).await?;
                    let audit_ctx = RequestCtx::system(self.org, "import");
                    self.eng
                        .app
                        .record_audit(
                            &mut *tx,
                            &audit_ctx,
                            AuditEvent::new(
                                "import.completed",
                                "import_run",
                                Some(run_id),
                                json!({ "source": self.run.source }),
                            ),
                        )
                        .await?;
                }
                done
            }
            Err(Halt::Pause) => RunOutcome::Paused,
            Err(Halt::Cancel) => {
                unlock_teams(&mut *tx, &self.run.source).await?;
                RunOutcome::Canceled
            }
            Err(Halt::Stop(n)) => RunOutcome::Stopped { after_phase: n },
            Err(Halt::Err(e)) => {
                let mut msg = e.message.clone();
                if msg.chars().count() > 500 {
                    msg = msg.chars().take(500).collect();
                }
                tracing::warn!(run = %run_id, error = %msg, "import run failed");
                tx.imports()
                    .update_import_run(
                        run_id,
                        &ImportRunUpdate {
                            status: Some("failed".into()),
                            error: Some(Some(msg.clone())),
                            stats: Some(stats),
                            finished_at: Some(Some(now)),
                            now: Some(now),
                            ..Default::default()
                        },
                    )
                    .await?;
                RunOutcome::Failed(msg)
            }
        };
        tx.imports().set_import_lease(run_id, None).await?;
        tx.commit().await?;
        Ok(outcome)
    }
}

async fn unlock_teams(tx: &mut dyn lineal_app::ports::Tx, source: &str) -> AppResult<()> {
    let mut ids = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = tx
            .imports()
            .list_mappings(source, "Team", after.as_deref(), 500)
            .await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map(|m| m.source_id.clone());
        ids.extend(page.iter().map(|m| m.target_id));
    }
    tx.imports().set_team_import_lock(&ids, None).await
}

// ---------------------------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------------------------

impl ImportEngine<'_> {
    /// Scan (Verbindung prüfen, Teams/User/Mengen lesen) für einen Lauf im Status `scanning`.
    /// Ergebnis in `import_run.stats.scan`, Status danach `planned` (bzw. `failed`).
    pub async fn scan(&self, org: OrgId, run_id: Uuid) -> AppResult<()> {
        let result = self.scan_inner(org).await;
        let now = self.app.clock.now();
        let mut tx = self.app.store.begin(org).await?;
        let update = match result {
            Ok(scan) => ImportRunUpdate {
                status: Some("planned".into()),
                stats: Some(json!({ "scan": scan })),
                error: Some(None),
                now: Some(now),
                ..Default::default()
            },
            Err(e) => ImportRunUpdate {
                status: Some("failed".into()),
                error: Some(Some(e.message.chars().take(500).collect())),
                finished_at: Some(Some(now)),
                now: Some(now),
                ..Default::default()
            },
        };
        tx.imports().update_import_run(run_id, &update).await?;
        tx.imports().set_import_lease(run_id, None).await?;
        tx.commit().await
    }

    async fn scan_inner(&self, org: OrgId) -> AppResult<Value> {
        let ws = self.source.workspace().await.map_err(src_err)?;
        let teams = self
            .source
            .collect(SourceEntity::Teams, None, SCAN_MAX_PAGES)
            .await
            .map_err(src_err)?;
        let users = self
            .source
            .collect(SourceEntity::Users, None, SCAN_MAX_PAGES)
            .await
            .map_err(src_err)?;
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        let mut issues_per_team: BTreeMap<String, i64> = BTreeMap::new();
        let mut truncated = Vec::new();
        for entity in SourceEntity::ALL {
            // Abgeleitete Entitäten (History, Abos) werden nicht gezählt (würde alle Issues erneut lesen).
            if matches!(
                entity,
                SourceEntity::IssueHistory | SourceEntity::IssueSubscriptions
            ) {
                continue;
            }
            let mut cursor = None;
            let mut n = 0i64;
            let mut pages = 0usize;
            loop {
                let page = self
                    .source
                    .page(&crate::port::PageQuery {
                        entity,
                        cursor: cursor.clone(),
                        updated_after: None,
                        limit: 100,
                    })
                    .await
                    .map_err(src_err)?;
                n += page.items.len() as i64;
                if entity == SourceEntity::Issues {
                    for v in &page.items {
                        if let Some(t) = N(v).ref_id("team") {
                            *issues_per_team.entry(t.to_owned()).or_default() += 1;
                        }
                    }
                }
                pages += 1;
                match page.next_cursor {
                    Some(c) if pages < SCAN_MAX_PAGES => cursor = Some(c),
                    Some(_) => {
                        truncated.push(entity.as_str());
                        break;
                    }
                    None => break,
                }
            }
            counts.insert(entity.as_str().to_owned(), n);
        }
        // Konflikte und Zuordnungsvorschläge.
        let mut tx = self.app.store.begin(org).await?;
        let mut team_out = Vec::new();
        let mut conflicts = Vec::new();
        for v in &teams {
            let n = N(v);
            let (Some(id), Some(key)) = (n.id(), n.text("key")) else {
                continue;
            };
            let existing = tx.imports().find_team_by_key(key).await?;
            let mapped = tx
                .imports()
                .get_mappings("linear", "Team", &[id.to_owned()])
                .await?;
            let conflict =
                existing.is_some() && !mapped.iter().any(|m| Some(m.target_id) == existing);
            if conflict {
                conflicts.push(json!({ "key": key, "reason": "key_exists" }));
            }
            team_out.push(json!({
                "id": id, "key": key, "name": n.text("name"), "private": n.bool("private").unwrap_or(false),
                "parentId": n.ref_id("parent"), "issueCount": issues_per_team.get(id).copied().unwrap_or(0),
                "conflict": conflict,
            }));
        }
        let mut user_out = Vec::new();
        for v in &users {
            let n = N(v);
            let Some(id) = n.id() else { continue };
            let matched = match n.text("email") {
                Some(e) => tx.imports().find_user_id_by_email(e).await?,
                None => None,
            };
            user_out.push(json!({
                "id": id, "name": n.text("name"), "displayName": n.text("displayName"), "email": n.text("email"),
                "admin": n.bool("admin").unwrap_or(false), "guest": n.bool("guest").unwrap_or(false),
                "active": n.bool("active").unwrap_or(true), "matchedUserId": matched,
            }));
        }
        drop(tx);
        Ok(json!({
            "scannedAt": Utc::now(),
            "workspace": { "id": ws.id, "name": ws.name, "urlKey": ws.url_key, "viewerEmail": ws.viewer_email },
            "teams": team_out,
            "users": user_out,
            "counts": counts,
            "conflicts": conflicts,
            "truncated": truncated,
        }))
    }
}
