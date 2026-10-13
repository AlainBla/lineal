//! usecases – firmenweite Sicht (docs/14): `workspace_overview`, `workload`, `active_cycles`.
//!
//! Sicherheitsregeln:
//! * Gäste → `forbidden` (`Action::ViewWorkspaceOverview`).
//! * Jede Abfrage rechnet nur über `visible_team_ids`; private Teams ohne Mitgliedschaft
//!   tauchen weder als Zeile noch in Summen, Rollups, Projekt-/Initiativzählern oder der
//!   Risikoliste auf. Ausdrücklich genannte Teams werden wie überall aufgelöst (unsichtbar →
//!   `not_found`).
//! * Rollups summieren nur sichtbare Sub-Teams.

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::AppResult;
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::overview::{OverviewQuery, TeamIssueStats, WorkloadCycle};
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::actor_user_id;
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use lineal_domain::ids::{ProjectId, TeamId, UserId};
use lineal_domain::enums::Health;
use lineal_domain::rules::progress::stats;
use super::project::pure::{Staleness, staleness};
use super::project::update::reminder_cycle_days;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Berichtszeitraum für „erledigt“.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverviewPeriod {
    #[default]
    Days7,
    Days30,
}

impl OverviewPeriod {
    pub const fn days(self) -> i64 {
        match self {
            Self::Days7 => 7,
            Self::Days30 => 30,
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Days7 => "7d",
            Self::Days30 => "30d",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "7d" => Some(Self::Days7),
            "30d" => Some(Self::Days30),
            _ => None,
        }
    }
}

/// Eingabe von `workspace_overview` (docs/14 §4).
#[derive(Debug, Clone, PartialEq)]
pub struct OverviewInput {
    /// Teams (Name, Key oder ID); leer = alle sichtbaren.
    pub teams: Vec<String>,
    pub include_sub_teams: bool,
    pub period: OverviewPeriod,
}

impl Default for OverviewInput {
    fn default() -> Self {
        Self {
            teams: Vec::new(),
            include_sub_teams: true,
            period: OverviewPeriod::Days7,
        }
    }
}

/// Kennzahlen aus docs/14 §3 (je Team, gerollt oder als Summe).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Counts {
    pub open_issues: i64,
    pub in_progress: i64,
    pub triage: i64,
    pub triage_oldest_at: Option<DateTime<Utc>>,
    pub completed: i64,
    pub completed_points: f64,
    pub overdue: i64,
    pub sla_at_risk: i64,
    pub sla_breached: i64,
}

impl Counts {
    fn add(&mut self, s: &TeamIssueStats) {
        self.open_issues += s.open;
        self.in_progress += s.started;
        self.triage += s.triage;
        self.triage_oldest_at = match (self.triage_oldest_at, s.triage_oldest_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.completed += s.completed;
        self.completed_points += s.completed_points;
        self.overdue += s.overdue;
        self.sla_at_risk += s.sla_at_risk;
        self.sla_breached += s.sla_breached;
    }
}

/// Projekte nach Health.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectHealth {
    pub on_track: i64,
    pub at_risk: i64,
    pub off_track: i64,
    pub no_update: i64,
}

impl ProjectHealth {
    pub fn total(&self) -> i64 {
        self.on_track + self.at_risk + self.off_track + self.no_update
    }
}

/// Durchsatz einer Woche (0 = jüngste 7 Tage).
#[derive(Debug, Clone, PartialEq)]
pub struct WeekPoint {
    pub week_offset: i32,
    pub issues: i64,
    pub points: f64,
}

/// Kachel eines Teams.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamMetrics {
    pub team_id: TeamId,
    pub key: String,
    pub name: String,
    pub parent_id: Option<TeamId>,
    /// Einrückung in der Baumdarstellung (nur sichtbare Eltern zählen).
    pub depth: u32,
    pub counts: Counts,
    /// Letzte 6 Wochen, älteste zuletzt (Index = `week_offset`).
    pub throughput: Vec<WeekPoint>,
    /// Fortschritt (0..1) des aktiven Cycles des Teams.
    pub cycle_progress: Option<f64>,
    pub projects: ProjectHealth,
}

/// Initiative mit Rollup der sichtbaren Projekte.
#[derive(Debug, Clone, PartialEq)]
pub struct InitiativeSummary {
    pub id: String,
    pub identifier: String,
    pub name: String,
    pub status: String,
    pub health: Option<String>,
    pub project_count: i64,
    pub completed_project_count: i64,
}

/// Überfälliges oder SLA-gefährdetes Issue.
#[derive(Debug, Clone, PartialEq)]
pub struct AtRiskIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub team_key: String,
    pub due_date: Option<NaiveDate>,
    pub sla_breaches_at: Option<DateTime<Utc>>,
    pub reasons: Vec<String>,
}

/// Ergebnis von `workspace_overview`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceOverview {
    pub period: OverviewPeriod,
    pub include_sub_teams: bool,
    pub teams: Vec<TeamMetrics>,
    /// Summe über alle berücksichtigten Teams (jedes Issue genau einmal).
    pub totals: Counts,
    pub projects: ProjectHealth,
    pub initiatives: Vec<InitiativeSummary>,
    pub at_risk: Vec<AtRiskIssue>,
}

fn counts_json(c: &Counts) -> Value {
    json!({
        "openIssues": c.open_issues,
        "inProgress": c.in_progress,
        "triage": c.triage,
        "triageOldestAt": c.triage_oldest_at.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        "completed": c.completed,
        "completedPoints": c.completed_points,
        "overdue": c.overdue,
        "slaAtRisk": c.sla_at_risk,
        "slaBreached": c.sla_breached,
    })
}

fn health_json(h: &ProjectHealth) -> Value {
    json!({
        "onTrack": h.on_track,
        "atRisk": h.at_risk,
        "offTrack": h.off_track,
        "noUpdate": h.no_update,
        "total": h.total(),
    })
}

impl WorkspaceOverview {
    /// JSON-Form für MCP `get_workspace_overview` (dieselben Zahlen wie GraphQL).
    pub fn to_json(&self) -> Value {
        json!({
            "period": self.period.as_str(),
            "includeSubTeams": self.include_sub_teams,
            "teams": self.teams.iter().map(|t| json!({
                "id": t.team_id.to_string(),
                "key": t.key,
                "name": t.name,
                "parentId": t.parent_id.map(|p| p.to_string()),
                "depth": t.depth,
                "counts": counts_json(&t.counts),
                "throughput": t.throughput.iter().map(|w| json!({
                    "weekOffset": w.week_offset, "issues": w.issues, "points": w.points,
                })).collect::<Vec<_>>(),
                "cycleProgress": t.cycle_progress,
                "projects": health_json(&t.projects),
            })).collect::<Vec<_>>(),
            "totals": counts_json(&self.totals),
            "projects": health_json(&self.projects),
            "initiatives": self.initiatives.iter().map(|i| json!({
                "id": i.id, "identifier": i.identifier, "name": i.name, "status": i.status,
                "health": i.health, "projectCount": i.project_count,
                "completedProjectCount": i.completed_project_count,
            })).collect::<Vec<_>>(),
            "atRisk": self.at_risk.iter().map(|i| json!({
                "id": i.id, "identifier": i.identifier, "title": i.title, "team": i.team_key,
                "dueDate": i.due_date.map(|d| d.to_string()),
                "slaBreachesAt": i.sla_breaches_at.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
                "reasons": i.reasons,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Auslastung eines Users in einem Team.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadTeam {
    pub team_id: TeamId,
    pub team_key: String,
    pub team_name: String,
    pub open_issues: i64,
    pub points: f64,
}

/// Zeile der Auslastungsmatrix.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadEntry {
    pub user_id: UserId,
    pub name: String,
    pub display_name: String,
    pub per_team: Vec<WorkloadTeam>,
    pub total_open_issues: i64,
    pub total_points: f64,
}

/// Aktiver Cycle eines Teams für die Cycle-Übersicht.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCycleInfo {
    pub team_id: TeamId,
    pub team_key: String,
    pub team_name: String,
    pub cycle_id: String,
    pub number: i32,
    pub name: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub remaining_days: i64,
    /// 0..1.
    pub progress: f64,
    pub scope: f64,
    pub completed_scope: f64,
    pub in_progress_scope: f64,
    /// Nach dem Start hinzugefügte Issues.
    pub scope_change: i64,
}

/// Sichtbare, nicht gelöschte Teams in Baumreihenfolge (Eltern vor Kindern, nach Name).
fn tree_order(snap: &PolicySnapshot, visible: &BTreeSet<TeamId>) -> Vec<(TeamId, u32)> {
    let mut out = Vec::new();
    fn walk(
        snap: &PolicySnapshot,
        visible: &BTreeSet<TeamId>,
        parent: Option<TeamId>,
        depth: u32,
        out: &mut Vec<(TeamId, u32)>,
    ) {
        let mut kids: Vec<_> = snap
            .tree
            .all()
            .filter(|t| visible.contains(&t.id) && !t.trashed && t.parent_id == parent)
            .collect();
        kids.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        for k in kids {
            out.push((k.id, depth));
            walk(snap, visible, Some(k.id), depth + 1, out);
        }
    }
    // Wurzeln: ohne Eltern oder mit unsichtbarem Elternteil.
    let mut roots: Vec<_> = snap
        .tree
        .all()
        .filter(|t| {
            visible.contains(&t.id)
                && !t.trashed
                && t.parent_id.is_none_or(|p| !visible.contains(&p))
        })
        .collect();
    roots.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    for r in roots {
        out.push((r.id, 0));
        walk(snap, visible, Some(r.id), 1, &mut out);
    }
    out
}

/// `{T} ∪ sichtbare Nachfahren` (nur mit `include_sub`).
fn base_set(
    snap: &PolicySnapshot,
    visible: &BTreeSet<TeamId>,
    team: TeamId,
    include_sub: bool,
) -> BTreeSet<TeamId> {
    let mut s = BTreeSet::from([team]);
    if include_sub {
        s.extend(
            snap.tree
                .descendants(team)
                .iter()
                .map(|t| t.id)
                .filter(|id| visible.contains(id)),
        );
    }
    s
}

async fn resolve_selection(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    refs: &[String],
    visible: &BTreeSet<TeamId>,
) -> AppResult<Vec<(TeamId, u32)>> {
    if refs.is_empty() {
        return Ok(tree_order(snap, visible));
    }
    let order = tree_order(snap, visible);
    let mut chosen = BTreeSet::new();
    for r in refs {
        chosen.insert(
            resolve::team::resolve_in(tx, snap, r, Visibility::Content)
                .await?
                .id,
        );
    }
    Ok(order
        .into_iter()
        .filter(|(id, _)| chosen.contains(id))
        .collect())
}

async fn today_for(tx: &mut dyn Tx, ctx: &RequestCtx, now: DateTime<Utc>) -> AppResult<NaiveDate> {
    let tz: Tz = match actor_user_id(ctx) {
        Some(u) => tx
            .users()
            .get(u)
            .await?
            .and_then(|u| u.timezone)
            .and_then(|z| z.parse().ok())
            .unwrap_or(chrono_tz::UTC),
        None => chrono_tz::UTC,
    };
    Ok(now.with_timezone(&tz).date_naive())
}

impl App {
    /// Dashboard der Firma (docs/14 §3): Kennzahlen je Team, Rollups, Projekte, Initiativen,
    /// Risiko-Liste.
    pub async fn workspace_overview(
        &self,
        ctx: &RequestCtx,
        input: OverviewInput,
    ) -> AppResult<WorkspaceOverview> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ViewWorkspaceOverview)?;
        let visible = snap.visible_team_ids();
        let now = self.clock.now();
        let rows = resolve_selection(&mut *tx, &snap, &input.teams, &visible).await?;

        let bases: BTreeMap<TeamId, BTreeSet<TeamId>> = rows
            .iter()
            .map(|(t, _)| (*t, base_set(&snap, &visible, *t, input.include_sub_teams)))
            .collect();
        let all: BTreeSet<TeamId> = bases.values().flatten().copied().collect();
        let q = OverviewQuery {
            teams: all.iter().copied().collect(),
            now,
            today: today_for(&mut *tx, ctx, now).await?,
            period_days: input.period.days(),
        };
        let (stats_rows, weeks, health_rows) = if all.is_empty() {
            (Vec::new(), Vec::new(), Vec::new())
        } else {
            (
                tx.overview().issue_stats(&q).await?,
                tx.overview().throughput(&q).await?,
                tx.overview().project_health(&q).await?,
            )
        };
        let by_team: HashMap<TeamId, &TeamIssueStats> = stats_rows
            .iter()
            .filter_map(|s| s.team_id.map(|t| (t, s)))
            .collect();

        // Veraltete Health-Werte zählen als „no update“ (docs/04 §5.5 „Update missing“/grau).
        let org_weeks = if health_rows.is_empty() {
            None
        } else {
            tx.projects()
                .reminder_settings()
                .await?
                .and_then(|s| s.project_frequency_in_weeks)
        };
        let mut project_health_of: HashMap<ProjectId, Option<String>> = HashMap::new();
        for r in &health_rows {
            let health = r.health.as_deref().and_then(|h| h.parse::<Health>().ok());
            let days = reminder_cycle_days(&r.reminder_mode, r.reminder_weeks, org_weeks);
            let st = staleness(
                !r.indefinite,
                days,
                health.zip(r.health_updated_at).map(|(h, at)| (at, h)),
                now,
            );
            let stale = matches!(st, Staleness::Missing | Staleness::Grey);
            project_health_of.insert(r.project_id, if stale { None } else { r.health.clone() });
        }
        let health_of = |teams: &BTreeSet<TeamId>| -> ProjectHealth {
            let ids: BTreeSet<ProjectId> = health_rows
                .iter()
                .filter(|r| teams.contains(&r.team_id))
                .map(|r| r.project_id)
                .collect();
            let mut h = ProjectHealth::default();
            for id in ids {
                match project_health_of.get(&id).and_then(|x| x.as_deref()) {
                    Some("onTrack") => h.on_track += 1,
                    Some("atRisk") => h.at_risk += 1,
                    Some("offTrack") => h.off_track += 1,
                    _ => h.no_update += 1,
                }
            }
            h
        };

        let mut teams = Vec::new();
        for (team_id, depth) in &rows {
            let Some(team) = snap.tree.get(*team_id) else {
                continue;
            };
            let base = &bases[team_id];
            let mut counts = Counts::default();
            for t in base {
                if let Some(s) = by_team.get(t) {
                    counts.add(s);
                }
            }
            let mut throughput: Vec<WeekPoint> = (0..6)
                .map(|w| WeekPoint {
                    week_offset: w,
                    issues: 0,
                    points: 0.0,
                })
                .collect();
            for w in weeks.iter().filter(|w| base.contains(&w.team_id)) {
                if let Some(p) = throughput.get_mut(w.week_offset.max(0) as usize) {
                    p.issues += w.issues;
                    p.points += w.points;
                }
            }
            // Cycle-Fortschritt: der aktive Cycle des Teams selbst.
            let mut cycle_progress = None;
            for c in tx.cycles().list_for_team(*team_id, false).await? {
                if c.is_active(now) {
                    let rows = tx.cycles().issue_rows(c.id).await?;
                    let inputs: Vec<_> = rows
                        .iter()
                        .filter(|r| visible.contains(&r.team_id))
                        .map(|r| r.progress_input())
                        .collect();
                    cycle_progress = Some(stats(&inputs).progress);
                    break;
                }
            }
            teams.push(TeamMetrics {
                team_id: *team_id,
                key: team.key.clone(),
                name: team.name.clone(),
                parent_id: team.parent_id.filter(|p| visible.contains(p)),
                depth: *depth,
                counts,
                throughput,
                cycle_progress,
                projects: health_of(base),
            });
        }

        let mut totals = Counts::default();
        for t in &all {
            if let Some(s) = by_team.get(t) {
                totals.add(s);
            }
        }
        let projects = health_of(&all);

        let initiatives = if snap.actor.is_system() || all.is_empty() {
            Vec::new()
        } else {
            let enabled = tx.orgs().get().await?.is_none_or(|o| o.initiatives_enabled);
            if enabled {
                tx.overview()
                    .initiatives(&visible.iter().copied().collect::<Vec<_>>(), 50)
                    .await?
                    .into_iter()
                    .map(|i| InitiativeSummary {
                        id: i.id.to_string(),
                        identifier: format!("I-{}", i.number),
                        name: i.name,
                        status: i.status,
                        health: i.health,
                        project_count: i.project_count,
                        completed_project_count: i.completed_project_count,
                    })
                    .collect()
            } else {
                Vec::new()
            }
        };
        let at_risk = if all.is_empty() {
            Vec::new()
        } else {
            tx.overview()
                .at_risk_issues(&q, 25)
                .await?
                .into_iter()
                .map(|i| AtRiskIssue {
                    id: i.id.to_string(),
                    identifier: i.identifier,
                    title: i.title,
                    team_key: i.team_key,
                    due_date: i.due_date,
                    sla_breaches_at: i.sla_breaches_at,
                    reasons: i.reasons,
                })
                .collect()
        };
        Ok(WorkspaceOverview {
            period: input.period,
            include_sub_teams: input.include_sub_teams,
            teams,
            totals,
            projects,
            initiatives,
            at_risk,
        })
    }

    /// Auslastung: Matrix Person × Team im aktuellen oder nächsten Cycle (docs/14 §2).
    pub async fn workload(
        &self,
        ctx: &RequestCtx,
        team_refs: Vec<String>,
        next: bool,
    ) -> AppResult<Vec<WorkloadEntry>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ViewWorkspaceOverview)?;
        let visible = snap.visible_team_ids();
        let now = self.clock.now();
        let rows = resolve_selection(&mut *tx, &snap, &team_refs, &visible).await?;
        let teams: Vec<TeamId> = rows.iter().map(|(t, _)| *t).collect();
        if teams.is_empty() {
            return Ok(Vec::new());
        }
        let q = OverviewQuery {
            teams,
            now,
            today: today_for(&mut *tx, ctx, now).await?,
            period_days: 7,
        };
        let data = tx
            .overview()
            .workload(
                &q,
                if next {
                    WorkloadCycle::Next
                } else {
                    WorkloadCycle::Current
                },
            )
            .await?;
        let mut by_user: BTreeMap<UserId, WorkloadEntry> = BTreeMap::new();
        for r in data {
            let Some(team) = snap.tree.get(r.team_id) else {
                continue;
            };
            let e = by_user.entry(r.user_id).or_insert_with(|| WorkloadEntry {
                user_id: r.user_id,
                name: r.user_name.clone(),
                display_name: r.user_display_name.clone(),
                per_team: Vec::new(),
                total_open_issues: 0,
                total_points: 0.0,
            });
            e.total_open_issues += r.open_issues;
            e.total_points += r.points;
            e.per_team.push(WorkloadTeam {
                team_id: r.team_id,
                team_key: team.key.clone(),
                team_name: team.name.clone(),
                open_issues: r.open_issues,
                points: r.points,
            });
        }
        let mut out: Vec<WorkloadEntry> = by_user.into_values().collect();
        for e in &mut out {
            e.per_team.sort_by(|a, b| a.team_key.cmp(&b.team_key));
        }
        out.sort_by(|a, b| {
            b.total_points
                .partial_cmp(&a.total_points)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.name.cmp(&b.name))
        });
        Ok(out)
    }

    /// Aktive Cycles aller (ausgewählten) sichtbaren Teams (docs/14 §2 „Cycle-Übersicht“).
    pub async fn active_cycles(
        &self,
        ctx: &RequestCtx,
        team_refs: Vec<String>,
    ) -> AppResult<Vec<ActiveCycleInfo>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ViewWorkspaceOverview)?;
        let visible = snap.visible_team_ids();
        let now = self.clock.now();
        let rows = resolve_selection(&mut *tx, &snap, &team_refs, &visible).await?;
        let teams: Vec<TeamId> = rows.iter().map(|(t, _)| *t).collect();
        if teams.is_empty() {
            return Ok(Vec::new());
        }
        let q = OverviewQuery {
            teams,
            now,
            today: now.date_naive(),
            period_days: 7,
        };
        let cycles = tx.overview().active_cycles(&q).await?;
        let order: HashMap<TeamId, usize> =
            rows.iter().enumerate().map(|(i, (t, _))| (*t, i)).collect();
        let mut out = Vec::new();
        for c in cycles {
            let Some(team) = snap.tree.get(c.team_id) else {
                continue;
            };
            let issue_rows = tx.cycles().issue_rows(c.cycle_id).await?;
            let inputs: Vec<_> = issue_rows
                .iter()
                .filter(|r| visible.contains(&r.team_id))
                .map(|r| r.progress_input())
                .collect();
            let s = stats(&inputs);
            let remaining = (c.ends_at - now).num_seconds().max(0);
            out.push(ActiveCycleInfo {
                team_id: c.team_id,
                team_key: team.key.clone(),
                team_name: team.name.clone(),
                cycle_id: c.cycle_id.to_string(),
                number: c.number,
                name: c.name,
                starts_at: c.starts_at,
                ends_at: c.ends_at,
                remaining_days: (remaining + 86_399) / 86_400,
                progress: s.progress,
                scope: s.scope,
                completed_scope: s.completed,
                in_progress_scope: s.started,
                scope_change: c.added_after_start,
            });
        }
        out.sort_by_key(|c| order.get(&c.team_id).copied().unwrap_or(usize::MAX));
        Ok(out)
    }
}
