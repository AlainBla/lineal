//! Planungs-Automationen (WP34): Auto-Archiv von Cycles und Projekten (docs/04 §1.8, §2.12),
//! Vorwarnung und Benachrichtigung zum Auto-Archiv von Issues (docs/03 §9.2).
//!
//! Läufe wie die Jobs in `triage::jobs`: eine Systemtransaktion listet die Teams, jede Einheit
//! läuft in einer eigenen Transaktion. Alle Läufe sind idempotent (Zielzustand wird in der
//! Transaktion erneut geprüft) und arbeiten in Batches.

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::AppResult;
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::issue::HistorySource;
use crate::usecases::cycle::engine::{self, Env};
use crate::usecases::issue::history::{self, Delta};
use crate::usecases::notification::fanout::Fan;
use crate::usecases::triage::jobs::JobStats;
use chrono::{DateTime, Duration, Months, Utc};
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{IssueId, OrgId, TeamId, UserId};
use std::collections::HashSet;

const BATCH: i64 = 100;
const MAX_BATCHES: usize = 20;
/// Vorwarnzeit vor der Auto-Archivierung eines Issues.
pub const AUTO_ARCHIVE_WARNING_DAYS: i64 = 7;

fn system_ctx(org: OrgId, job: &str) -> RequestCtx {
    RequestCtx::system(org, format!("job-{job}"))
}

fn months(m: i32) -> Months {
    Months::new(u32::try_from(m).unwrap_or(0))
}

fn fail(stats: &mut JobStats, scope: &str, e: impl std::fmt::Display) {
    tracing::warn!(scope, error = %e, "automation job unit failed");
    stats.errors.push(format!("{scope}: {e}"));
}

/// Schreibt die Benachrichtigung `kind` an den Ersteller eines Issues (nicht Inbox-prioritär;
/// der Fan-out prüft Sichtbarkeit und Einstellungen). Idempotent über `dedup`.
pub(crate) async fn notify_issue_creator(
    app: &App,
    tx: &mut dyn Tx,
    issue: IssueId,
    kind: &'static str,
    dedup: &str,
    now: DateTime<Utc>,
) -> AppResult<usize> {
    let Some(facts) = tx.notifications().issue_facts(issue).await? else {
        return Ok(0);
    };
    let Some(creator) = facts.creator_id else {
        return Ok(0);
    };
    let mut fan = Fan::new(tx, now, None).await?;
    fan.reset_for_issue_reminder(facts.id, facts.team_id);
    fan.add(creator, kind);
    fan.deliver(app.ids.as_ref(), dedup).await
}

/// Beendet ein Triage-Snooze des Issues, wenn `actor` (ein anderer als der Snoozende) aktiv wird,
/// z. B. mit einem neuen Kommentar (docs/03 §10.3: „neue Aktivität“). Schreibt History
/// (`snoozedUntilAt` → null) und `IssueUpdated`. Liefert, ob ein Snooze beendet wurde.
pub(crate) async fn wake_snoozed_issue(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    issue_id: IssueId,
    actor: Option<UserId>,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    // Zuerst ohne Sperre lesen: Nur ein tatsächlich gesnoozetes Issue wird gesperrt und
    // geändert. Eine Zeilensperre bei jedem Kommentar verklemmt sich mit parallelen
    // Fremdschlüssel-Inserts und mit dem endgültigen Löschen des Issues (B1.01/B1.13).
    match tx.issues().get(issue_id, false).await? {
        Some(i) if i.snoozed_until_at.is_some() => {}
        _ => return Ok(false),
    }
    let Some(mut issue) = tx.issues().get(issue_id, true).await? else {
        return Ok(false);
    };
    let Some(until) = issue.snoozed_until_at else {
        return Ok(false);
    };
    if issue.snoozed_by_id.is_some() && issue.snoozed_by_id == actor {
        return Ok(false);
    }
    let mut delta = Delta::default();
    delta.set(
        "snoozedUntilAt",
        serde_json::Value::String(until.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        serde_json::Value::Null,
    );
    issue.snoozed_until_at = None;
    issue.snoozed_by_id = None;
    let saved = tx.issues().save(&issue, now).await?;
    history::record(
        tx,
        uuid::Uuid::now_v7(),
        &saved,
        actor,
        HistorySource::System,
        &delta,
        now,
    )
    .await?;
    tx.outbox()
        .push(NewEvent::from_ctx(
            DomainEvent::IssueUpdated {
                issue_id: saved.id,
                team_id: saved.team_id,
                changed: vec!["snoozedUntilAt".into()],
                mentioned_user_ids: vec![],
                previous: None,
            },
            ctx,
        ))
        .await?;
    Ok(true)
}

impl App {
    /// Endgültiges Löschen aus dem Papierkorb (docs/06 §1.10, docs/04 §2.12): Dokumente und
    /// Projekte, die länger als `limits.trashRetentionDays` im Papierkorb liegen, samt Inhalt und
    /// Snapshots; außerdem Inhalte von Dokumenten, die per Fremdschlüssel mit ihrem Elternobjekt
    /// (Issue-Purge, Team-Purge, Projekt-Purge) gelöscht wurden. Liefert die Zahl gelöschter
    /// Dokumente und Projekte.
    pub async fn run_trash_purge(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let orgs = sys.notifications().list_org_ids().await?;
        drop(sys);
        let mut stats = JobStats::default();
        for org in orgs {
            match self.trash_purge_org(OrgId(org), now).await {
                Ok(n) => stats.processed += n,
                Err(e) => fail(&mut stats, &format!("trash_purge org {org}"), e),
            }
        }
        Ok(stats)
    }

    async fn trash_purge_org(&self, org: OrgId, now: DateTime<Utc>) -> AppResult<u64> {
        let ctx = system_ctx(org, "trash-purge");
        let mut total = 0;
        for _ in 0..MAX_BATCHES {
            let mut tx = self.store.begin(org).await?;
            let snap = PolicySnapshot::load(&mut *tx, &ctx).await?;
            let days = snap.org.limits.trash_retention_days.clamp(1, 3650);
            let Some(cutoff) = now.checked_sub_signed(Duration::days(days)) else {
                return Ok(total);
            };
            let projects = tx.triage().purge_trashed_projects(cutoff, BATCH).await?;
            let docs = tx.documents().purge_trashed(cutoff, BATCH).await?;
            let swept = tx.documents().sweep_orphan_contents(BATCH * 4).await?;
            tx.commit().await?;
            total += (projects.len() + docs.len()) as u64;
            let full = projects.len() as i64 == BATCH || docs.len() as i64 == BATCH || swept > 0;
            if !full {
                return Ok(total);
            }
        }
        Ok(total)
    }

    /// Auto-Archiv von Cycles und Projekten: abgeschlossene Cycles, deren `completedAt` länger als
    /// `autoArchivePeriod` Monate zurückliegt, und Projekte nach docs/04 §2.12. Liefert die Zahl
    /// archivierter Cycles und Projekte.
    pub async fn run_planning_auto_archive(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let teams = sys.triage().teams_with_auto_archive(now).await?;
        drop(sys);
        let mut stats = JobStats::default();
        for t in teams {
            match self
                .planning_auto_archive_team(t.org_id, t.team_id, now)
                .await
            {
                Ok(n) => stats.processed += n,
                Err(e) => fail(
                    &mut stats,
                    &format!("planning_auto_archive {}", t.team_id),
                    e,
                ),
            }
        }
        Ok(stats)
    }

    async fn planning_auto_archive_team(
        &self,
        org: OrgId,
        team_id: TeamId,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let ctx = system_ctx(org, "auto-archive");
        let mut total = 0;
        for _ in 0..MAX_BATCHES {
            let mut tx = self.store.begin(org).await?;
            let snap = PolicySnapshot::load(&mut *tx, &ctx).await?;
            let Some(team) = snap.tree.get(team_id) else {
                return Ok(total);
            };
            let Some(cutoff) = now.checked_sub_months(months(team.auto_archive_period)) else {
                return Ok(total);
            };
            let non_inh: HashSet<TeamId> = tx
                .cycles()
                .teams_not_inheriting()
                .await?
                .into_iter()
                .collect();
            let env = Env {
                tree: &snap.tree,
                non_inheriting: &non_inh,
                now,
                ctx: &ctx,
                ids: self.ids.as_ref(),
            };
            let cycles = tx
                .cycles()
                .auto_archive_cycle_candidates(team_id, cutoff, BATCH)
                .await?;
            let projects = tx
                .cycles()
                .auto_archive_project_candidates(team_id, cutoff, BATCH)
                .await?;
            if cycles.is_empty() && projects.is_empty() {
                return Ok(total);
            }
            let full = cycles.len() as i64 == BATCH || projects.len() as i64 == BATCH;
            for id in cycles {
                // Zugeordnete Issues vorher lösen (docs/04 §1.8), dann archivieren.
                for issue in tx.cycles().all_issue_ids(id).await? {
                    let t = tx
                        .cycles()
                        .set_issue_cycle(issue, Some(id), None, now)
                        .await?;
                    engine::emit_issue_cycle(&mut *tx, &env, issue, t).await?;
                }
                tx.cycles().auto_archive(id, now).await?;
                engine::emit_cycle_updated(&mut *tx, &env, id, team_id).await?;
                total += 1;
            }
            for id in projects {
                if tx.cycles().auto_archive_project(id, now).await? {
                    tx.outbox()
                        .push(NewEvent::new(
                            DomainEvent::ProjectUpdated { project_id: id },
                            EventOrigin::System,
                        ))
                        .await?;
                    total += 1;
                }
            }
            tx.commit().await?;
            if !full {
                return Ok(total);
            }
        }
        Ok(total)
    }

    /// Vorwarnung (docs/03 §9.2): informiert den Ersteller, dass sein Issue innerhalb der nächsten
    /// [`AUTO_ARCHIVE_WARNING_DAYS`] Tage automatisch archiviert wird. Je Issue und
    /// Änderungsstand genau einmal.
    pub async fn run_auto_archive_warnings(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let teams = sys.triage().teams_with_auto_archive(now).await?;
        drop(sys);
        let mut stats = JobStats::default();
        for t in teams {
            match self.auto_archive_warn_team(t.org_id, t.team_id, now).await {
                Ok(n) => stats.processed += n,
                Err(e) => fail(
                    &mut stats,
                    &format!("auto_archive_warning {}", t.team_id),
                    e,
                ),
            }
        }
        Ok(stats)
    }

    async fn auto_archive_warn_team(
        &self,
        org: OrgId,
        team_id: TeamId,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let ctx = system_ctx(org, "auto-archive-warning");
        let mut total = 0;
        for _ in 0..MAX_BATCHES {
            let mut tx = self.store.begin(org).await?;
            let snap = PolicySnapshot::load(&mut *tx, &ctx).await?;
            let Some(team) = snap.tree.get(team_id) else {
                return Ok(total);
            };
            let Some(cutoff) = now
                .checked_sub_months(months(team.auto_archive_period))
                .and_then(|c| c.checked_add_signed(Duration::days(AUTO_ARCHIVE_WARNING_DAYS)))
            else {
                return Ok(total);
            };
            let ids = tx
                .triage()
                .auto_archive_unwarned_candidates(team_id, cutoff, BATCH)
                .await?;
            if ids.is_empty() {
                return Ok(total);
            }
            let full = ids.len() as i64 == BATCH;
            for id in ids {
                let Some(issue) = tx.issues().get(id, false).await? else {
                    continue;
                };
                let dedup = format!(
                    "autoarchive-warn:{}:{}",
                    id,
                    issue.updated_at.timestamp_millis()
                );
                let n = notify_issue_creator(
                    self,
                    &mut *tx,
                    id,
                    "issueAutoArchiveWarning",
                    &dedup,
                    now,
                )
                .await?;
                total += n as u64;
            }
            tx.commit().await?;
            if !full {
                return Ok(total);
            }
        }
        Ok(total)
    }
}
