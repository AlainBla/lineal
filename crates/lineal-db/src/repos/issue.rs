//! `IssueRepo` für `PgTx` (WP06): Issues, Labels-Zuordnung, Relationen, History, Aliase, Sharing.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_app::ports::repos::IssueRepo;
use lineal_app::ports::repos::issue::{
    CycleLookup, HistoryEntry, HistorySource, Issue, IssueExtras, IssueListFilter, IssueShare,
    MilestoneLookup, ProjectLookup, Relation,
};
use lineal_app::ports::repos::user::{OrderBy, Page, PageCursor, PageRequest};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::{Priority, RelationType, SlaType, StateType};
use lineal_domain::ids::{
    CycleId, IssueId, IssueRelationId, LabelId, MilestoneId, OrgId, ProjectId, StateId, TeamId,
    UserId,
};
use sqlx::{Postgres, QueryBuilder};
use std::collections::HashMap;
use uuid::Uuid;

const COLS: &str = "id, organization_id, team_id, number, identifier, title, description, state_id, \
    state_type, state_entered_at, priority, estimate, assignee_id, delegate_id, creator_id, parent_id, \
    project_id, project_milestone_id, cycle_id, due_date, sort_order, priority_sort_order, \
    sub_issue_sort_order, started_triage_at, triaged_at, started_at, completed_at, canceled_at, \
    auto_closed_at, snoozed_until_at, snoozed_by_id, added_to_team_at, added_to_cycle_at, \
    added_to_project_at, sla_type, sla_started_at, sla_medium_risk_at, sla_high_risk_at, \
    sla_breaches_at, trashed, trashed_at, archived_at, version, created_at, updated_at";

#[derive(sqlx::FromRow)]
pub(crate) struct IssueRow {
    id: Uuid,
    organization_id: Uuid,
    team_id: Uuid,
    number: i32,
    identifier: String,
    title: String,
    description: Option<String>,
    state_id: Uuid,
    state_type: String,
    state_entered_at: DateTime<Utc>,
    priority: i32,
    estimate: Option<i32>,
    assignee_id: Option<Uuid>,
    delegate_id: Option<Uuid>,
    creator_id: Option<Uuid>,
    parent_id: Option<Uuid>,
    project_id: Option<Uuid>,
    project_milestone_id: Option<Uuid>,
    cycle_id: Option<Uuid>,
    due_date: Option<NaiveDate>,
    sort_order: f64,
    priority_sort_order: f64,
    sub_issue_sort_order: Option<f64>,
    started_triage_at: Option<DateTime<Utc>>,
    triaged_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    canceled_at: Option<DateTime<Utc>>,
    auto_closed_at: Option<DateTime<Utc>>,
    snoozed_until_at: Option<DateTime<Utc>>,
    snoozed_by_id: Option<Uuid>,
    added_to_team_at: DateTime<Utc>,
    added_to_cycle_at: Option<DateTime<Utc>>,
    added_to_project_at: Option<DateTime<Utc>>,
    sla_type: Option<String>,
    sla_started_at: Option<DateTime<Utc>>,
    sla_medium_risk_at: Option<DateTime<Utc>>,
    sla_high_risk_at: Option<DateTime<Utc>>,
    sla_breaches_at: Option<DateTime<Utc>>,
    trashed: bool,
    trashed_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn bad<T>(what: &str) -> AppResult<T> {
    Err(AppError::internal(format!("invalid {what} in database")))
}

impl IssueRow {
    fn into_issue(self) -> AppResult<Issue> {
        let Ok(state_type) = self.state_type.parse::<StateType>() else {
            return bad("state type");
        };
        let Ok(priority) = Priority::try_from(self.priority as u8) else {
            return bad("priority");
        };
        let sla_type = match self.sla_type.as_deref() {
            None => None,
            Some(s) => match s.parse::<SlaType>() {
                Ok(t) => Some(t),
                Err(_) => return bad("sla type"),
            },
        };
        Ok(Issue {
            id: IssueId(self.id),
            org_id: OrgId(self.organization_id),
            team_id: TeamId(self.team_id),
            number: self.number,
            identifier: self.identifier,
            title: self.title,
            description: self.description,
            state_id: StateId(self.state_id),
            state_type,
            state_entered_at: self.state_entered_at,
            priority,
            estimate: self.estimate,
            assignee_id: self.assignee_id.map(UserId),
            delegate_id: self.delegate_id.map(UserId),
            creator_id: self.creator_id.map(UserId),
            parent_id: self.parent_id.map(IssueId),
            project_id: self.project_id.map(ProjectId),
            project_milestone_id: self.project_milestone_id.map(MilestoneId),
            cycle_id: self.cycle_id.map(CycleId),
            due_date: self.due_date,
            sort_order: self.sort_order,
            priority_sort_order: self.priority_sort_order,
            sub_issue_sort_order: self.sub_issue_sort_order,
            started_triage_at: self.started_triage_at,
            triaged_at: self.triaged_at,
            started_at: self.started_at,
            completed_at: self.completed_at,
            canceled_at: self.canceled_at,
            auto_closed_at: self.auto_closed_at,
            snoozed_until_at: self.snoozed_until_at,
            snoozed_by_id: self.snoozed_by_id.map(UserId),
            added_to_team_at: self.added_to_team_at,
            added_to_cycle_at: self.added_to_cycle_at,
            added_to_project_at: self.added_to_project_at,
            sla_type,
            sla_started_at: self.sla_started_at,
            sla_medium_risk_at: self.sla_medium_risk_at,
            sla_high_risk_at: self.sla_high_risk_at,
            sla_breaches_at: self.sla_breaches_at,
            trashed: self.trashed,
            trashed_at: self.trashed_at,
            archived_at: self.archived_at,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct HistoryRow {
    id: Uuid,
    issue_id: Uuid,
    actor_id: Option<Uuid>,
    source: String,
    changes: serde_json::Value,
    added_label_ids: Option<Vec<Uuid>>,
    removed_label_ids: Option<Vec<Uuid>>,
    relation_changes: Option<serde_json::Value>,
    flags: Vec<String>,
    created_at: DateTime<Utc>,
}

const HISTORY_COLS: &str = "id, issue_id, actor_id, source, changes, added_label_ids, \
    removed_label_ids, relation_changes, flags, created_at";

impl From<HistoryRow> for HistoryEntry {
    fn from(r: HistoryRow) -> Self {
        let labels = |v: Option<Vec<Uuid>>| v.map(|v| v.into_iter().map(LabelId).collect());
        HistoryEntry {
            id: r.id,
            issue_id: IssueId(r.issue_id),
            actor_id: r.actor_id.map(UserId),
            source: HistorySource::parse(&r.source),
            changes: r.changes,
            added_label_ids: labels(r.added_label_ids),
            removed_label_ids: labels(r.removed_label_ids),
            relation_changes: r.relation_changes,
            flags: r.flags,
            created_at: r.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct RelationRow {
    id: Uuid,
    issue_id: Uuid,
    related_issue_id: Uuid,
    #[sqlx(rename = "type")]
    kind: String,
    creator_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

const RELATION_COLS: &str = "id, issue_id, related_issue_id, type, creator_id, created_at";

impl RelationRow {
    fn into_relation(self) -> AppResult<Relation> {
        let Ok(kind) = self.kind.parse::<RelationType>() else {
            return bad("relation type");
        };
        Ok(Relation {
            id: IssueRelationId(self.id),
            issue_id: IssueId(self.issue_id),
            related_issue_id: IssueId(self.related_issue_id),
            relation_type: kind,
            creator_id: self.creator_id.map(UserId),
            created_at: self.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ExtrasRow {
    id: Uuid,
    state_name: String,
    team_key: String,
    team_name: String,
    assignee_name: Option<String>,
    delegate_name: Option<String>,
    creator_name: Option<String>,
    project_name: Option<String>,
    project_identifier: Option<String>,
    milestone_name: Option<String>,
    parent_identifier: Option<String>,
    label_names: Vec<String>,
    label_ids: Vec<Uuid>,
    children_count: i64,
}

#[derive(sqlx::FromRow)]
struct CycleRow {
    id: Uuid,
    team_id: Uuid,
    number: i32,
    name: Option<String>,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
    inherited_from_id: Option<Uuid>,
}

impl From<CycleRow> for CycleLookup {
    fn from(r: CycleRow) -> Self {
        CycleLookup {
            id: CycleId(r.id),
            team_id: TeamId(r.team_id),
            number: r.number,
            name: r.name,
            starts_at: r.starts_at,
            ends_at: r.ends_at,
            completed_at: r.completed_at,
            inherited_from_id: r.inherited_from_id.map(CycleId),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: Uuid,
    name: String,
    identifier: String,
    team_ids: Vec<Uuid>,
    trashed: bool,
}

impl From<ProjectRow> for ProjectLookup {
    fn from(r: ProjectRow) -> Self {
        ProjectLookup {
            id: ProjectId(r.id),
            name: r.name,
            identifier: r.identifier,
            team_ids: r.team_ids.into_iter().map(TeamId).collect(),
            trashed: r.trashed,
        }
    }
}

const PROJECT_SELECT: &str = "SELECT p.id, p.name, 'P-' || lt.key || '-' || p.number AS identifier, \
    COALESCE((SELECT array_agg(pt.team_id) FROM project_team pt WHERE pt.project_id = p.id), '{}') AS team_ids, \
    p.trashed FROM project p JOIN team lt ON lt.id = p.lead_team_id AND lt.organization_id = p.organization_id";

fn ids<T: Copy + Into<Uuid>>(v: &[T]) -> Vec<Uuid> {
    v.iter().map(|i| (*i).into()).collect()
}

/// Bedingungen von [`IssueRepo::list`] (nach `WHERE i.organization_id = `): Mandant, Papierkorb,
/// Sichtbarkeit (UND, außerhalb des Benutzerfilters), flache Filter, Filterbaum und Suche.
fn push_list_filters<'a>(
    qb: &mut QueryBuilder<'a, Postgres>,
    org: Uuid,
    f: &IssueListFilter,
) -> AppResult<()> {
    qb.push_bind(org);
    qb.push(" AND NOT i.trashed");
    qb.push(" AND (i.team_id = ANY(");
    qb.push_bind(ids(&f.team_ids));
    qb.push(")");
    if let Some(u) = f.shared_with {
        qb.push(" OR i.id IN (SELECT s.issue_id FROM issue_share s WHERE s.organization_id = i.organization_id AND s.user_id = ");
        qb.push_bind(u.0);
        qb.push(")");
    }
    qb.push(")");
    if !f.include_archived {
        qb.push(" AND i.archived_at IS NULL");
    }
    match f.assignee {
        Some(None) => {
            qb.push(" AND i.assignee_id IS NULL");
        }
        Some(Some(u)) => {
            qb.push(" AND i.assignee_id = ").push_bind(u.0);
        }
        None => {}
    }
    if let Some(u) = f.creator {
        qb.push(" AND i.creator_id = ").push_bind(u.0);
    }
    if let Some(u) = f.delegate {
        qb.push(" AND i.delegate_id = ").push_bind(u.0);
    }
    if let Some(s) = &f.state_ids {
        qb.push(" AND i.state_id = ANY(")
            .push_bind(ids(s))
            .push(")");
    }
    if let Some(t) = &f.state_types {
        let v: Vec<String> = t.iter().map(|t| t.as_str().to_owned()).collect();
        qb.push(" AND i.state_type = ANY(").push_bind(v).push(")");
    }
    if let Some(l) = &f.label_ids {
        qb.push(
            " AND EXISTS (SELECT 1 FROM issue_to_label il WHERE il.issue_id = i.id AND il.label_id = ANY(",
        );
        qb.push_bind(ids(l));
        qb.push("))");
    }
    if let Some(p) = f.project_id {
        qb.push(" AND i.project_id = ").push_bind(p.0);
    }
    if let Some(c) = f.cycle_id {
        qb.push(" AND i.cycle_id = ").push_bind(c.0);
    }
    if let Some(p) = f.parent_id {
        qb.push(" AND i.parent_id = ").push_bind(p.0);
    }
    if let Some(p) = f.priority {
        qb.push(" AND i.priority = ").push_bind(p.value() as i32);
    }
    if let Some(t) = f.created_after {
        qb.push(" AND i.created_at > ").push_bind(t);
    }
    if let Some(t) = f.updated_after {
        qb.push(" AND i.updated_at > ").push_bind(t);
    }
    if let Some(t) = f.triaged_after {
        qb.push(" AND i.triaged_at > ").push_bind(t);
    }
    if let Some(ast) = &f.ast {
        let ev = f
            .eval
            .as_ref()
            .ok_or_else(|| AppError::internal("filter without evaluation context"))?;
        let cx = crate::filter_sql::Cx::new(ev);
        qb.push(" AND ");
        crate::filter_sql::push_issue_filter(qb, &cx, "i", ast)?;
        crate::filter_sql::push_triage_guard(qb, "i", ast);
    }
    if let Some(text) = &f.search {
        crate::search::push_issue_search(qb, "i", text);
    }
    Ok(())
}

#[async_trait]
impl IssueRepo for PgTx {
    async fn insert(&mut self, i: &Issue) -> AppResult<Issue> {
        let row = sqlx::query_as::<_, IssueRow>(&format!(
            "INSERT INTO issue (id, organization_id, team_id, number, identifier, title, description, state_id, \
             state_type, state_entered_at, priority, estimate, assignee_id, delegate_id, creator_id, parent_id, \
             project_id, project_milestone_id, cycle_id, due_date, sort_order, priority_sort_order, \
             sub_issue_sort_order, started_triage_at, triaged_at, started_at, completed_at, canceled_at, \
             auto_closed_at, snoozed_until_at, snoozed_by_id, added_to_team_at, added_to_cycle_at, \
             added_to_project_at, sla_type, sla_started_at, sla_medium_risk_at, sla_high_risk_at, \
             sla_breaches_at, trashed, trashed_at, archived_at, version, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,\
             $26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,$37,$38,$39,$40,$41,$42,1,$43,$44) \
             RETURNING {COLS}"
        ))
        .bind(i.id.0)
        .bind(self.org.0)
        .bind(i.team_id.0)
        .bind(i.number)
        .bind("") // Trigger setzt die Kennung
        .bind(&i.title)
        .bind(&i.description)
        .bind(i.state_id.0)
        .bind(i.state_type.as_str()) // Trigger überschreibt
        .bind(i.state_entered_at)
        .bind(i.priority.value() as i32)
        .bind(i.estimate)
        .bind(i.assignee_id.map(|u| u.0))
        .bind(i.delegate_id.map(|u| u.0))
        .bind(i.creator_id.map(|u| u.0))
        .bind(i.parent_id.map(|u| u.0))
        .bind(i.project_id.map(|u| u.0))
        .bind(i.project_milestone_id.map(|u| u.0))
        .bind(i.cycle_id.map(|u| u.0))
        .bind(i.due_date)
        .bind(i.sort_order)
        .bind(i.priority_sort_order)
        .bind(i.sub_issue_sort_order)
        .bind(i.started_triage_at)
        .bind(i.triaged_at)
        .bind(i.started_at)
        .bind(i.completed_at)
        .bind(i.canceled_at)
        .bind(i.auto_closed_at)
        .bind(i.snoozed_until_at)
        .bind(i.snoozed_by_id.map(|u| u.0))
        .bind(i.added_to_team_at)
        .bind(i.added_to_cycle_at)
        .bind(i.added_to_project_at)
        .bind(i.sla_type.map(|t| t.as_str()))
        .bind(i.sla_started_at)
        .bind(i.sla_medium_risk_at)
        .bind(i.sla_high_risk_at)
        .bind(i.sla_breaches_at)
        .bind(i.trashed)
        .bind(i.trashed_at)
        .bind(i.archived_at)
        .bind(i.created_at)
        .bind(i.updated_at)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(|e| {
            // Client-ID kollidiert (B8.25): eigene Meldung, für sichtbare und unsichtbare Issues gleich.
            let id_conflict = e
                .as_database_error()
                .and_then(|d| d.constraint())
                .is_some_and(|c| c == "issue_pkey" || c == "issue_organization_id_id_key");
            let msg = if id_conflict {
                "id already in use"
            } else {
                "issue number already in use"
            };
            super::org::conflict_or_internal(e, msg)
        })?;
        row.into_issue()
    }

    async fn get(&mut self, id: IssueId, lock: bool) -> AppResult<Option<Issue>> {
        let row = sqlx::query_as::<_, IssueRow>(&format!(
            "SELECT {COLS} FROM issue WHERE organization_id = $1 AND id = $2{}",
            if lock { " FOR UPDATE" } else { "" }
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(IssueRow::into_issue).transpose()
    }

    async fn save(&mut self, i: &Issue, now: DateTime<Utc>) -> AppResult<Issue> {
        let row = sqlx::query_as::<_, IssueRow>(&format!(
            "UPDATE issue SET team_id = $3, number = $4, title = $5, description = $6, state_id = $7, \
             state_entered_at = $8, priority = $9, estimate = $10, assignee_id = $11, delegate_id = $12, \
             parent_id = $13, project_id = $14, project_milestone_id = $15, cycle_id = $16, due_date = $17, \
             sort_order = $18, priority_sort_order = $19, sub_issue_sort_order = $20, \
             started_triage_at = $21, triaged_at = $22, started_at = $23, completed_at = $24, \
             canceled_at = $25, auto_closed_at = $26, snoozed_until_at = $27, snoozed_by_id = $28, \
             added_to_team_at = $29, added_to_cycle_at = $30, added_to_project_at = $31, sla_type = $32, \
             sla_started_at = $33, sla_medium_risk_at = $34, sla_high_risk_at = $35, sla_breaches_at = $36, \
             trashed = $37, trashed_at = $38, archived_at = $39, version = version + 1, updated_at = $40 \
             WHERE organization_id = $1 AND id = $2 RETURNING {COLS}"
        ))
        .bind(self.org.0)
        .bind(i.id.0)
        .bind(i.team_id.0)
        .bind(i.number)
        .bind(&i.title)
        .bind(&i.description)
        .bind(i.state_id.0)
        .bind(i.state_entered_at)
        .bind(i.priority.value() as i32)
        .bind(i.estimate)
        .bind(i.assignee_id.map(|u| u.0))
        .bind(i.delegate_id.map(|u| u.0))
        .bind(i.parent_id.map(|u| u.0))
        .bind(i.project_id.map(|u| u.0))
        .bind(i.project_milestone_id.map(|u| u.0))
        .bind(i.cycle_id.map(|u| u.0))
        .bind(i.due_date)
        .bind(i.sort_order)
        .bind(i.priority_sort_order)
        .bind(i.sub_issue_sort_order)
        .bind(i.started_triage_at)
        .bind(i.triaged_at)
        .bind(i.started_at)
        .bind(i.completed_at)
        .bind(i.canceled_at)
        .bind(i.auto_closed_at)
        .bind(i.snoozed_until_at)
        .bind(i.snoozed_by_id.map(|u| u.0))
        .bind(i.added_to_team_at)
        .bind(i.added_to_cycle_at)
        .bind(i.added_to_project_at)
        .bind(i.sla_type.map(|t| t.as_str()))
        .bind(i.sla_started_at)
        .bind(i.sla_medium_risk_at)
        .bind(i.sla_high_risk_at)
        .bind(i.sla_breaches_at)
        .bind(i.trashed)
        .bind(i.trashed_at)
        .bind(i.archived_at)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(|e| super::org::conflict_or_internal(e, "issue number already in use"))?;
        row.ok_or_else(|| AppError::not_found("Could not find referenced Issue."))?
            .into_issue()
    }

    async fn find_by_number(&mut self, team: TeamId, number: i32) -> AppResult<Option<IssueId>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM issue WHERE organization_id = $1 AND team_id = $2 AND number = $3",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(number)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(id.map(IssueId))
    }

    async fn find_by_alias(&mut self, identifier: &str) -> AppResult<Option<IssueId>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT issue_id FROM issue_identifier_alias WHERE organization_id = $1 AND identifier = $2 \
             AND entity_type = 'issue' AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(identifier.to_uppercase())
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(id.map(IssueId))
    }

    async fn list(&mut self, f: &IssueListFilter, page: &PageRequest) -> AppResult<Page<Issue>> {
        let order_col = match page.order_by {
            OrderBy::CreatedAt => "i.created_at",
            OrderBy::UpdatedAt => "i.updated_at",
        };
        let cols = COLS
            .split(", ")
            .map(|c| format!("i.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new("");
        let has_cte = f.eval.is_some();
        if let Some(ev) = &f.eval {
            crate::filter_sql::push_cte_prefix(&mut qb, ev);
        }
        if f.search.is_some() {
            // Suche: erst die besten 500 Treffer nach Sortierung bestimmen (docs/05 §5.4), dann
            // paginieren – so bleibt der Cursor stabil und die Obergrenze hart.
            qb.push(if has_cte {
                ", matched AS ("
            } else {
                "WITH matched AS ("
            });
            qb.push(format!(
                "SELECT i.id, {order_col} AS sort_key FROM issue i WHERE i.organization_id = "
            ));
            push_list_filters(&mut qb, self.org.0, f)?;
            qb.push(format!(
                " ORDER BY {order_col} DESC, i.id DESC LIMIT 500) SELECT {cols}, m.sort_key \
                 FROM matched m JOIN issue i ON i.id = m.id WHERE TRUE"
            ));
            if let Some(c) = &page.cursor {
                qb.push(" AND (m.sort_key, i.id) < (");
                qb.push_bind(c.key);
                qb.push(", ");
                qb.push_bind(c.id);
                qb.push(")");
            }
            qb.push(" ORDER BY m.sort_key DESC, i.id DESC LIMIT ");
            qb.push_bind(page.limit + 1);
        } else {
            if has_cte {
                qb.push(" ");
            }
            qb.push(format!(
                "SELECT {cols}, {order_col} AS sort_key FROM issue i WHERE i.organization_id = "
            ));
            push_list_filters(&mut qb, self.org.0, f)?;
            if let Some(c) = &page.cursor {
                qb.push(format!(" AND ({order_col}, i.id) < ("));
                qb.push_bind(c.key);
                qb.push(", ");
                qb.push_bind(c.id);
                qb.push(")");
            }
            qb.push(format!(" ORDER BY {order_col} DESC, i.id DESC LIMIT "));
            qb.push_bind(page.limit + 1);
        }

        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            issue: IssueRow,
            sort_key: DateTime<Utc>,
        }
        let mut rows = qb
            .build_query_as::<Row>()
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        let has_next = rows.len() as i64 > page.limit;
        if has_next {
            rows.truncate(page.limit as usize);
        }
        let next = if has_next {
            rows.last().map(|r| PageCursor {
                key: r.sort_key,
                id: r.issue.id,
            })
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|r| r.issue.into_issue())
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Page { items, next })
    }

    async fn extras(&mut self, issue_ids: &[IssueId]) -> AppResult<HashMap<IssueId, IssueExtras>> {
        if issue_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query_as::<_, ExtrasRow>(
            "SELECT i.id, s.name AS state_name, t.key AS team_key, t.name AS team_name, \
               a.name AS assignee_name, d.name AS delegate_name, c.name AS creator_name, \
               p.name AS project_name, \
               CASE WHEN p.id IS NULL THEN NULL ELSE 'P-' || plt.key || '-' || p.number END AS project_identifier, \
               m.name AS milestone_name, pi.identifier AS parent_identifier, \
               COALESCE((SELECT array_agg(l.name ORDER BY l.name, l.id) FROM issue_to_label il \
                 JOIN issue_label l ON l.id = il.label_id WHERE il.issue_id = i.id), '{}') AS label_names, \
               COALESCE((SELECT array_agg(l.id ORDER BY l.name, l.id) FROM issue_to_label il \
                 JOIN issue_label l ON l.id = il.label_id WHERE il.issue_id = i.id), '{}') AS label_ids, \
               (SELECT count(*) FROM issue ch WHERE ch.parent_id = i.id AND NOT ch.trashed) AS children_count \
             FROM issue i \
             JOIN workflow_state s ON s.id = i.state_id \
             JOIN team t ON t.id = i.team_id \
             LEFT JOIN \"user\" a ON a.id = i.assignee_id \
             LEFT JOIN \"user\" d ON d.id = i.delegate_id \
             LEFT JOIN \"user\" c ON c.id = i.creator_id \
             LEFT JOIN project p ON p.id = i.project_id \
             LEFT JOIN team plt ON plt.id = p.lead_team_id \
             LEFT JOIN project_milestone m ON m.id = i.project_milestone_id \
             LEFT JOIN issue pi ON pi.id = i.parent_id \
             WHERE i.organization_id = $1 AND i.id = ANY($2)",
        )
        .bind(self.org.0)
        .bind(ids(issue_ids))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    IssueId(r.id),
                    IssueExtras {
                        state_name: r.state_name,
                        team_key: r.team_key,
                        team_name: r.team_name,
                        assignee_name: r.assignee_name,
                        delegate_name: r.delegate_name,
                        creator_name: r.creator_name,
                        project_name: r.project_name,
                        project_identifier: r.project_identifier,
                        milestone_name: r.milestone_name,
                        parent_identifier: r.parent_identifier,
                        label_names: r.label_names,
                        label_ids: r.label_ids.into_iter().map(LabelId).collect(),
                        children_count: r.children_count,
                    },
                )
            })
            .collect())
    }

    async fn previous_identifiers(&mut self, issue: IssueId) -> AppResult<Vec<String>> {
        let mut out: Vec<String> = sqlx::query_scalar(
            "SELECT identifier FROM issue_identifier_alias WHERE organization_id = $1 AND issue_id = $2 \
             AND archived_at IS NULL ORDER BY created_at, identifier",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let keys: Vec<String> = sqlx::query_scalar(
            "SELECT tk.key || '-' || i.number FROM issue i JOIN team_key tk ON tk.team_id = i.team_id \
             AND tk.organization_id = i.organization_id WHERE i.organization_id = $1 AND i.id = $2 \
             ORDER BY tk.replaced_at, tk.key",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        for k in keys {
            if !out.contains(&k) {
                out.push(k);
            }
        }
        Ok(out)
    }

    async fn count_open(&mut self, team: TeamId) -> AppResult<i64> {
        sqlx::query_scalar(
            "SELECT count(*) FROM issue WHERE organization_id = $1 AND team_id = $2 \
             AND NOT trashed AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn sort_extent(
        &mut self,
        team: TeamId,
        state: StateId,
    ) -> AppResult<(Option<f64>, Option<f64>)> {
        sqlx::query_as::<_, (Option<f64>, Option<f64>)>(
            "SELECT min(sort_order), max(sort_order) FROM issue WHERE organization_id = $1 AND team_id = $2 \
             AND state_id = $3 AND NOT trashed AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(state.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn max_sub_issue_sort_order(&mut self, parent: IssueId) -> AppResult<Option<f64>> {
        sqlx::query_scalar(
            "SELECT max(sub_issue_sort_order) FROM issue WHERE organization_id = $1 AND parent_id = $2",
        )
        .bind(self.org.0)
        .bind(parent.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn ancestor_ids(&mut self, issue: IssueId) -> AppResult<Vec<IssueId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "WITH RECURSIVE anc(id, parent_id, depth) AS ( \
               SELECT id, parent_id, 0 FROM issue WHERE organization_id = $1 AND id = $2 \
               UNION ALL \
               SELECT i.id, i.parent_id, a.depth + 1 FROM issue i JOIN anc a ON i.id = a.parent_id \
               WHERE i.organization_id = $1 AND a.depth < 64) \
             SELECT id FROM anc WHERE id <> $2 ORDER BY depth",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(IssueId).collect())
    }

    async fn descendant_depth(&mut self, issue: IssueId) -> AppResult<i64> {
        sqlx::query_scalar(
            "WITH RECURSIVE d(id, depth) AS ( \
               SELECT id, 0 FROM issue WHERE organization_id = $1 AND id = $2 \
               UNION ALL \
               SELECT i.id, d.depth + 1 FROM issue i JOIN d ON i.parent_id = d.id \
               WHERE i.organization_id = $1 AND d.depth < 64) \
             SELECT COALESCE(max(depth), 0)::bigint FROM d",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn children(&mut self, issue: IssueId) -> AppResult<Vec<Issue>> {
        let rows = sqlx::query_as::<_, IssueRow>(&format!(
            "SELECT {COLS} FROM issue WHERE organization_id = $1 AND parent_id = $2 AND NOT trashed \
             ORDER BY sub_issue_sort_order NULLS LAST, created_at, id"
        ))
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(IssueRow::into_issue).collect()
    }

    async fn label_ids(&mut self, issue: IssueId) -> AppResult<Vec<LabelId>> {
        let rows: Vec<Uuid> = sqlx::query_scalar(
            "SELECT il.label_id FROM issue_to_label il JOIN issue_label l ON l.id = il.label_id \
             WHERE il.organization_id = $1 AND il.issue_id = $2 ORDER BY il.created_at, l.name, il.label_id",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(LabelId).collect())
    }

    async fn apply_labels(
        &mut self,
        issue: IssueId,
        remove: &[LabelId],
        add: &[(LabelId, Option<LabelId>)],
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        if !remove.is_empty() {
            sqlx::query(
                "DELETE FROM issue_to_label WHERE organization_id = $1 AND issue_id = $2 AND label_id = ANY($3)",
            )
            .bind(self.org.0)
            .bind(issue.0)
            .bind(ids(remove))
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        for (label, group) in add {
            sqlx::query(
                "INSERT INTO issue_to_label (organization_id, issue_id, label_id, single_select_group_id, created_at) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
            )
            .bind(self.org.0)
            .bind(issue.0)
            .bind(label.0)
            .bind(group.map(|g| g.0))
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        if !add.is_empty() {
            let added: Vec<Uuid> = add.iter().map(|(l, _)| l.0).collect();
            sqlx::query(
                "UPDATE issue_label SET last_applied_at = $3 WHERE organization_id = $1 AND id = ANY($2)",
            )
            .bind(self.org.0)
            .bind(added)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn insert_history(&mut self, e: &HistoryEntry) -> AppResult<()> {
        let labels = |v: &Option<Vec<LabelId>>| v.as_ref().map(|v| ids(v));
        sqlx::query(
            "INSERT INTO issue_history (id, organization_id, issue_id, actor_id, source, changes, \
             added_label_ids, removed_label_ids, relation_changes, flags, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(e.id)
        .bind(self.org.0)
        .bind(e.issue_id.0)
        .bind(e.actor_id.map(|a| a.0))
        .bind(e.source.as_str())
        .bind(&e.changes)
        .bind(labels(&e.added_label_ids))
        .bind(labels(&e.removed_label_ids))
        .bind(&e.relation_changes)
        .bind(&e.flags)
        .bind(e.created_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn last_history(&mut self, issue: IssueId) -> AppResult<Option<HistoryEntry>> {
        let row = sqlx::query_as::<_, HistoryRow>(&format!(
            "SELECT {HISTORY_COLS} FROM issue_history WHERE organization_id = $1 AND issue_id = $2 \
             ORDER BY created_at DESC, id DESC LIMIT 1"
        ))
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(HistoryEntry::from))
    }

    async fn delete_history(&mut self, id: Uuid) -> AppResult<()> {
        sqlx::query("DELETE FROM issue_history WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(id)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn list_history(&mut self, issue: IssueId, limit: i64) -> AppResult<Vec<HistoryEntry>> {
        let rows = sqlx::query_as::<_, HistoryRow>(&format!(
            "SELECT {HISTORY_COLS} FROM issue_history WHERE organization_id = $1 AND issue_id = $2 \
             ORDER BY created_at, id LIMIT $3"
        ))
        .bind(self.org.0)
        .bind(issue.0)
        .bind(limit)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(HistoryEntry::from).collect())
    }

    async fn insert_relation(
        &mut self,
        id: IssueRelationId,
        issue: IssueId,
        related: IssueId,
        kind: RelationType,
        creator: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        // Symmetrische Typen kanonisch speichern: kleinere UUID als issue_id.
        let (a, b) = match kind {
            RelationType::Related | RelationType::Similar if related.0 < issue.0 => {
                (related, issue)
            }
            _ => (issue, related),
        };
        let res = sqlx::query(
            "INSERT INTO issue_relation (id, organization_id, issue_id, related_issue_id, type, creator_id, \
             created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7) ON CONFLICT DO NOTHING",
        )
        .bind(id.0)
        .bind(self.org.0)
        .bind(a.0)
        .bind(b.0)
        .bind(kind.as_str())
        .bind(creator.map(|c| c.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn delete_relation(
        &mut self,
        issue: IssueId,
        related: IssueId,
        kind: RelationType,
    ) -> AppResult<Option<Relation>> {
        let symmetric = matches!(kind, RelationType::Related | RelationType::Similar);
        let row = sqlx::query_as::<_, RelationRow>(&format!(
            "DELETE FROM issue_relation WHERE organization_id = $1 AND type = $4 AND \
             ((issue_id = $2 AND related_issue_id = $3) OR ($5 AND issue_id = $3 AND related_issue_id = $2)) \
             RETURNING {RELATION_COLS}"
        ))
        .bind(self.org.0)
        .bind(issue.0)
        .bind(related.0)
        .bind(kind.as_str())
        .bind(symmetric)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(RelationRow::into_relation).transpose()
    }

    async fn relations_of(&mut self, issue: IssueId) -> AppResult<Vec<Relation>> {
        let rows = sqlx::query_as::<_, RelationRow>(&format!(
            "SELECT {RELATION_COLS} FROM issue_relation WHERE organization_id = $1 \
             AND (issue_id = $2 OR related_issue_id = $2) ORDER BY created_at, id"
        ))
        .bind(self.org.0)
        .bind(issue.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(RelationRow::into_relation).collect()
    }

    async fn get_relation(&mut self, id: IssueRelationId) -> AppResult<Option<Relation>> {
        let row = sqlx::query_as::<_, RelationRow>(&format!(
            "SELECT {RELATION_COLS} FROM issue_relation WHERE organization_id = $1 AND id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(RelationRow::into_relation).transpose()
    }

    async fn delete_relation_by_id(&mut self, id: IssueRelationId) -> AppResult<bool> {
        let res = sqlx::query("DELETE FROM issue_relation WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(id.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn insert_alias(
        &mut self,
        issue: IssueId,
        identifier: &str,
        team_key: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO issue_identifier_alias (organization_id, identifier, entity_type, issue_id, team_key, \
             created_at, updated_at) VALUES ($1, $2, 'issue', $3, $4, $5, $5) \
             ON CONFLICT (organization_id, identifier) DO UPDATE SET issue_id = EXCLUDED.issue_id, \
             team_key = EXCLUDED.team_key, entity_type = 'issue', project_id = NULL, archived_at = NULL, \
             version = issue_identifier_alias.version + 1, updated_at = EXCLUDED.updated_at",
        )
        .bind(self.org.0)
        .bind(identifier.to_uppercase())
        .bind(issue.0)
        .bind(team_key)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn find_alias_for_keys(
        &mut self,
        issue: IssueId,
        keys: &[String],
    ) -> AppResult<Option<(String, i32)>> {
        let ident: Option<String> = sqlx::query_scalar(
            "SELECT identifier FROM issue_identifier_alias WHERE organization_id = $1 AND issue_id = $2 \
             AND team_key = ANY($3) AND archived_at IS NULL ORDER BY created_at DESC LIMIT 1",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(keys)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(ident.and_then(|i| {
            let n = i.rsplit('-').next()?.parse::<i32>().ok()?;
            Some((i, n))
        }))
    }

    async fn delete_alias(&mut self, identifier: &str) -> AppResult<()> {
        sqlx::query(
            "DELETE FROM issue_identifier_alias WHERE organization_id = $1 AND identifier = $2",
        )
        .bind(self.org.0)
        .bind(identifier.to_uppercase())
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn team_key_history(&mut self, team: TeamId) -> AppResult<Vec<String>> {
        sqlx::query_scalar(
            "SELECT key FROM team_key WHERE organization_id = $1 AND team_id = $2 ORDER BY replaced_at, key",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn find_cycles(&mut self, team: TeamId, reference: &str) -> AppResult<Vec<CycleLookup>> {
        let rows = sqlx::query_as::<_, CycleRow>(
            "SELECT id, team_id, number, name, starts_at, ends_at, completed_at, inherited_from_id FROM cycle \
             WHERE organization_id = $1 AND team_id = $2 AND archived_at IS NULL \
             AND (id::text = lower($3) OR number::text = $3 OR lower(name) = lower($3)) ORDER BY number",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(reference.trim())
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(CycleLookup::from).collect())
    }

    async fn find_cycle_inheriting(
        &mut self,
        team: TeamId,
        root: CycleId,
    ) -> AppResult<Option<CycleLookup>> {
        let row = sqlx::query_as::<_, CycleRow>(
            "SELECT id, team_id, number, name, starts_at, ends_at, completed_at, inherited_from_id FROM cycle \
             WHERE organization_id = $1 AND team_id = $2 AND archived_at IS NULL AND completed_at IS NULL \
             AND (id = $3 OR inherited_from_id = $3) ORDER BY starts_at LIMIT 1",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(root.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(CycleLookup::from))
    }

    async fn get_cycle(&mut self, id: CycleId) -> AppResult<Option<CycleLookup>> {
        let row = sqlx::query_as::<_, CycleRow>(
            "SELECT id, team_id, number, name, starts_at, ends_at, completed_at, inherited_from_id FROM cycle \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(CycleLookup::from))
    }

    async fn find_projects(&mut self, reference: &str) -> AppResult<Vec<ProjectLookup>> {
        let rows = sqlx::query_as::<_, ProjectRow>(&format!(
            "{PROJECT_SELECT} WHERE p.organization_id = $1 AND p.archived_at IS NULL AND NOT p.trashed \
             AND (p.id::text = lower($2) OR p.slug_id = lower($2) OR lower(p.name) = lower($2) \
                  OR 'P-' || lt.key || '-' || p.number = upper($2)) ORDER BY p.created_at, p.id"
        ))
        .bind(self.org.0)
        .bind(reference.trim())
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(ProjectLookup::from).collect())
    }

    async fn get_project(&mut self, id: ProjectId) -> AppResult<Option<ProjectLookup>> {
        let row = sqlx::query_as::<_, ProjectRow>(&format!(
            "{PROJECT_SELECT} WHERE p.organization_id = $1 AND p.id = $2"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(ProjectLookup::from))
    }

    async fn find_milestones(
        &mut self,
        project: ProjectId,
        reference: &str,
    ) -> AppResult<Vec<MilestoneLookup>> {
        let rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT id, project_id, name FROM project_milestone WHERE organization_id = $1 AND project_id = $2 \
             AND archived_at IS NULL AND (id::text = lower($3) OR lower(name) = lower($3)) ORDER BY sort_order, id",
        )
        .bind(self.org.0)
        .bind(project.0)
        .bind(reference.trim())
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, p, name)| MilestoneLookup {
                id: MilestoneId(id),
                project_id: ProjectId(p),
                name,
            })
            .collect())
    }

    async fn insert_share(&mut self, s: &IssueShare) -> AppResult<bool> {
        let res = sqlx::query(
            "INSERT INTO issue_share (organization_id, issue_id, user_id, shared_by_id, created_at) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
        )
        .bind(self.org.0)
        .bind(s.issue_id.0)
        .bind(s.user_id.0)
        .bind(s.shared_by_id.map(|u| u.0))
        .bind(s.created_at)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn delete_share(&mut self, issue: IssueId, user: UserId) -> AppResult<bool> {
        let res = sqlx::query(
            "DELETE FROM issue_share WHERE organization_id = $1 AND issue_id = $2 AND user_id = $3",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(user.0)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn get_share(&mut self, issue: IssueId, user: UserId) -> AppResult<Option<IssueShare>> {
        let row: Option<(Uuid, Uuid, Option<Uuid>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT issue_id, user_id, shared_by_id, created_at FROM issue_share \
             WHERE organization_id = $1 AND issue_id = $2 AND user_id = $3",
        )
        .bind(self.org.0)
        .bind(issue.0)
        .bind(user.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(i, u, by, at)| IssueShare {
            issue_id: IssueId(i),
            user_id: UserId(u),
            shared_by_id: by.map(UserId),
            created_at: at,
        }))
    }

    async fn delete(&mut self, id: IssueId) -> AppResult<()> {
        sqlx::query("DELETE FROM issue WHERE organization_id = $1 AND id = $2")
            .bind(self.org.0)
            .bind(id.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn subscribe(
        &mut self,
        id: Uuid,
        issue: IssueId,
        user: UserId,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO issue_subscription (id, organization_id, issue_id, user_id, reason, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(self.org.0)
        .bind(issue.0)
        .bind(user.0)
        .bind(reason)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }
}
