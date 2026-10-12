//! Läufe der periodischen Jobs von WP12 (docs/10 §3): Auto-Close, Auto-Archive, Papierkorb-
//! Purge, SLA-Meldungen, Triage-Rotation und Snooze-Wiedervorlage in der Triage.
//!
//! Muster wie `run_cycle_maintenance`: eine Systemtransaktion (`begin_system`) listet die
//! betroffenen Workspaces bzw. Teams mandantenübergreifend, danach läuft jede Arbeitseinheit in
//! einer eigenen Transaktion `begin(org)` mit System-Kontext. Alle Läufe sind idempotent
//! (Zielzustand wird in der Transaktion erneut geprüft) und arbeiten in Batches; Fehler einer
//! Einheit stoppen die anderen nicht.

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::AppResult;
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::issue::{HistorySource, Issue};
use crate::ports::repos::team::Team;
use crate::usecases::issue::fields::{change_state, first_state_of};
use crate::usecases::issue::history::{self, Delta};
use crate::usecases::triage::automation::{is_closed, responsible_users};
use chrono::{DateTime, Months, NaiveDate, Utc};
use chrono_tz::Tz;
use lineal_domain::enums::StateType;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{IssueId, OrgId, TeamId};
use lineal_domain::rules::state::TransitionSource;
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Größe eines Batches (docs/10 §3: „z. B. 500“; kleiner, weil jedes Issue mehrere Zeilen schreibt).
const BATCH: i64 = 200;
/// Obergrenze der Batches je Einheit und Lauf (der nächste Lauf macht weiter).
const MAX_BATCHES: usize = 50;
/// Tiefe, bis zu der Sub-Issues auf Auto-Close-Fähigkeit geprüft werden (= maximale Verschachtelung).
const MAX_CHILD_DEPTH: usize = 11;
/// Fenster, in dem geänderte Issues auf SLA-Regeln geprüft werden (Vielfaches des Job-Intervalls).
const SLA_SWEEP_WINDOW_MINUTES: i64 = 15;

/// Ergebnis eines Laufs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JobStats {
    /// Anzahl geänderter Datensätze.
    pub processed: u64,
    /// Fehler einzelner Einheiten (Team/Workspace).
    pub errors: Vec<String>,
}

impl JobStats {
    fn fail(&mut self, scope: &str, e: impl std::fmt::Display) {
        tracing::warn!(scope, error = %e, "triage job unit failed");
        self.errors.push(format!("{scope}: {e}"));
    }
}

fn months(team_months: i32) -> Months {
    Months::new(u32::try_from(team_months).unwrap_or(0))
}

fn val_uuid(id: Uuid) -> Value {
    Value::String(id.to_string())
}

fn system_ctx(org: OrgId, job: &str) -> RequestCtx {
    RequestCtx::system(org, format!("job-{job}"))
}

async fn emit_system(tx: &mut dyn Tx, event: DomainEvent) -> AppResult<()> {
    tx.outbox()
        .push(NewEvent::new(event, EventOrigin::System))
        .await?;
    Ok(())
}

fn team_tz(team: &Team) -> Tz {
    team.timezone.parse().unwrap_or(chrono_tz::UTC)
}

/// Heute in der Zeitzone des Teams.
fn today_of(team: &Team, now: DateTime<Utc>) -> NaiveDate {
    now.with_timezone(&team_tz(team)).date_naive()
}

/// Ob ein Issue nach docs/03 §9.3 jetzt automatisch geschlossen werden dürfte
/// (ohne die Prüfung seiner Sub-Issues).
fn closable_itself(
    issue: &Issue,
    info: &crate::ports::repos::triage::AutoCloseCandidate,
    cutoff: DateTime<Utc>,
    today: NaiveDate,
    now: DateTime<Utc>,
) -> bool {
    matches!(
        issue.state_type,
        StateType::Triage | StateType::Backlog | StateType::Unstarted
    ) && issue.archived_at.is_none()
        && !issue.trashed
        && issue.updated_at < cutoff
        // harte Ausnahmen
        && !info.in_active_cycle
        && !info.in_unfinished_project
        // Aufschub
        && issue.due_date.is_none_or(|d| d < today)
        && issue.sla_breaches_at.is_none_or(|b| b <= now)
}

impl App {
    /// Auto-Close (docs/03 §9.3): schließt offene Issues (`triage`/`backlog`/`unstarted`) ohne
    /// Aktualisierung seit `autoClosePeriod` Monaten. Ausnahmen: aktiver Cycle und unfertiges
    /// Projekt (nie), künftiges Due-Date, aktives SLA und nicht schließbare Sub-Issues
    /// (Aufschub bis zum nächsten Lauf).
    pub async fn run_auto_close(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let teams = sys.triage().teams_with_auto_close(now).await?;
        drop(sys);
        let mut stats = JobStats::default();
        for t in teams {
            match self.auto_close_team(t.org_id, t.team_id, now).await {
                Ok(n) => stats.processed += n,
                Err(e) => stats.fail(&format!("auto_close team {}", t.team_id), e),
            }
        }
        Ok(stats)
    }

    async fn auto_close_team(
        &self,
        org: OrgId,
        team_id: TeamId,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let ctx = system_ctx(org, "auto-close");
        let mut total = 0;
        let mut after: Option<IssueId> = None;
        for _ in 0..MAX_BATCHES {
            let mut tx = self.store.begin(org).await?;
            let snap = PolicySnapshot::load(&mut *tx, &ctx).await?;
            let Some(team) = snap.tree.get(team_id).cloned() else {
                return Ok(total);
            };
            let Some(period) = team.auto_close_period else {
                return Ok(total);
            };
            let Some(cutoff) = now.checked_sub_months(months(period)) else {
                return Ok(total);
            };
            let target = match team.auto_close_state_id {
                Some(id) => tx.states().get(id).await?.filter(|s| {
                    s.team_id == team.id
                        && s.archived_at.is_none()
                        && s.state_type == StateType::Canceled
                }),
                None => None,
            };
            let target = match target {
                Some(t) => t,
                None => match first_state_of(&mut *tx, team.id, StateType::Canceled).await? {
                    Some(t) => t,
                    None => return Ok(total),
                },
            };
            let today = today_of(&team, now);
            let batch = tx
                .triage()
                .auto_close_candidates(team.id, cutoff, now, after, BATCH)
                .await?;
            if batch.is_empty() {
                return Ok(total);
            }
            after = batch.last().map(|c| c.issue_id);
            let full = batch.len() as i64 == BATCH;
            for cand in batch {
                if self
                    .try_auto_close(&mut *tx, &team, &target, cand.issue_id, cutoff, today, now)
                    .await?
                {
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

    #[allow(clippy::too_many_arguments)]
    async fn try_auto_close(
        &self,
        tx: &mut dyn Tx,
        team: &Team,
        target: &crate::ports::repos::state::WorkflowState,
        id: IssueId,
        cutoff: DateTime<Utc>,
        today: NaiveDate,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        // Frisch lesen und sperren: nur wenn das Issue noch offen und weiterhin inaktiv ist.
        let Some(mut issue) = tx.issues().get(id, true).await? else {
            return Ok(false);
        };
        let info = tx.triage().auto_close_info(id, now).await?;
        if !closable_itself(&issue, &info, cutoff, today, now) {
            return Ok(false);
        }
        if !self
            .children_allow_close(tx, issue.id, cutoff, today, now)
            .await?
        {
            return Ok(false);
        }
        let before = issue.clone();
        change_state(
            tx,
            team,
            &mut issue,
            target,
            TransitionSource::Update,
            None,
            now,
            false,
        )
        .await?;
        issue.auto_closed_at = Some(now);
        let saved = tx.issues().save(&issue, now).await?;
        let mut delta = Delta::default();
        delta.set(
            "stateId",
            val_uuid(before.state_id.0),
            val_uuid(saved.state_id.0),
        );
        delta.flag("autoClosed");
        history::record(
            tx,
            Uuid::now_v7(),
            &saved,
            None,
            HistorySource::Automation,
            &delta,
            now,
        )
        .await?;
        emit_system(
            tx,
            DomainEvent::IssueUpdated {
                previous: None,
                issue_id: saved.id,
                team_id: saved.team_id,
                changed: vec!["stateId".into(), "autoClosedAt".into()],
                mentioned_user_ids: vec![],
            },
        )
        .await?;
        Ok(true)
    }

    /// `any(isOpen(c) and not autoClosable(c))` aus docs/03 §9.3, iterativ über alle
    /// Nachkommen (`false`, sobald ein offenes Sub-Issue nicht schließbar ist).
    async fn children_allow_close(
        &self,
        tx: &mut dyn Tx,
        root: IssueId,
        cutoff: DateTime<Utc>,
        today: NaiveDate,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut stack: Vec<(IssueId, usize)> = vec![(root, 0)];
        let mut tz_cache: BTreeMap<TeamId, NaiveDate> = BTreeMap::new();
        while let Some((id, depth)) = stack.pop() {
            if depth >= MAX_CHILD_DEPTH {
                return Ok(false);
            }
            for c in tx.issues().children(id).await? {
                if c.archived_at.is_some() || is_closed(c.state_type) {
                    continue;
                }
                // Offenes Sub-Issue: `started` ist nie auto-schließbar, die übrigen nach denselben
                // Kriterien (mit dem „heute“ des eigenen Teams).
                let child_today = match tz_cache.get(&c.team_id) {
                    Some(d) => *d,
                    None => {
                        let d = match tx.teams().get(c.team_id).await? {
                            Some(t) => today_of(&t, now),
                            None => today,
                        };
                        tz_cache.insert(c.team_id, d);
                        d
                    }
                };
                let info = tx.triage().auto_close_info(c.id, now).await?;
                if !closable_itself(&c, &info, cutoff, child_today, now) {
                    return Ok(false);
                }
                stack.push((c.id, depth + 1));
            }
        }
        Ok(true)
    }

    /// Auto-Archive (docs/03 §9.2): archiviert lange geschlossene, inaktive Issues.
    pub async fn run_auto_archive(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let teams = sys.triage().teams_with_auto_archive(now).await?;
        drop(sys);
        let mut stats = JobStats::default();
        for t in teams {
            match self.auto_archive_team(t.org_id, t.team_id, now).await {
                Ok(n) => stats.processed += n,
                Err(e) => stats.fail(&format!("auto_archive team {}", t.team_id), e),
            }
        }
        Ok(stats)
    }

    async fn auto_archive_team(
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
            let ids = tx
                .triage()
                .auto_archive_candidates(team_id, cutoff, BATCH)
                .await?;
            if ids.is_empty() {
                return Ok(total);
            }
            let full = ids.len() as i64 == BATCH;
            for id in ids {
                // Die Kandidatenliste ist ein Schnappschuss: das Issue sperren und die
                // Archivierbarkeit nicht erneut vollständig prüfen müssen, weil die Abfrage in
                // derselben Transaktion läuft; `archive_issue` ist selbst idempotent.
                let Some(issue) = tx.issues().get(id, true).await? else {
                    continue;
                };
                if issue.archived_at.is_some() || issue.trashed {
                    continue;
                }
                tx.triage().archive_issue(id, true, now).await?;
                let mut delta = Delta::default();
                delta.flag("autoArchived");
                history::record(
                    &mut *tx,
                    Uuid::now_v7(),
                    &issue,
                    None,
                    HistorySource::Automation,
                    &delta,
                    now,
                )
                .await?;
                emit_system(
                    &mut *tx,
                    DomainEvent::IssueUpdated {
                        previous: None,
                        issue_id: id,
                        team_id: issue.team_id,
                        changed: vec!["archivedAt".into()],
                        mentioned_user_ids: vec![],
                    },
                )
                .await?;
                total += 1;
            }
            tx.commit().await?;
            if !full {
                return Ok(total);
            }
        }
        Ok(total)
    }

    /// Papierkorb-Purge (docs/03 §9.1): löscht Issues endgültig, die länger als die
    /// Aufbewahrungsfrist (`limits.trashRetentionDays`, Default 30) im Papierkorb liegen.
    /// Kommentare, Reaktionen, History, Relationen, Anhänge und Notifications fallen per
    /// Fremdschlüssel mit; Inhalte ohne Fremdschlüssel (`document_content`) räumt das Repo ab.
    pub async fn run_purge(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let orgs = sys.triage().orgs_with_trashed_issues().await?;
        drop(sys);
        let mut stats = JobStats::default();
        for org in orgs {
            match self.purge_org(org, now).await {
                Ok(n) => stats.processed += n,
                Err(e) => stats.fail(&format!("purge org {org}"), e),
            }
        }
        Ok(stats)
    }

    async fn purge_org(&self, org: OrgId, now: DateTime<Utc>) -> AppResult<u64> {
        let ctx = system_ctx(org, "purge");
        let mut total = 0;
        for _ in 0..MAX_BATCHES {
            let mut tx = self.store.begin(org).await?;
            let snap = PolicySnapshot::load(&mut *tx, &ctx).await?;
            let days = snap.org.limits.trash_retention_days.clamp(1, 3650);
            let Some(cutoff) = now.checked_sub_signed(chrono::Duration::days(days)) else {
                return Ok(total);
            };
            let ids = tx.triage().purge_candidates(cutoff, BATCH).await?;
            if ids.is_empty() {
                return Ok(total);
            }
            let full = ids.len() as i64 == BATCH;
            let mut keys = Vec::new();
            for id in ids {
                let Some(issue) = tx.issues().get(id, true).await? else {
                    continue;
                };
                if !issue.trashed || issue.trashed_at.is_none_or(|t| t > cutoff) {
                    continue;
                }
                keys.extend(tx.attachments().storage_keys_for_issue(id).await?);
                tx.triage().delete_issue_leftovers(id).await?;
                tx.issues().delete(id).await?;
                total += 1;
            }
            // Blobs erst nach dem Commit löschen und nur, wenn nichts mehr auf sie verweist.
            let orphans = tx.attachments().unreferenced_storage_keys(&keys).await?;
            tx.commit().await?;
            self.delete_blobs(&orphans).await;
            if !full {
                return Ok(total);
            }
        }
        Ok(total)
    }

    /// Triage-Rotation (docs/10 §3): speichert den aktuell Verantwortlichen jedes Teams mit
    /// Schichtplan, nur wenn er sich geändert hat.
    pub async fn run_triage_rotation(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let teams = sys.triage().responsibilities_with_schedule().await?;
        drop(sys);
        let mut stats = JobStats::default();
        for t in teams {
            match self.rotate_team(t.org_id, t.team_id, now).await {
                Ok(true) => stats.processed += 1,
                Ok(false) => {}
                Err(e) => stats.fail(&format!("triage_rotation team {}", t.team_id), e),
            }
        }
        Ok(stats)
    }

    async fn rotate_team(
        &self,
        org: OrgId,
        team_id: TeamId,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut tx = self.store.begin(org).await?;
        let Some(tr) = tx.triage().get_responsibility(team_id).await? else {
            return Ok(false);
        };
        let users = responsible_users(&mut *tx, &tr, team_id, now).await?;
        let current = users.first().copied();
        if current == tr.current_user_id {
            return Ok(false);
        }
        tx.triage().set_current_user(team_id, current, now).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Snooze-Wiedervorlage in der Triage (docs/03 §10.3): Issues mit abgelaufenem
    /// `snoozedUntilAt` tauchen wieder auf (`snoozedUntilAt`/`snoozedBy` werden geleert).
    pub async fn run_triage_unsnooze(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let orgs = sys.triage().orgs_with_due_snooze(now).await?;
        drop(sys);
        let mut stats = JobStats::default();
        for org in orgs {
            match self.unsnooze_org(org, now).await {
                Ok(n) => stats.processed += n,
                Err(e) => stats.fail(&format!("triage_unsnooze org {org}"), e),
            }
        }
        Ok(stats)
    }

    async fn unsnooze_org(&self, org: OrgId, now: DateTime<Utc>) -> AppResult<u64> {
        let mut total = 0;
        for _ in 0..MAX_BATCHES {
            let mut tx = self.store.begin(org).await?;
            let ids = tx.triage().due_snoozed(now, BATCH).await?;
            if ids.is_empty() {
                return Ok(total);
            }
            let full = ids.len() as i64 == BATCH;
            for id in ids {
                let Some(mut issue) = tx.issues().get(id, true).await? else {
                    continue;
                };
                let Some(until) = issue.snoozed_until_at else {
                    continue;
                };
                if issue.state_type != StateType::Triage || until > now {
                    continue;
                }
                let mut delta = Delta::default();
                delta.set(
                    "snoozedUntilAt",
                    Value::String(until.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
                    Value::Null,
                );
                issue.snoozed_until_at = None;
                issue.snoozed_by_id = None;
                let saved = tx.issues().save(&issue, now).await?;
                history::record(
                    &mut *tx,
                    Uuid::now_v7(),
                    &saved,
                    None,
                    HistorySource::System,
                    &delta,
                    now,
                )
                .await?;
                emit_system(
                    &mut *tx,
                    DomainEvent::IssueUpdated {
                        previous: None,
                        issue_id: saved.id,
                        team_id: saved.team_id,
                        changed: vec!["snoozedUntilAt".into()],
                        mentioned_user_ids: vec![],
                    },
                )
                .await?;
                total += 1;
            }
            tx.commit().await?;
            if !full {
                return Ok(total);
            }
        }
        Ok(total)
    }

    /// SLA-Lauf (docs/03 §11.4): berechnet fehlende Risikoschwellen und meldet „gesetzt“,
    /// „hohes Risiko“ und „Breach“ je SLA genau einmal (`IssueSlaSet/HighRisk/Breached`).
    pub async fn run_sla(&self, now: DateTime<Utc>) -> AppResult<JobStats> {
        let mut sys = self.store.begin_system().await?;
        let orgs = sys.triage().orgs_with_sla().await?;
        drop(sys);
        let mut stats = JobStats::default();
        for org in orgs {
            match self.sla_org(org, now).await {
                Ok(n) => stats.processed += n,
                Err(e) => stats.fail(&format!("sla org {org}"), e),
            }
        }
        Ok(stats)
    }

    async fn sla_org(&self, org: OrgId, now: DateTime<Utc>) -> AppResult<u64> {
        let ctx = system_ctx(org, "sla");
        let mut total = 0;
        let mut rules_done = false;
        for _ in 0..MAX_BATCHES {
            let mut tx = self.store.begin(org).await?;
            let snap = PolicySnapshot::load(&mut *tx, &ctx).await?;
            if !rules_done {
                total += self.sla_rule_sweep(&mut *tx, &snap, now).await?;
                rules_done = true;
            }
            let ids = tx.triage().sla_due(now, BATCH).await?;
            if ids.is_empty() {
                tx.commit().await?;
                return Ok(total);
            }
            let full = ids.len() as i64 == BATCH;
            for id in ids {
                let Some(issue) = tx.issues().get(id, true).await? else {
                    continue;
                };
                let Some(team) = snap.tree.get(issue.team_id) else {
                    continue;
                };
                total += crate::usecases::triage::sla::process_issue(&mut *tx, team, &issue, now)
                    .await?;
            }
            tx.commit().await?;
            if !full {
                return Ok(total);
            }
        }
        Ok(total)
    }

    /// Regelauswertung für kürzlich geänderte Issues in Teams mit SLA (fängt Create/Updates ab,
    /// die nicht über `change_state` laufen). Idempotent: ein SLA derselben Regel bleibt bestehen.
    async fn sla_rule_sweep(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let since = now
            .checked_sub_signed(chrono::Duration::minutes(SLA_SWEEP_WINDOW_MINUTES))
            .unwrap_or(now);
        let mut total = 0;
        let mut after: Option<IssueId> = None;
        for _ in 0..MAX_BATCHES {
            let ids = tx.triage().sla_rule_candidates(since, after, BATCH).await?;
            let Some(last) = ids.last().copied() else {
                break;
            };
            after = Some(last);
            let full = ids.len() as i64 == BATCH;
            for id in ids {
                let Some(mut issue) = tx.issues().get(id, true).await? else {
                    continue;
                };
                let Some(team) = snap.tree.get(issue.team_id) else {
                    continue;
                };
                let old = issue.clone();
                if crate::usecases::triage::sla::evaluate_rules(tx, team, &mut issue, now).await? {
                    crate::usecases::triage::sla::persist_rule_change(tx, &old, &issue, now)
                        .await?;
                    total += 1;
                }
            }
            if !full {
                break;
            }
        }
        Ok(total)
    }
}
