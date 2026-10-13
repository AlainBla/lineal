//! usecases – Issues (docs/03 §2–5, §8–9, §12; docs/02 §7): anlegen, ändern (inkl. Statuswechsel,
//! Teamwechsel, Patch), Relationen, Labels, History, Papierkorb, Sharing, Listen.
//!
//! Jede Schreiboperation läuft in genau einer Transaktion inkl. History und Outbox-Event.

mod batch;
pub(crate) mod fields;
pub(crate) mod history;
pub(crate) mod mentions;
mod relations;

pub use batch::{BatchFailure, BatchPatch, BatchResult, BatchUpdateInput, MAX_BATCH};
pub use fields::MAX_SUB_ISSUE_DEPTH;
pub use relations::{RelatedIssue, RelationChanges, RelationsView};

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::document::DocParent;
use crate::ports::repos::issue::{
    HistoryEntry, HistorySource, Issue, IssueExtras, IssueListFilter, IssueShare,
};
use crate::ports::repos::label::LabelKind;
use crate::ports::repos::team::Team;
use crate::ports::repos::user::{OrderBy, PageCursor, PageRequest};
use crate::ports::repos::view::ViewModel;
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::actor_user_id;
use crate::usecases::view;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use fields::{
    LabelPlan, PlanningChange, apply_planning, change_state, convert_estimate, default_state,
    estimation_of, first_state_of, keep_person_after_move, labels_available_in, map_cycle,
    map_estimate, map_state, plan_labels, resolve_assignee, resolve_delegate, resolve_parent,
    triage_state, validate_description, validate_title,
};
use history::Delta;
use lineal_domain::enums::{Priority, SlaType, StateType};
use lineal_domain::events::{DomainEvent, IssuePrevious};
use lineal_domain::ids::{IssueId, LabelId, StateId, TeamId, UserId};
use lineal_domain::rules::patch::{self, PatchFailure, PatchOp};
use lineal_domain::rules::state::{IssueTimestamps, TransitionSource, apply_transition};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

/// Ein Issue mit Anzeigewerten.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueView {
    pub issue: Issue,
    pub extras: IssueExtras,
    /// Frühere Kennungen (Aliase und frühere Team-Keys); nur bei `get`/`save` gefüllt.
    pub previous_identifiers: Vec<String>,
}

/// Verweildauer in einem Status (`stateHistory`).
#[derive(Debug, Clone, PartialEq)]
pub struct StateSpan {
    pub state_id: StateId,
    pub name: String,
    pub state_type: StateType,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// Ein Issue vollständig (für `get_issue`).
#[derive(Debug, Clone, PartialEq)]
pub struct IssueDetail {
    pub view: IssueView,
    pub relations: RelationsView,
    pub state_history: Vec<StateSpan>,
}

/// Seite einer Issue-Liste.
#[derive(Debug, Clone, PartialEq)]
pub struct IssuePage {
    pub items: Vec<IssueView>,
    pub next: Option<PageCursor>,
}

/// Änderung der Beschreibung.
#[derive(Debug, Clone, PartialEq)]
pub enum DescriptionEdit {
    /// Ersetzt die Beschreibung.
    Set(String),
    /// Teiländerungen (atomar), nur bei Updates.
    Patch(Vec<PatchOp>),
}

/// Eingabe von `create_issue`; Referenzen sind Strings (UUID, Kennung, Name …).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateIssueInput {
    /// Client-generierte ID (optimistische Updates).
    pub id: Option<IssueId>,
    pub team: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub state: Option<String>,
    pub priority: Option<i64>,
    pub assignee: Option<String>,
    pub delegate: Option<String>,
    pub estimate: Option<f64>,
    pub labels: Option<Vec<String>>,
    pub add_labels: Vec<String>,
    pub due_date: Option<NaiveDate>,
    pub cycle: Option<String>,
    pub project: Option<String>,
    pub milestone: Option<String>,
    pub parent: Option<String>,
    pub sort_order: Option<f64>,
    pub sla_breaches_at: Option<DateTime<Utc>>,
    pub sla_type: Option<SlaType>,
    pub relations: RelationChanges,
    pub source: HistorySource,
}

/// Eingabe von `update_issue` (Tri-State: `None` unverändert, `Some(None)` leeren).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateIssueInput {
    /// Referenz des Issues (UUID, Kennung, frühere Kennung).
    pub id: String,
    pub team: Option<String>,
    pub title: Option<String>,
    pub description: Option<DescriptionEdit>,
    pub state: Option<String>,
    pub priority: Option<i64>,
    pub assignee: Option<Option<String>>,
    pub delegate: Option<Option<String>>,
    pub estimate: Option<Option<f64>>,
    pub labels: Option<Vec<String>>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub due_date: Option<Option<NaiveDate>>,
    pub cycle: Option<Option<String>>,
    pub project: Option<Option<String>>,
    pub milestone: Option<Option<String>>,
    pub parent: Option<Option<String>>,
    pub sort_order: Option<f64>,
    pub sla_breaches_at: Option<Option<DateTime<Utc>>>,
    pub sla_type: Option<Option<SlaType>>,
    pub relations: RelationChanges,
    /// Bei einem Teamwechsel auch alle Nachfahren verschieben (docs/02 §7.2 Nr. 14).
    pub move_sub_issues: bool,
    pub source: HistorySource,
}

/// Filter von `list_issues` (MCP-Parameter inkl. `customView` und `query`, WP11).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListIssuesInput {
    /// Gespeicherte View (UUID, `slugId`, URL oder exakter Name): Filter und Scope der View UND
    /// alle weiteren Parameter; Sortierung nicht aus der View (docs/05 §4.3).
    pub custom_view: Option<String>,
    /// Volltext in Titel/Beschreibung oder eine Kennung (auch frühere); höchstens 500 Treffer.
    /// Nicht mit `custom_view` kombinierbar.
    pub query: Option<String>,
    /// Zusätzlicher Filterbaum (`IssueFilter` als JSON, z. B. aus GraphQL); wird strikt
    /// validiert (Referenzen müssen sichtbar existieren) und per UND angewendet.
    pub filter: Option<Value>,
    pub team: Option<String>,
    pub include_sub_teams: bool,
    /// `Some(None)` = ohne Assignee (`null` bzw. `"null"`).
    pub assignee: Option<Option<String>>,
    pub creator: Option<String>,
    pub delegate: Option<String>,
    pub state: Option<String>,
    pub label: Option<String>,
    pub project: Option<String>,
    pub cycle: Option<String>,
    pub parent: Option<String>,
    pub priority: Option<i64>,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub triaged_after: Option<DateTime<Utc>>,
    pub include_archived: bool,
    pub order_by: OrderBy,
    pub limit: i64,
    pub cursor: Option<PageCursor>,
}

/// Ergebnis von `share_issue`/`unshare_issue`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShareResult {
    pub issue: Issue,
    pub user_id: UserId,
    pub user_name: String,
    pub shared: bool,
    pub shared_at: Option<DateTime<Utc>>,
    pub message: Option<String>,
}

fn invalid(field: &str, reason: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field, "reason": reason }))
}

fn val_uuid<T: Copy + Into<uuid::Uuid>>(v: Option<T>) -> Value {
    v.map_or(Value::Null, |i| {
        Value::String(Into::<uuid::Uuid>::into(i).to_string())
    })
}

fn val_ts(v: Option<DateTime<Utc>>) -> Value {
    v.map_or(Value::Null, |t| {
        Value::String(t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    })
}

/// Vergleicht zwei Zustände und liefert die History-Änderungen sowie die geänderten
/// Feldnamen (camelCase) für `IssueUpdated.changed`.
fn diff_issue(old: &Issue, new: &Issue) -> (Delta, Vec<String>) {
    let mut d = Delta::default();
    d.set("title", json!(old.title), json!(new.title));
    d.set(
        "stateId",
        val_uuid(Some(old.state_id)),
        val_uuid(Some(new.state_id)),
    );
    d.set(
        "priority",
        json!(old.priority.value()),
        json!(new.priority.value()),
    );
    d.set("estimate", json!(old.estimate), json!(new.estimate));
    d.set(
        "assigneeId",
        val_uuid(old.assignee_id),
        val_uuid(new.assignee_id),
    );
    d.set(
        "delegateId",
        val_uuid(old.delegate_id),
        val_uuid(new.delegate_id),
    );
    d.set(
        "teamId",
        val_uuid(Some(old.team_id)),
        val_uuid(Some(new.team_id)),
    );
    d.set("identifier", json!(old.identifier), json!(new.identifier));
    d.set("parentId", val_uuid(old.parent_id), val_uuid(new.parent_id));
    d.set("cycleId", val_uuid(old.cycle_id), val_uuid(new.cycle_id));
    d.set(
        "projectId",
        val_uuid(old.project_id),
        val_uuid(new.project_id),
    );
    d.set(
        "projectMilestoneId",
        val_uuid(old.project_milestone_id),
        val_uuid(new.project_milestone_id),
    );
    d.set(
        "dueDate",
        json!(old.due_date.map(|x| x.to_string())),
        json!(new.due_date.map(|x| x.to_string())),
    );
    d.set(
        "slaBreachesAt",
        val_ts(old.sla_breaches_at),
        val_ts(new.sla_breaches_at),
    );
    d.set(
        "slaStartedAt",
        val_ts(old.sla_started_at),
        val_ts(new.sla_started_at),
    );
    d.set(
        "slaType",
        json!(old.sla_type.map(|t| t.as_str())),
        json!(new.sla_type.map(|t| t.as_str())),
    );
    d.set(
        "snoozedUntilAt",
        val_ts(old.snoozed_until_at),
        val_ts(new.snoozed_until_at),
    );
    let mut changed: Vec<String> = d.changes.keys().cloned().collect();
    changed.retain(|k| k != "identifier");
    if old.description != new.description {
        changed.push("description".into());
        d.flag("descriptionUpdated");
    }
    if (old.sort_order - new.sort_order).abs() > f64::EPSILON {
        changed.push("sortOrder".into());
    }
    if (old.priority_sort_order - new.priority_sort_order).abs() > f64::EPSILON {
        changed.push("prioritySortOrder".into());
    }
    if old.sub_issue_sort_order != new.sub_issue_sort_order {
        changed.push("subIssueSortOrder".into());
    }
    if old.trashed != new.trashed {
        changed.push("trashed".into());
    }
    if old.archived_at != new.archived_at {
        changed.push("archivedAt".into());
    }
    (d, changed)
}

fn patch_error(e: patch::PatchError) -> AppError {
    let details = json!({
        "field": "description", "opIndex": e.op_index, "op": e.op,
        "anchor": e.anchor, "matches": e.matches,
    });
    match e.reason {
        PatchFailure::AnchorNotFound | PatchFailure::AnchorAmbiguous => {
            AppError::conflict(e.to_string()).with_details(details)
        }
        PatchFailure::LimitExceeded => {
            AppError::limit_exceeded(e.to_string()).with_details(details)
        }
        PatchFailure::NoOps | PatchFailure::TooManyOps | PatchFailure::EmptyArgument => {
            AppError::invalid_request(e.to_string()).with_details(details)
        }
    }
}

fn priority_of(value: i64) -> AppResult<Priority> {
    u8::try_from(value)
        .ok()
        .and_then(|v| Priority::try_from(v).ok())
        .ok_or_else(|| {
            invalid(
                "priority",
                "out_of_range",
                "priority must be between 0 and 4",
            )
        })
}

/// SLA und Fälligkeit (docs/03 §2.6): entweder `dueDate` oder SLA.
fn apply_due_and_sla(
    issue: &mut Issue,
    due: Option<Option<NaiveDate>>,
    sla_at: Option<Option<DateTime<Utc>>>,
    sla_type: Option<Option<SlaType>>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if matches!(due, Some(Some(_))) && matches!(sla_at, Some(Some(_))) {
        return Err(invalid(
            "dueDate",
            "due_date_and_sla",
            "dueDate and slaBreachesAt cannot be set in the same request",
        ));
    }
    let clear_sla = |i: &mut Issue| {
        i.sla_type = None;
        i.sla_started_at = None;
        i.sla_medium_risk_at = None;
        i.sla_high_risk_at = None;
        i.sla_breaches_at = None;
    };
    if let Some(d) = due {
        issue.due_date = d;
        if d.is_some() && issue.sla_breaches_at.is_some() {
            clear_sla(issue);
        }
    }
    match sla_at {
        Some(Some(t)) => {
            if t <= now {
                return Err(invalid(
                    "slaBreachesAt",
                    "sla_in_the_past",
                    "slaBreachesAt must be in the future",
                ));
            }
            issue.sla_breaches_at = Some(t);
            let ty = match sla_type {
                Some(Some(ty)) => ty,
                _ => issue.sla_type.unwrap_or(SlaType::All),
            };
            issue.sla_type = Some(ty);
            issue.sla_started_at.get_or_insert(now);
            issue.sla_medium_risk_at = None;
            issue.sla_high_risk_at = None;
            issue.due_date = None;
        }
        Some(None) => clear_sla(issue),
        None => match sla_type {
            Some(Some(ty)) if issue.sla_breaches_at.is_some() => issue.sla_type = Some(ty),
            Some(Some(_)) => {
                return Err(invalid(
                    "slaType",
                    "sla_type_without_sla",
                    "slaType can only be used together with slaBreachesAt",
                ));
            }
            Some(None) if issue.sla_breaches_at.is_some() => clear_sla(issue),
            _ => {}
        },
    }
    Ok(())
}

async fn emit(tx: &mut dyn Tx, ctx: &RequestCtx, event: DomainEvent) -> AppResult<()> {
    tx.outbox().push(NewEvent::from_ctx(event, ctx)).await?;
    Ok(())
}

async fn load_view(tx: &mut dyn Tx, issue: Issue, with_previous: bool) -> AppResult<IssueView> {
    let mut extras = tx.issues().extras(&[issue.id]).await?;
    let previous_identifiers = if with_previous {
        tx.issues().previous_identifiers(issue.id).await?
    } else {
        Vec::new()
    };
    Ok(IssueView {
        extras: extras.remove(&issue.id).unwrap_or_default(),
        issue,
        previous_identifiers,
    })
}

async fn load_views(tx: &mut dyn Tx, issues: Vec<Issue>) -> AppResult<Vec<IssueView>> {
    let ids: Vec<IssueId> = issues.iter().map(|i| i.id).collect();
    let mut extras: HashMap<IssueId, IssueExtras> = tx.issues().extras(&ids).await?;
    Ok(issues
        .into_iter()
        .map(|issue| IssueView {
            extras: extras.remove(&issue.id).unwrap_or_default(),
            issue,
            previous_identifiers: Vec::new(),
        })
        .collect())
}

fn team_of(snap: &PolicySnapshot, id: TeamId) -> AppResult<&Team> {
    snap.tree.get(id).ok_or_else(crate::policy::team_not_found)
}

impl App {
    /// Legt ein Issue an (docs/03 §2.1).
    pub async fn create_issue(
        &self,
        ctx: &RequestCtx,
        input: CreateIssueInput,
    ) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let view = self.create_issue_in(&mut *tx, ctx, input).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Wie [`App::create_issue`] in einer vorhandenen Transaktion (z. B. für Templates, Import).
    pub async fn create_issue_in(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        input: CreateIssueInput,
    ) -> AppResult<IssueView> {
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let now = self.clock.now();
        let actor = actor_user_id(ctx);
        let team = resolve::team::resolve_in(tx, &snap, &input.team, Visibility::Content).await?;
        snap.require_team(ctx, Action::CreateIssue, team.id)?;

        let title = validate_title(input.title.as_deref().ok_or_else(|| {
            invalid(
                "title",
                "required",
                "title is required when creating an issue",
            )
        })?)?;
        let description = match input.description.clone() {
            Some(d) => validate_description(d)?,
            None => None,
        };
        if input.labels.is_some() && !input.add_labels.is_empty() {
            return Err(invalid(
                "labels",
                "labels_and_add_labels",
                "labels cannot be combined with addLabels",
            ));
        }
        if let Some(max) = snap.org.limits.max_active_issues_per_team
            && tx.issues().count_open(team.id).await? >= max
        {
            return Err(AppError::limit_exceeded(
                "the team has reached its limit of active issues",
            )
            .with_details(json!({ "limit": "maxActiveIssuesPerTeam", "max": max })));
        }

        // Status: explizit > Triage (Nicht-Mitglied) > Default
        let state = match &input.state {
            Some(r) => {
                let s = resolve::state::resolve(tx, &team, r).await?;
                if s.state_type == StateType::Duplicate {
                    return Err(invalid(
                        "state",
                        "duplicate_requires_mark_duplicate",
                        "the duplicate state can only be reached by marking the issue as a duplicate",
                    ));
                }
                if s.archived_at.is_some() {
                    return Err(invalid("state", "state_archived", "state is archived"));
                }
                s
            }
            None => {
                let triage = if team.triage_enabled && !snap.actor.is_member(team.id) {
                    triage_state(tx, &team).await?
                } else {
                    None
                };
                match triage {
                    Some(t) => t,
                    None => default_state(tx, &team).await?,
                }
            }
        };

        let id = input.id.unwrap_or_else(|| IssueId(self.ids.new_uuid()));
        let mut issue = Issue {
            id,
            org_id: ctx.org_id,
            team_id: team.id,
            number: 0,
            identifier: String::new(),
            title,
            description,
            state_id: state.id,
            state_type: state.state_type,
            state_entered_at: now,
            priority: Priority::NoPriority,
            estimate: None,
            assignee_id: None,
            delegate_id: None,
            creator_id: actor,
            parent_id: None,
            project_id: None,
            project_milestone_id: None,
            cycle_id: None,
            due_date: None,
            sort_order: 0.0,
            priority_sort_order: 0.0,
            sub_issue_sort_order: None,
            started_triage_at: None,
            triaged_at: None,
            started_at: None,
            completed_at: None,
            canceled_at: None,
            auto_closed_at: None,
            snoozed_until_at: None,
            snoozed_by_id: None,
            added_to_team_at: now,
            added_to_cycle_at: None,
            added_to_project_at: None,
            sla_type: None,
            sla_started_at: None,
            sla_medium_risk_at: None,
            sla_high_risk_at: None,
            sla_breaches_at: None,
            trashed: false,
            trashed_at: None,
            archived_at: None,
            version: 1,
            created_at: now,
            updated_at: now,
        };
        // Initiale Zeitstempel nach Kategorie.
        let ts = apply_transition(
            StateType::Backlog,
            state.state_type,
            now,
            IssueTimestamps::default(),
        );
        issue.started_triage_at = ts.started_triage_at;
        issue.started_at = ts.started_at;
        issue.completed_at = ts.completed_at;
        issue.canceled_at = ts.canceled_at;

        // Parent und Vererbung (docs/03 §3.2)
        let mut inherited_project = None;
        if let Some(r) = &input.parent {
            let parent = resolve_parent(tx, &snap, ctx, &issue, r).await?;
            issue.parent_id = Some(parent.id);
            let max = tx.issues().max_sub_issue_sort_order(parent.id).await?;
            issue.sub_issue_sort_order = Some(max.map_or(1.0, |m| m + 1.0));
            if input.priority.is_none() {
                issue.priority = parent.priority;
            }
            if let Some(p) = parent.project_id
                && input.project.is_none()
                && let Some(pl) = tx.issues().get_project(p).await?
                && pl.team_ids.contains(&team.id)
            {
                inherited_project = Some(p);
            }
        }
        if let Some(p) = input.priority {
            issue.priority = priority_of(p)?;
        }
        if let Some(e) = input.estimate {
            issue.estimate = Some(convert_estimate(&snap.tree, &team, e)?);
        }
        if let Some(r) = &input.assignee {
            issue.assignee_id = Some(resolve_assignee(tx, &snap, ctx, &team, r).await?.id);
        }
        if let Some(r) = &input.delegate {
            issue.delegate_id = Some(resolve_delegate(tx, &snap, ctx, &team, r).await?.id);
        }
        apply_due_and_sla(
            &mut issue,
            input.due_date.map(Some),
            input.sla_breaches_at.map(Some),
            input.sla_type.map(Some),
            now,
        )?;
        if let Some(p) = inherited_project {
            issue.project_id = Some(p);
            issue.added_to_project_at = Some(now);
        }
        apply_planning(
            tx,
            &snap,
            ctx,
            &team,
            &mut issue,
            &PlanningChange {
                project: input.project.clone().map(Some),
                milestone: input.milestone.clone().map(Some),
                cycle: input.cycle.clone().map(Some),
            },
            input.state.is_some(),
            now,
        )
        .await?;
        // Auto-Zuordnung beim Anlegen (docs/04 §1.6, Hook mit oldType = null).
        if issue.cycle_id.is_none()
            && let Some(c) = Box::pin(crate::usecases::cycle::auto_cycle_for_state(
                tx,
                &team,
                issue.state_type,
                now,
            ))
            .await?
        {
            issue.cycle_id = Some(c.id);
            issue.added_to_cycle_at = Some(now);
        }
        let plan: LabelPlan = if input.labels.is_some() || !input.add_labels.is_empty() {
            plan_labels(
                tx,
                &snap.tree,
                team.id,
                &[],
                &[],
                input.labels.as_deref(),
                &input.add_labels,
                &[],
            )
            .await?
        } else {
            LabelPlan::default()
        };
        // sortOrder: oben im Zielstatus (docs/03 §2.1)
        issue.sort_order = match input.sort_order {
            Some(v) => v,
            None => {
                let (min, _) = tx.issues().sort_extent(team.id, issue.state_id).await?;
                min.map_or(0.0, |m| m - 1.0)
            }
        };
        issue.priority_sort_order = issue.sort_order;

        // Nummer erst jetzt reservieren (hält die Zeile des Teams bis zum Commit gesperrt).
        issue.number = tx.teams().allocate_issue_numbers(team.id, 1).await?;
        let mut stored = tx.issues().insert(&issue).await?;
        tx.issues()
            .apply_labels(stored.id, &[], &plan.added, now)
            .await?;

        // Relationen (kann den Status auf `duplicate` setzen)
        let mut own = Delta::default();
        if !input.relations.is_empty() {
            let env = relations::Env {
                ctx,
                snap: &snap,
                ids: self.ids.as_ref(),
                now,
                source: input.source,
            };
            let before = stored.clone();
            relations::apply(tx, &env, &team, &mut stored, &input.relations, &mut own).await?;
            if stored != before {
                stored = tx.issues().save(&stored, now).await?;
            }
        }
        history::record_created(tx, self.ids.new_uuid(), &stored, actor, input.source, now).await?;
        // Automationen von WP12: SLA-Regeln und Triage-Verantwortung für das neue Issue.
        {
            let before = stored.clone();
            crate::usecases::triage::sla::evaluate_rules(tx, &team, &mut stored, now).await?;
            if stored.state_type == StateType::Triage {
                crate::usecases::triage::automation::run_triage_responsibility(
                    tx,
                    &team,
                    &mut stored,
                    now,
                )
                .await?;
            }
            if stored != before {
                stored = tx.issues().save(&stored, now).await?;
            }
        }

        // Abos: Creator, Assignee, Erwähnte
        let mentioned = match &stored.description {
            Some(d) => mentions::resolve_mentions(tx, &snap, team.id, d).await?,
            None => Vec::new(),
        };
        if let Some(a) = actor
            && matches!(ctx.actor, crate::context::Actor::User { .. })
        {
            tx.issues()
                .subscribe(self.ids.new_uuid(), stored.id, a, "creator", now)
                .await?;
        }
        if let Some(a) = stored.assignee_id {
            tx.issues()
                .subscribe(self.ids.new_uuid(), stored.id, a, "assignee", now)
                .await?;
        }
        for u in &mentioned {
            tx.issues()
                .subscribe(self.ids.new_uuid(), stored.id, *u, "mention", now)
                .await?;
        }
        if let Some(d) = &stored.description {
            Box::pin(mentions::link_issue_mentions(
                tx,
                &snap,
                ctx,
                self.ids.as_ref(),
                now,
                input.source,
                &stored,
                d,
                None,
            ))
            .await?;
        }
        emit(
            tx,
            ctx,
            DomainEvent::IssueCreated {
                issue_id: stored.id,
                team_id: stored.team_id,
                mentioned_user_ids: mentioned,
            },
        )
        .await?;
        load_view(tx, stored, true).await
    }

    /// Ändert ein Issue (docs/03 §2.2, inkl. Teamwechsel nach docs/02 §7).
    pub async fn update_issue(
        &self,
        ctx: &RequestCtx,
        input: UpdateIssueInput,
    ) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let view = self.update_issue_in(&mut *tx, ctx, input).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Wie [`App::update_issue`] in einer vorhandenen Transaktion.
    pub async fn update_issue_in(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        input: UpdateIssueInput,
    ) -> AppResult<IssueView> {
        let move_subs = input.move_sub_issues && input.team.is_some();
        let source = input.source;
        let view = Box::pin(self.update_issue_core(tx, ctx, input)).await?;
        if move_subs {
            self.move_descendants(tx, ctx, &view.issue, source).await?;
        }
        Ok(view)
    }

    /// Verschiebt alle Nachfahren von `root` ins Team von `root` (gleiche Transaktion, docs/02
    /// §7.2 Nr. 14). Für den Aufrufer nicht sichtbare Nachfahren bleiben unberührt.
    async fn move_descendants(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        root: &Issue,
        source: HistorySource,
    ) -> AppResult<()> {
        const MAX_MOVED: usize = 500;
        let mut queue = vec![(root.id, 0_i64)];
        let mut seen = std::collections::HashSet::from([root.id]);
        let mut handled = 0_usize;
        while let Some((id, depth)) = queue.pop() {
            if depth >= MAX_SUB_ISSUE_DEPTH {
                continue;
            }
            for child in tx.issues().children(id).await? {
                if !seen.insert(child.id) {
                    continue;
                }
                queue.push((child.id, depth + 1));
                if child.trashed || child.archived_at.is_some() || child.team_id == root.team_id {
                    continue;
                }
                handled += 1;
                if handled > MAX_MOVED {
                    return Err(AppError::limit_exceeded(
                        "too many sub-issues to move in one call",
                    )
                    .with_details(json!({ "limit": "moveSubIssues", "max": MAX_MOVED })));
                }
                let moved = Box::pin(self.update_issue_core(
                    tx,
                    ctx,
                    UpdateIssueInput {
                        id: child.id.to_string(),
                        team: Some(root.team_id.to_string()),
                        source,
                        ..Default::default()
                    },
                ))
                .await;
                match moved {
                    Ok(_) => {}
                    // Unsichtbare Sub-Issues verraten wir nicht und verschieben sie nicht.
                    Err(e) if e.kind == crate::error::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }

    async fn update_issue_core(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        input: UpdateIssueInput,
    ) -> AppResult<IssueView> {
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let now = self.clock.now();
        let actor = actor_user_id(ctx);
        let found = resolve::issue::resolve_in(tx, &snap, ctx, &input.id).await?;
        if found.trashed {
            return Err(invalid(
                "id",
                "issue_trashed",
                "the issue is in the trash; restore it first",
            ));
        }
        if found.archived_at.is_some() {
            return Err(invalid(
                "id",
                "issue_archived",
                "archived issues are read-only",
            ));
        }
        snap.require_team(ctx, Action::UpdateIssue, found.team_id)?;
        // Zeile sperren und frischen Stand lesen.
        let mut issue = tx
            .issues()
            .get(found.id, true)
            .await?
            .ok_or_else(|| resolve::issue::issue_not_found(&input.id))?;

        if input.labels.is_some()
            && (!input.add_labels.is_empty() || !input.remove_labels.is_empty())
        {
            return Err(invalid(
                "labels",
                "labels_and_add_labels",
                "labels cannot be combined with addLabels/removeLabels",
            ));
        }
        let old = issue.clone();
        let old_labels = tx.issues().label_ids(issue.id).await?;
        let src_team = team_of(&snap, issue.team_id)?.clone();
        let mut team = src_team.clone();
        let mut moved = false;
        if let Some(r) = &input.team {
            let z = resolve::team::resolve_in(tx, &snap, r, Visibility::Content).await?;
            if z.id != issue.team_id {
                if !input.add_labels.is_empty() || !input.remove_labels.is_empty() {
                    return Err(invalid(
                        "team",
                        "team_change_with_add_labels",
                        "addLabels/removeLabels cannot be combined with a team change",
                    ));
                }
                moved = true;
                team = z;
            }
        }
        let mut label_baseline = old_labels.clone();
        if moved {
            label_baseline = self
                .move_to_team(
                    tx,
                    &snap,
                    ctx,
                    now,
                    &mut issue,
                    &src_team,
                    &team,
                    &old_labels,
                )
                .await?;
        }

        // Felder
        if let Some(t) = &input.title {
            issue.title = validate_title(t)?;
        }
        let mut mention_text: Option<String> = None;
        match &input.description {
            Some(DescriptionEdit::Set(d)) => {
                issue.description = validate_description(d.clone())?;
                mention_text = issue.description.clone();
            }
            Some(DescriptionEdit::Patch(ops)) => {
                let base = issue.description.clone().unwrap_or_default();
                let out = patch::apply(&base, ops).map_err(patch_error)?;
                issue.description = validate_description(out)?;
                mention_text = issue.description.clone();
            }
            None => {}
        }
        if let Some(p) = input.priority {
            issue.priority = priority_of(p)?;
        }
        if let Some(e) = &input.estimate {
            issue.estimate = match e {
                Some(v) => Some(convert_estimate(&snap.tree, &team, *v)?),
                None => None,
            };
        }
        if let Some(a) = &input.assignee {
            issue.assignee_id = match a {
                Some(r) => Some(resolve_assignee(tx, &snap, ctx, &team, r).await?.id),
                None => None,
            };
        }
        if let Some(d) = &input.delegate {
            issue.delegate_id = match d {
                Some(r) => Some(resolve_delegate(tx, &snap, ctx, &team, r).await?.id),
                None => None,
            };
        }
        if let Some(r) = &input.state {
            let s = resolve::state::resolve(tx, &team, r).await?;
            change_state(
                tx,
                &team,
                &mut issue,
                &s,
                TransitionSource::Update,
                input.sort_order,
                now,
                true,
            )
            .await?;
        }
        apply_due_and_sla(
            &mut issue,
            input.due_date,
            input.sla_breaches_at,
            input.sla_type,
            now,
        )?;
        apply_planning(
            tx,
            &snap,
            ctx,
            &team,
            &mut issue,
            &PlanningChange {
                project: input.project.clone(),
                milestone: input.milestone.clone(),
                cycle: input.cycle.clone(),
            },
            input.state.is_some(),
            now,
        )
        .await?;
        if let Some(p) = &input.parent {
            match p {
                Some(r) => {
                    let parent = resolve_parent(tx, &snap, ctx, &issue, r).await?;
                    if issue.parent_id != Some(parent.id) {
                        issue.parent_id = Some(parent.id);
                        let max = tx.issues().max_sub_issue_sort_order(parent.id).await?;
                        issue.sub_issue_sort_order = Some(max.map_or(1.0, |m| m + 1.0));
                    }
                }
                None => {
                    issue.parent_id = None;
                    issue.sub_issue_sort_order = None;
                }
            }
        }
        if let Some(s) = input.sort_order {
            issue.sort_order = s;
        }
        // Labels
        let plan = if input.labels.is_some()
            || !input.add_labels.is_empty()
            || !input.remove_labels.is_empty()
            || moved
        {
            plan_labels(
                tx,
                &snap.tree,
                team.id,
                &old_labels,
                &label_baseline,
                input.labels.as_deref(),
                &input.add_labels,
                &input.remove_labels,
            )
            .await?
        } else {
            LabelPlan::default()
        };
        // Relationen
        let mut own = Delta::default();
        if !input.relations.is_empty() {
            let env = relations::Env {
                ctx,
                snap: &snap,
                ids: self.ids.as_ref(),
                now,
                source: input.source,
            };
            relations::apply(tx, &env, &team, &mut issue, &input.relations, &mut own).await?;
        }

        let labels_changed = !plan.added.is_empty() || !plan.removed.is_empty();
        if issue == old && !labels_changed && own.is_empty() {
            return load_view(tx, issue, true).await;
        }
        // Aktivität eines anderen als des Snoozenden beendet das Triage-Snooze (docs/03 §10).
        if issue != old && issue.snoozed_until_at.is_some() && issue.snoozed_by_id != actor {
            issue.snoozed_until_at = None;
            issue.snoozed_by_id = None;
        }
        let saved = if issue != old {
            tx.issues().save(&issue, now).await?
        } else {
            issue.clone()
        };
        if labels_changed {
            tx.issues()
                .apply_labels(saved.id, &plan.removed, &plan.added, now)
                .await?;
        }
        let (mut delta, mut changed) = diff_issue(&old, &saved);
        delta.added_labels = plan.added.iter().map(|(l, _)| *l).collect();
        delta.removed_labels = plan.removed.clone();
        delta.relation_changes = own.relation_changes;
        if labels_changed {
            changed.push("labelIds".into());
        }
        history::record(
            tx,
            self.ids.new_uuid(),
            &saved,
            actor,
            input.source,
            &delta,
            now,
        )
        .await?;

        // Abos und Erwähnungen (nur neue)
        let mut new_mentions = Vec::new();
        if let Some(text) = mention_text {
            let before = match &old.description {
                Some(d) => mentions::resolve_mentions(tx, &snap, saved.team_id, d).await?,
                None => Vec::new(),
            };
            let after = mentions::resolve_mentions(tx, &snap, saved.team_id, &text).await?;
            new_mentions = after.into_iter().filter(|u| !before.contains(u)).collect();
            for u in &new_mentions {
                tx.issues()
                    .subscribe(self.ids.new_uuid(), saved.id, *u, "mention", now)
                    .await?;
            }
            Box::pin(mentions::link_issue_mentions(
                tx,
                &snap,
                ctx,
                self.ids.as_ref(),
                now,
                input.source,
                &saved,
                &text,
                old.description.as_deref(),
            ))
            .await?;
        }
        if saved.assignee_id != old.assignee_id
            && let Some(a) = saved.assignee_id
        {
            tx.issues()
                .subscribe(self.ids.new_uuid(), saved.id, a, "assignee", now)
                .await?;
        }
        if !changed.is_empty() {
            let previous = IssuePrevious::of(&changed, old.assignee_id, old.state_type.as_str());
            emit(
                tx,
                ctx,
                DomainEvent::IssueUpdated {
                    previous,
                    issue_id: saved.id,
                    team_id: saved.team_id,
                    changed,
                    mentioned_user_ids: new_mentions,
                },
            )
            .await?;
        }
        load_view(tx, saved, true).await
    }

    /// Teamwechsel nach docs/02 §7.2 (Schritte 1–13). Verändert `issue` in-memory und liefert die
    /// um nicht verfügbare Labels bereinigte Label-Menge; gespeichert wird vom Aufrufer.
    #[allow(clippy::too_many_arguments)]
    async fn move_to_team(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        now: DateTime<Utc>,
        issue: &mut Issue,
        src: &Team,
        z: &Team,
        old_labels: &[LabelId],
    ) -> AppResult<Vec<LabelId>> {
        snap.require_team(ctx, Action::CreateIssue, z.id)?;
        if issue.archived_at.is_none()
            && let Some(max) = snap.org.limits.max_active_issues_per_team
            && tx.issues().count_open(z.id).await? + 1 > max
        {
            return Err(AppError::limit_exceeded(
                "the target team has reached its limit of active issues",
            )
            .with_details(json!({ "limit": "maxActiveIssuesPerTeam", "max": max })));
        }
        // Neue Nummer: frühere Nummer im Zielteam wiederverwenden, sonst reservieren.
        let mut keys = vec![z.key.clone()];
        keys.extend(tx.issues().team_key_history(z.id).await?);
        let number = match tx.issues().find_alias_for_keys(issue.id, &keys).await? {
            Some((ident, n)) => {
                tx.issues().delete_alias(&ident).await?;
                n
            }
            None => tx.teams().allocate_issue_numbers(z.id, 1).await?,
        };
        // Alte Kennung sichern.
        tx.issues()
            .insert_alias(
                issue.id,
                &format!("{}-{}", src.key, issue.number),
                &src.key,
                now,
            )
            .await?;
        let src_state = resolve::state::by_id(tx, issue.state_id).await?;
        let mapped = map_state(tx, z, &src_state, snap.actor.is_member(z.id)).await?;
        issue.team_id = z.id;
        issue.number = number;
        issue.added_to_team_at = now;
        change_state(
            tx,
            z,
            issue,
            &mapped,
            TransitionSource::Update,
            None,
            now,
            false,
        )
        .await?;
        let old_cycle = issue.cycle_id;
        issue.cycle_id = map_cycle(tx, z.id, issue.cycle_id).await?;
        if issue.cycle_id != old_cycle {
            issue.added_to_cycle_at = issue.cycle_id.map(|_| now);
        }
        if let Some(p) = issue.project_id {
            let keep = match tx.issues().get_project(p).await? {
                Some(pl) => pl.team_ids.contains(&z.id),
                None => false,
            };
            if !keep {
                issue.project_id = None;
                issue.project_milestone_id = None;
                issue.added_to_project_at = None;
            }
        }
        issue.estimate = map_estimate(&snap.tree, z, issue.estimate);
        issue.assignee_id = keep_person_after_move(tx, z, issue.assignee_id).await?;
        issue.delegate_id = keep_person_after_move(tx, z, issue.delegate_id).await?;
        let _ = estimation_of(&snap.tree, z);
        labels_available_in(tx, &snap.tree, z.id, old_labels).await
    }

    /// Liest ein Issue vollständig (Relationen, Statusverlauf, frühere Kennungen).
    pub async fn get_issue(&self, ctx: &RequestCtx, reference: &str) -> AppResult<IssueDetail> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let issue = resolve::issue::resolve_in(&mut *tx, &snap, ctx, reference).await?;
        if issue.trashed {
            return Err(resolve::issue::issue_not_found(reference));
        }
        let relations = relations::view(&mut *tx, &snap, ctx, issue.id).await?;
        let state_history = state_spans(&mut *tx, &issue).await?;
        let view = load_view(&mut *tx, issue, true).await?;
        Ok(IssueDetail {
            view,
            relations,
            state_history,
        })
    }

    /// Relationen eines Issues (für Antworten von `save_issue` mit Relationsparametern).
    pub async fn issue_relations(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<RelationsView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let issue = resolve::issue::resolve_in(&mut *tx, &snap, ctx, reference).await?;
        relations::view(&mut *tx, &snap, ctx, issue.id).await
    }

    /// Aktivitätsverlauf eines Issues, älteste zuerst.
    pub async fn issue_history(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<Vec<HistoryEntry>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let issue = resolve::issue::resolve_in(&mut *tx, &snap, ctx, reference).await?;
        let entries = tx.issues().list_history(issue.id, 1000).await?;
        redact_history(&mut *tx, &snap, ctx, entries).await
    }

    /// Issues filtern (docs/07 §3.2 `list_issues`).
    pub async fn list_issues(
        &self,
        ctx: &RequestCtx,
        input: ListIssuesInput,
    ) -> AppResult<IssuePage> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let tx = &mut *tx;
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let visible = snap.visible_team_ids();
        let mut f = IssueListFilter {
            team_ids: visible.iter().copied().collect(),
            include_archived: input.include_archived,
            created_after: input.created_after,
            updated_after: input.updated_after,
            triaged_after: input.triaged_after,
            shared_with: resolve::issue::share_recipient(&snap, ctx),
            ..Default::default()
        };
        let mut scope_team: Option<Team> = None;
        if let Some(r) = &input.team {
            let team = resolve::team::resolve_in(tx, &snap, r, Visibility::Content).await?;
            let mut ids: BTreeSet<TeamId> = BTreeSet::from([team.id]);
            let by_uuid = uuid::Uuid::parse_str(r.trim()).is_ok();
            if input.include_sub_teams || by_uuid {
                ids.extend(snap.tree.descendants(team.id).iter().map(|t| t.id));
            }
            f.team_ids = ids.into_iter().filter(|t| visible.contains(t)).collect();
            f.shared_with = None;
            scope_team = Some(team);
        }
        match &input.assignee {
            Some(None) => f.assignee = Some(None),
            Some(Some(r)) if r.trim().eq_ignore_ascii_case("null") => f.assignee = Some(None),
            Some(Some(r)) => {
                f.assignee = Some(Some(resolve::user::resolve_in(tx, &snap, ctx, r).await?.id));
            }
            None => {}
        }
        if let Some(r) = &input.creator {
            f.creator = Some(resolve::user::resolve_in(tx, &snap, ctx, r).await?.id);
        }
        if let Some(r) = &input.delegate {
            f.delegate = Some(resolve::user::resolve_in(tx, &snap, ctx, r).await?.id);
        }
        if let Some(r) = &input.state {
            apply_state_filter(tx, &f.team_ids.clone(), r, &mut f).await?;
        }
        if let Some(r) = &input.label {
            let found =
                resolve::label::resolve_visible(tx, LabelKind::Issue, &visible, r, None).await?;
            if found.is_empty() {
                return Err(resolve::label::label_not_found(LabelKind::Issue, r));
            }
            let mut ids: Vec<LabelId> = Vec::new();
            for l in found {
                if l.is_group {
                    ids.extend(
                        tx.labels()
                            .children(LabelKind::Issue, l.id)
                            .await?
                            .iter()
                            .map(|c| c.id),
                    );
                } else {
                    ids.push(l.id);
                }
            }
            f.label_ids = Some(ids);
        }
        if let Some(r) = &input.project {
            let mut found: Vec<_> = tx
                .issues()
                .find_projects(r)
                .await?
                .into_iter()
                .filter(|p| p.team_ids.iter().any(|t| visible.contains(t)))
                .collect();
            match found.len() {
                0 => return Err(resolve_not_found("Project", r)),
                1 => f.project_id = Some(found.remove(0).id),
                _ => {
                    return Err(AppError::ambiguous_reference(format!(
                        "Project \"{r}\" matches several projects."
                    ))
                    .with_details(json!({ "reference": r, "type": "Project",
                        "candidates": found.iter().take(10).map(|p| json!({"id": p.id.to_string(), "name": p.name})).collect::<Vec<_>>() })));
                }
            }
        }
        if let Some(r) = &input.cycle {
            let teams: Vec<TeamId> = match &scope_team {
                Some(t) => vec![t.id],
                None => f.team_ids.clone(),
            };
            let mut found = Vec::new();
            if let Ok(u) = uuid::Uuid::parse_str(r.trim()) {
                if let Some(c) = tx
                    .issues()
                    .get_cycle(lineal_domain::ids::CycleId(u))
                    .await?
                    && visible.contains(&c.team_id)
                {
                    found.push(c);
                }
            } else {
                for t in teams {
                    found.extend(tx.issues().find_cycles(t, r).await?);
                }
            }
            match found.len() {
                0 => return Err(resolve_not_found("Cycle", r)),
                1 => f.cycle_id = Some(found.remove(0).id),
                _ => {
                    return Err(AppError::ambiguous_reference(format!(
                        "Cycle \"{r}\" matches several cycles; pass a team."
                    ))
                    .with_details(json!({ "reference": r, "type": "Cycle" })));
                }
            }
        }
        if let Some(r) = &input.parent {
            f.parent_id = Some(resolve::issue::resolve_in(tx, &snap, ctx, r).await?.id);
        }
        if let Some(p) = input.priority {
            f.priority = Some(priority_of(p)?);
        }
        let query = input
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty());
        if input.custom_view.is_some() && query.is_some() {
            return Err(
                AppError::invalid_request("query cannot be combined with customView").with_details(
                    json!({ "field": "query", "reason": "conflicts_with_customView" }),
                ),
            );
        }
        let mut parts: Vec<Value> = Vec::new();
        if let Some(r) = &input.custom_view {
            let v = view::view_for_list(tx, &snap, ctx, r, ViewModel::Issue).await?;
            parts.push(view::combined_filter(&v, Vec::new()));
        }
        let mut extra = None;
        if let Some(fj) = &input.filter {
            extra = Some(view::parse_view_filter(ViewModel::Issue, fj)?);
        }
        if !parts.is_empty() || extra.is_some() {
            let eval =
                view::build_eval(tx, &snap, ctx, self.clock.now(), extra.is_some(), true).await?;
            if let Some((any, norm)) = extra {
                tx.views().verify_filter_refs(&any, &eval).await?;
                parts.push(norm);
            }
            let ast: lineal_domain::rules::filter::IssueFilter =
                lineal_domain::rules::filter::from_json_value(&json!({ "and": parts })).map_err(
                    |e| {
                        AppError::invalid_request(format!("Invalid filter: {}", e.reason))
                            .with_details(json!({ "path": e.path, "reason": e.reason }))
                    },
                )?;
            f.ast = Some(Box::new(ast));
            f.eval = Some(eval);
        }
        if let Some(q) = query {
            f.search = Some(q.to_owned());
        }
        let page = PageRequest {
            limit: input.limit.clamp(1, 250),
            cursor: input.cursor,
            order_by: input.order_by,
        };
        let res = tx.issues().list(&f, &page).await?;
        let items = load_views(tx, res.items).await?;
        Ok(IssuePage {
            items,
            next: res.next,
        })
    }

    /// Legt ein Issue in den Papierkorb (`permanently` = endgültig löschen, nur Admins).
    pub async fn delete_issue(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        permanently: bool,
    ) -> AppResult<Issue> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let found = resolve::issue::resolve_in(&mut *tx, &snap, ctx, reference).await?;
        snap.require_team(ctx, Action::DeleteIssue, found.team_id)?;
        let mut issue = tx
            .issues()
            .get(found.id, true)
            .await?
            .ok_or_else(|| resolve::issue::issue_not_found(reference))?;
        if permanently {
            if !(snap.actor.is_admin() || snap.actor.is_system()) {
                return Err(
                    AppError::forbidden("Only admins can delete issues permanently.")
                        .with_details(json!({ "action": "DeleteIssue", "reason": "role" })),
                );
            }
            tx.issues().delete(issue.id).await?;
            emit(
                &mut *tx,
                ctx,
                DomainEvent::IssueRemoved {
                    issue_id: issue.id,
                    team_id: issue.team_id,
                },
            )
            .await?;
            tx.commit().await?;
            return Ok(issue);
        }
        if issue.trashed {
            return Ok(issue);
        }
        let old = issue.clone();
        issue.trashed = true;
        issue.trashed_at = Some(now);
        issue.archived_at.get_or_insert(now);
        let saved = tx.issues().save(&issue, now).await?;
        let mut d = Delta::default();
        d.flag("trashed");
        history::record(
            &mut *tx,
            self.ids.new_uuid(),
            &saved,
            actor_user_id(ctx),
            HistorySource::User,
            &d,
            now,
        )
        .await?;
        let _ = old;
        // Dokumente des Issues wandern mit in den Papierkorb (docs/06 §1.10).
        self.trash_documents_of(&mut *tx, DocParent::Issue(saved.id))
            .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::IssueRemoved {
                issue_id: saved.id,
                team_id: saved.team_id,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(saved)
    }

    /// Stellt ein Issue aus dem Papierkorb (innerhalb der Aufbewahrungsfrist) oder aus dem
    /// Archiv wieder her.
    pub async fn restore_issue(&self, ctx: &RequestCtx, reference: &str) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let found = resolve::issue::resolve_in(&mut *tx, &snap, ctx, reference).await?;
        snap.require_team(ctx, Action::UpdateIssue, found.team_id)?;
        let mut issue = tx
            .issues()
            .get(found.id, true)
            .await?
            .ok_or_else(|| resolve::issue::issue_not_found(reference))?;
        let was_trashed = issue.trashed;
        if was_trashed {
            let days = snap.org.limits.trash_retention_days;
            if issue
                .trashed_at
                .is_some_and(|t| now - t > Duration::days(days))
            {
                return Err(invalid(
                    "id",
                    "retention_expired",
                    "the retention period for deleted issues has expired",
                ));
            }
        } else if issue.archived_at.is_none() {
            return load_view(&mut *tx, issue, true).await;
        }
        // Archivierten Status auf die erste Status derselben Kategorie abbilden.
        let state = resolve::state::by_id(&mut *tx, issue.state_id).await?;
        if state.archived_at.is_some()
            && let Some(s) = first_state_of(&mut *tx, issue.team_id, state.state_type).await?
        {
            issue.state_id = s.id;
        }
        issue.trashed = false;
        issue.trashed_at = None;
        issue.archived_at = None;
        let saved = tx.issues().save(&issue, now).await?;
        if was_trashed {
            self.restore_documents_of(&mut *tx, DocParent::Issue(saved.id))
                .await?;
        }
        let mut d = Delta::default();
        d.flag(if was_trashed {
            "restored"
        } else {
            "unarchived"
        });
        history::record(
            &mut *tx,
            self.ids.new_uuid(),
            &saved,
            actor_user_id(ctx),
            HistorySource::User,
            &d,
            now,
        )
        .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::IssueUpdated {
                previous: None,
                issue_id: saved.id,
                team_id: saved.team_id,
                changed: vec![if was_trashed { "trashed" } else { "archivedAt" }.into()],
                mentioned_user_ids: vec![],
            },
        )
        .await?;
        let view = load_view(&mut *tx, saved, true).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Teilt ein Issue mit einem User, der es sonst nicht sehen könnte (docs/07 §3.17).
    pub async fn share_issue(
        &self,
        ctx: &RequestCtx,
        issue_ref: &str,
        user_ref: &str,
    ) -> AppResult<ShareResult> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let issue = resolve::issue::resolve_in(&mut *tx, &snap, ctx, issue_ref).await?;
        if issue.trashed {
            return Err(resolve::issue::issue_not_found(issue_ref));
        }
        snap.require_team(ctx, Action::ShareIssue, issue.team_id)?;
        let user = resolve::user::resolve_in(&mut *tx, &snap, ctx, user_ref).await?;
        if fields::user_can_access_team(&mut *tx, &snap.tree, &user, issue.team_id).await? {
            return Ok(ShareResult {
                issue,
                user_id: user.id,
                user_name: user.name,
                shared: false,
                shared_at: None,
                message: Some("User already has access".into()),
            });
        }
        let created = tx
            .issues()
            .insert_share(&IssueShare {
                issue_id: issue.id,
                user_id: user.id,
                shared_by_id: actor_user_id(ctx),
                created_at: now,
            })
            .await?;
        let share = tx
            .issues()
            .get_share(issue.id, user.id)
            .await?
            .ok_or_else(|| AppError::internal("share vanished"))?;
        if created {
            emit(
                &mut *tx,
                ctx,
                DomainEvent::IssueUpdated {
                    previous: None,
                    issue_id: issue.id,
                    team_id: issue.team_id,
                    changed: vec!["sharedWith".into()],
                    mentioned_user_ids: vec![],
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(ShareResult {
            issue,
            user_id: user.id,
            user_name: user.name,
            shared: true,
            shared_at: Some(share.created_at),
            message: None,
        })
    }

    /// Nimmt den geteilten Zugriff zurück (idempotent).
    pub async fn unshare_issue(
        &self,
        ctx: &RequestCtx,
        issue_ref: &str,
        user_ref: &str,
    ) -> AppResult<ShareResult> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let issue = resolve::issue::resolve_in(&mut *tx, &snap, ctx, issue_ref).await?;
        snap.require_team(ctx, Action::ShareIssue, issue.team_id)?;
        let user = resolve::user::resolve_in(&mut *tx, &snap, ctx, user_ref).await?;
        if tx.issues().delete_share(issue.id, user.id).await? {
            emit(
                &mut *tx,
                ctx,
                DomainEvent::IssueUpdated {
                    previous: None,
                    issue_id: issue.id,
                    team_id: issue.team_id,
                    changed: vec!["sharedWith".into()],
                    mentioned_user_ids: vec![],
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(ShareResult {
            issue,
            user_id: user.id,
            user_name: user.name,
            shared: false,
            shared_at: None,
            message: None,
        })
    }
}

/// Felder eines History-Eintrags, deren Werte auf andere Objekte zeigen.
const HISTORY_REF_FIELDS: [&str; 5] = [
    "teamId",
    "parentId",
    "projectId",
    "cycleId",
    "projectMilestoneId",
];

/// Ist das Objekt, auf das `value` im Feld `field` zeigt, für den Aufrufer sichtbar?
/// Nicht auflösbare Werte gelten als unsichtbar.
async fn history_ref_visible(
    tx: &mut dyn Tx,
    visible: &BTreeSet<TeamId>,
    field: &str,
    value: &str,
) -> AppResult<bool> {
    let Ok(uuid) = uuid::Uuid::parse_str(value) else {
        return Ok(false);
    };
    let any_visible = |teams: &[TeamId]| teams.iter().any(|t| visible.contains(t));
    Ok(match field {
        "teamId" => visible.contains(&TeamId(uuid)),
        "parentId" => tx
            .issues()
            .get(IssueId(uuid), false)
            .await?
            .is_some_and(|i| visible.contains(&i.team_id)),
        "cycleId" => tx
            .issues()
            .get_cycle(lineal_domain::ids::CycleId(uuid))
            .await?
            .is_some_and(|c| visible.contains(&c.team_id)),
        "projectId" => tx
            .issues()
            .get_project(lineal_domain::ids::ProjectId(uuid))
            .await?
            .is_some_and(|p| any_visible(&p.team_ids)),
        "projectMilestoneId" => {
            match tx
                .projects()
                .get_milestone(lineal_domain::ids::MilestoneId(uuid))
                .await?
            {
                Some(m) => tx
                    .issues()
                    .get_project(m.project_id)
                    .await?
                    .is_some_and(|p| any_visible(&p.team_ids)),
                None => false,
            }
        }
        _ => true,
    })
}

/// Entfernt aus der Historie alles, was auf unsichtbare Objekte zeigt: Relationsänderungen
/// zu Issues, die der Aufrufer nicht sehen darf, entfallen; Verweise (`teamId`, `parentId`,
/// `projectId`, `cycleId`, `projectMilestoneId`) auf unsichtbare Ziele werden `null`.
async fn redact_history(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    entries: Vec<HistoryEntry>,
) -> AppResult<Vec<HistoryEntry>> {
    let visible = snap.visible_team_ids();
    let mut cache: HashMap<(&'static str, String), bool> = HashMap::new();
    let mut out = Vec::with_capacity(entries.len());
    for mut e in entries {
        if let Some(Value::Array(items)) = e.relation_changes.take() {
            let mut kept = Vec::new();
            for item in items {
                let identifier = item.get("identifier").and_then(Value::as_str).unwrap_or("");
                match resolve::issue::resolve_in(tx, snap, ctx, identifier).await {
                    Ok(_) => kept.push(item),
                    Err(err) if err.kind == crate::error::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
            }
            e.relation_changes = (!kept.is_empty()).then_some(Value::Array(kept));
        }
        if let Value::Object(changes) = &mut e.changes {
            for field in HISTORY_REF_FIELDS {
                let Some(Value::Object(sides)) = changes.get_mut(field) else {
                    continue;
                };
                for side in ["from", "to"] {
                    let Some(v) = sides.get_mut(side) else {
                        continue;
                    };
                    let Some(text) = v.as_str().map(str::to_owned) else {
                        continue;
                    };
                    let key = (field, text);
                    let ok = match cache.get(&key) {
                        Some(ok) => *ok,
                        None => {
                            let ok = history_ref_visible(tx, &visible, field, &key.1).await?;
                            cache.insert(key, ok);
                            ok
                        }
                    };
                    if !ok {
                        *v = Value::Null;
                    }
                }
            }
        }
        out.push(e);
    }
    Ok(out)
}

fn resolve_not_found(kind: &str, reference: &str) -> AppError {
    AppError::not_found(format!("Could not find referenced {kind}."))
        .with_details(json!({ "reference": reference, "type": kind }))
}

/// `state`-Filter: Kategorie, UUID oder Statusname (über alle sichtbaren Teams).
async fn apply_state_filter(
    tx: &mut dyn Tx,
    teams: &[TeamId],
    reference: &str,
    f: &mut IssueListFilter,
) -> AppResult<()> {
    let r = reference.trim();
    if let Ok(kind) = r.parse::<StateType>() {
        f.state_types = Some(vec![kind]);
        return Ok(());
    }
    if let Ok(u) = uuid::Uuid::parse_str(r) {
        match tx.states().get(StateId(u)).await? {
            Some(s) if teams.contains(&s.team_id) => {
                f.state_ids = Some(vec![s.id]);
                return Ok(());
            }
            _ => return Err(resolve_not_found("WorkflowState", reference)),
        }
    }
    let mut ids = Vec::new();
    for t in teams {
        if let Some(s) = tx.states().find_by_name(*t, r).await? {
            ids.push(s.id);
        }
    }
    if ids.is_empty() {
        return Err(resolve_not_found("WorkflowState", reference));
    }
    f.state_ids = Some(ids);
    Ok(())
}

/// Verweildauer je Status aus der History (`stateHistory`).
async fn state_spans(tx: &mut dyn Tx, issue: &Issue) -> AppResult<Vec<StateSpan>> {
    let entries = tx.issues().list_history(issue.id, 1000).await?;
    let parse = |v: &Value| -> Option<StateId> {
        v.as_str()
            .and_then(|s| s.parse::<uuid::Uuid>().ok())
            .map(StateId)
    };
    let mut changes: Vec<(DateTime<Utc>, StateId, StateId)> = Vec::new();
    for e in &entries {
        if let Some(c) = e.changes.get("stateId")
            && let (Some(from), Some(to)) =
                (c.get("from").and_then(parse), c.get("to").and_then(parse))
        {
            changes.push((e.created_at, from, to));
        }
    }
    let initial = changes.first().map_or(issue.state_id, |c| c.1);
    let mut spans: Vec<(StateId, DateTime<Utc>, Option<DateTime<Utc>>)> =
        vec![(initial, issue.created_at, None)];
    for (at, _from, to) in &changes {
        if let Some(last) = spans.last_mut() {
            last.2 = Some(*at);
        }
        spans.push((*to, *at, None));
    }
    let mut out = Vec::new();
    for (id, start, end) in spans {
        let state = resolve::state::by_id(tx, id).await?;
        out.push(StateSpan {
            state_id: id,
            name: state.name,
            state_type: state.state_type,
            started_at: start,
            ended_at: end,
        });
    }
    Ok(out)
}
