//! Sichten auf Projekte und Meilensteine: Projekt-Hülle (nur sichtbare Teams), Fortschritt, Scope,
//! Meilenstein-Status (docs/02 §5.3, docs/04 §2.9, §3.1).

use super::pure::{MilestoneStatus, MilestoneStatusInput, milestone_statuses};
use crate::error::AppResult;
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::cycle::IssueScopeRow;
use crate::ports::repos::initiative::Initiative;
use crate::ports::repos::project::{Milestone, Project, ProjectLabelRow, ProjectStatus};
use crate::ports::repos::team::Team;
use crate::ports::repos::user::User;
use crate::resolve;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use lineal_domain::enums::{StateType, UserRole};
use lineal_domain::ids::{MilestoneId, ProjectId, TeamId, UserId};
use lineal_domain::rules::progress::{Stats, stats};
use std::collections::HashMap;

/// Scope und Fortschritt (immer über **alle** Issues, docs/02 §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProjectScope {
    /// 0..1.
    pub progress: f64,
    pub scope: f64,
    pub completed_scope: f64,
    pub in_progress_scope: f64,
    /// Relevante Issues (ohne `canceled`/`duplicate`).
    pub issue_count: i64,
    pub completed_issue_count: i64,
}

impl ProjectScope {
    /// Berechnet Scope und Fortschritt aus Issue-Zeilen.
    pub fn from_rows(rows: &[IssueScopeRow]) -> Self {
        let inputs: Vec<_> = rows.iter().map(IssueScopeRow::progress_input).collect();
        let s: Stats = stats(&inputs);
        let relevant = rows
            .iter()
            .filter(|r| !matches!(r.state_type, StateType::Canceled | StateType::Duplicate));
        Self {
            progress: s.progress,
            scope: s.scope,
            completed_scope: s.completed,
            in_progress_scope: s.started,
            issue_count: relevant.clone().count() as i64,
            completed_issue_count: relevant
                .filter(|r| r.state_type == StateType::Completed)
                .count() as i64,
        }
    }
}

/// Ein Meilenstein mit berechneten Feldern.
#[derive(Debug, Clone, PartialEq)]
pub struct MilestoneView {
    pub milestone: Milestone,
    pub status: MilestoneStatus,
    /// 0..1.
    pub progress: f64,
    pub issue_count: i64,
    pub completed_issue_count: i64,
}

/// Ein Projekt, wie ein bestimmter Betrachter es sieht (docs/02 §5.3 `projectView`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectView {
    pub project: Project,
    /// `P-<KEY>-<n>` mit dem Key des Lead-Teams (der Key gilt nicht als geheim).
    pub identifier: String,
    pub previous_identifiers: Vec<String>,
    pub status: ProjectStatus,
    /// `None`, wenn das Lead-Team für den Betrachter nicht zugänglich ist.
    pub lead_team: Option<Team>,
    /// Nur zugängliche Teams.
    pub teams: Vec<Team>,
    pub lead: Option<User>,
    pub creator: Option<User>,
    pub members: Vec<User>,
    pub labels: Vec<ProjectLabelRow>,
    /// Für Gäste leer; nur sichtbare Initiativen.
    pub initiatives: Vec<Initiative>,
    pub scope: ProjectScope,
    pub milestones: Vec<MilestoneView>,
    /// Anzahl offener Issues; gesetzt, wenn der Status in `completed`/`canceled` wechselt (Warnung).
    pub open_issue_count: Option<i64>,
}

/// Welche optionalen Teile geladen werden.
#[derive(Debug, Clone, Copy, Default)]
pub struct ViewOpts {
    pub members: bool,
    pub milestones: bool,
}

pub(crate) async fn load_user(
    tx: &mut dyn Tx,
    cache: &mut HashMap<UserId, Option<User>>,
    id: UserId,
) -> AppResult<Option<User>> {
    if let Some(u) = cache.get(&id) {
        return Ok(u.clone());
    }
    let u = tx.users().get(id).await?;
    cache.insert(id, u.clone());
    Ok(u)
}

/// Meilensteine mit Status und Fortschritt (Zeitzone des Lead-Teams für „heute“).
pub async fn milestone_views(
    tx: &mut dyn Tx,
    project: ProjectId,
    lead_tz: Tz,
    now: DateTime<Utc>,
) -> AppResult<Vec<MilestoneView>> {
    let milestones = tx.projects().list_milestones(project).await?;
    let rows = tx.projects().milestone_issue_rows(project).await?;
    let today = now.with_timezone(&lead_tz).date_naive();
    let mut by_ms: HashMap<MilestoneId, Vec<IssueScopeRow>> = HashMap::new();
    for (m, r) in rows {
        by_ms.entry(m).or_default().push(r);
    }
    let scopes: Vec<ProjectScope> = milestones
        .iter()
        .map(|m| ProjectScope::from_rows(by_ms.get(&m.id).map(Vec::as_slice).unwrap_or(&[])))
        .collect();
    let inputs: Vec<MilestoneStatusInput> = milestones
        .iter()
        .zip(&scopes)
        .map(|(m, s)| MilestoneStatusInput {
            target_date: m.target_date,
            has_relevant_issues: s.issue_count > 0,
            all_completed: s.issue_count > 0 && s.completed_issue_count == s.issue_count,
        })
        .collect();
    let statuses = milestone_statuses(&inputs, today);
    Ok(milestones
        .into_iter()
        .zip(scopes)
        .zip(statuses)
        .map(|((milestone, s), status)| MilestoneView {
            milestone,
            status,
            progress: s.progress,
            issue_count: s.issue_count,
            completed_issue_count: s.completed_issue_count,
        })
        .collect())
}

/// Zeitzone des Lead-Teams (Fallback UTC).
pub(crate) fn team_tz(snap: &PolicySnapshot, team: TeamId) -> Tz {
    snap.tree
        .get(team)
        .and_then(|t| t.timezone.parse::<Tz>().ok())
        .unwrap_or(chrono_tz::UTC)
}

/// Baut Sichten für mehrere Projekte (Teams/Status/Scope in wenigen Abfragen).
pub async fn build_views(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    projects: Vec<Project>,
    opts: ViewOpts,
    now: DateTime<Utc>,
) -> AppResult<Vec<ProjectView>> {
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<ProjectId> = projects.iter().map(|p| p.id).collect();
    let statuses = tx.projects().list_statuses().await?;
    let team_rows = tx.projects().teams_of_projects(&ids).await?;
    let issue_rows = tx.projects().issue_rows_of_projects(&ids).await?;
    let init_rows = if snap.actor.role == UserRole::Guest {
        Vec::new()
    } else {
        tx.initiatives().initiatives_of_projects(&ids).await?
    };
    let visible = snap.visible_team_ids();
    let mut users: HashMap<UserId, Option<User>> = HashMap::new();
    let mut initiatives: HashMap<_, Option<Initiative>> = HashMap::new();
    let mut out = Vec::with_capacity(projects.len());
    for p in projects {
        let status = statuses
            .iter()
            .find(|s| s.id == p.status_id)
            .cloned()
            .ok_or_else(|| crate::error::AppError::internal("project status missing"))?;
        let team_ids: Vec<TeamId> = team_rows
            .iter()
            .filter(|(pid, _)| *pid == p.id)
            .map(|(_, t)| *t)
            .collect();
        let teams: Vec<Team> = team_ids
            .iter()
            .filter(|t| visible.contains(t))
            .filter_map(|t| snap.tree.get(*t).cloned())
            .collect();
        let lead_team = visible
            .contains(&p.lead_team_id)
            .then(|| snap.tree.get(p.lead_team_id).cloned())
            .flatten();
        let key = snap
            .tree
            .get(p.lead_team_id)
            .map(|t| t.key.clone())
            .unwrap_or_default();
        let rows: Vec<IssueScopeRow> = issue_rows
            .iter()
            .filter(|(pid, _)| *pid == p.id)
            .map(|(_, r)| *r)
            .collect();
        let lead = match p.lead_id {
            Some(id) => load_user(tx, &mut users, id).await?,
            None => None,
        };
        let creator = match p.creator_id {
            Some(id) => load_user(tx, &mut users, id).await?,
            None => None,
        };
        let mut members = Vec::new();
        if opts.members {
            for id in tx.projects().list_members(p.id).await? {
                if let Some(u) = load_user(tx, &mut users, id).await? {
                    members.push(u);
                }
            }
        }
        let labels = tx.projects().labels_of(p.id).await?;
        let mut inits = Vec::new();
        for (pid, iid) in init_rows.iter().filter(|(pid, _)| *pid == p.id) {
            let _ = pid;
            if !initiatives.contains_key(iid) {
                let i = tx.initiatives().get(*iid).await?;
                initiatives.insert(*iid, i);
            }
            if let Some(Some(i)) = initiatives.get(iid)
                && !i.trashed
                && resolve::initiative::is_visible(snap, i)
            {
                inits.push(i.clone());
            }
        }
        let milestones = if opts.milestones {
            milestone_views(tx, p.id, team_tz(snap, p.lead_team_id), now).await?
        } else {
            Vec::new()
        };
        let previous_identifiers = tx.projects().previous_identifiers(p.id).await?;
        out.push(ProjectView {
            identifier: format!("P-{}-{}", key, p.number),
            previous_identifiers,
            status,
            lead_team,
            teams,
            lead,
            creator,
            members,
            labels,
            initiatives: inits,
            scope: ProjectScope::from_rows(&rows),
            milestones,
            open_issue_count: None,
            project: p,
        });
    }
    Ok(out)
}
