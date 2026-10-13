//! Feldregeln für Issues (docs/03 §2.3): Status, Schätzung, Zuweisung, Zuordnung zu Cycle,
//! Projekt, Meilenstein, Parent und Labels. Alle Funktionen arbeiten innerhalb der Transaktion
//! des Aufrufers und verändern nur die übergebene Struktur; gespeichert wird später.

use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{ActorView, PolicySnapshot, TeamTree, compute_visible_team_ids};
use crate::ports::Tx;
use crate::ports::repos::issue::{CycleLookup, Issue, MilestoneLookup, ProjectLookup};
use crate::ports::repos::label::{Label, LabelKind};
use crate::ports::repos::state::WorkflowState;
use crate::ports::repos::team::Team;
use crate::ports::repos::user::User;
use crate::resolve;
use chrono::{DateTime, Utc};
use lineal_domain::DomainError;
use lineal_domain::enums::{EstimationType, LabelGroupType, StateType};
use lineal_domain::ids::{CycleId, LabelId, ProjectId, TeamId};
use lineal_domain::rules::estimate::{self, EstimateInput};
use lineal_domain::rules::labels::{self, LabelGroup};
use lineal_domain::rules::sort_order::{self, SortOrderOnStateChange};
use lineal_domain::rules::state::{
    IssueTimestamps, TransitionSource, apply_transition, check_transition,
};
use serde_json::json;
use std::collections::BTreeSet;

/// Höchste Verschachtelungstiefe von Sub-Issues (docs/03 §3.1).
pub const MAX_SUB_ISSUE_DEPTH: i64 = 10;

/// Titel: 1–255 Zeichen nach Trimmen, keine Zeilenumbrüche.
pub fn validate_title(raw: &str) -> AppResult<String> {
    let t = raw.trim();
    let err = |reason: &str| {
        AppError::invalid_request(format!("invalid title: {reason}"))
            .with_details(json!({ "field": "title", "reason": reason }))
    };
    if t.is_empty() {
        return Err(err("must not be empty"));
    }
    if t.chars().count() > 255 {
        return Err(err("must be at most 255 characters"));
    }
    if t.contains(['\n', '\r']) {
        return Err(err("must not contain line breaks"));
    }
    Ok(t.to_owned())
}

/// Beschreibung: höchstens 500 000 Zeichen; leer = keine Beschreibung.
pub fn validate_description(raw: String) -> AppResult<Option<String>> {
    if raw.chars().count() > lineal_domain::rules::patch::MAX_TEXT_CHARS {
        return Err(
            AppError::limit_exceeded("description exceeds 500000 characters")
                .with_details(json!({ "field": "description" })),
        );
    }
    // Bereinigung beim Speichern (docs/15 §4.3), auch für API/MCP: sonst schreibt der Editor die
    // Beschreibung beim bloßen Öffnen um.
    let clean = lineal_domain::rules::markdown::sanitize(&raw);
    Ok((!clean.is_empty()).then_some(clean))
}

/// Schätzskala des Teams (inkl. Vererbung von Parent-Teams).
pub fn estimation_of(tree: &TeamTree, team: &Team) -> (EstimationType, bool, bool) {
    let mut cur = team;
    let mut hops = 0;
    while cur.inherit_issue_estimation && hops < 16 {
        match cur.parent_id.and_then(|p| tree.get(p)) {
            Some(p) => cur = p,
            None => break,
        }
        hops += 1;
    }
    (
        cur.issue_estimation_type,
        cur.issue_estimation_extended,
        cur.issue_estimation_allow_zero,
    )
}

/// Prüft eine Schätzung gegen die Skala des Teams und liefert den gespeicherten Wert.
pub fn convert_estimate(tree: &TeamTree, team: &Team, value: f64) -> AppResult<i32> {
    let (ty, extended, zero) = estimation_of(tree, team);
    match estimate::convert(ty, extended, zero, &EstimateInput::Number(value)) {
        Ok(v) => Ok(v as i32),
        Err(DomainError::Validation { reason, .. }) => {
            let allowed = estimate::allowed_values(ty, extended, zero);
            Err(
                AppError::invalid_request(format!("invalid estimate: {reason}"))
                    .with_details(json!({ "field": "estimate", "allowed": allowed })),
            )
        }
    }
}

/// `mapEstimate` beim Teamwechsel (docs/02 §7.2 Schritt 11).
pub fn map_estimate(tree: &TeamTree, target: &Team, value: Option<i32>) -> Option<i32> {
    let v = value?;
    let (ty, extended, zero) = estimation_of(tree, target);
    if ty == EstimationType::NotUsed {
        return None;
    }
    let allowed = estimate::allowed_values(ty, extended, zero);
    allowed
        .iter()
        .any(|a| (*a - f64::from(v)).abs() < f64::EPSILON)
        .then_some(v)
}

fn transition_error(e: DomainError) -> AppError {
    let DomainError::Validation { field, reason } = e;
    AppError::invalid_request(format!("invalid {field}: {reason}"))
        .with_details(json!({ "field": field, "reason": reason }))
}

/// Statuswechsel mit Zeitstempeln und `sortOrder` (docs/03 §1.6, §12). `explicit_sort` hat Vorrang.
///
/// Zusätzlich zur Kernlogik ([`change_state_core`]) laufen hier die Automationen von WP12:
/// Auto-Cycle (docs/04 §1.6), Parent-/Child-Auto-Close (docs/03 §3.4) und die Triage-
/// Verantwortung beim Eintritt in Triage (docs/03 §10.6). Automatisch geänderte Issues werden
/// in der Transaktion des Aufrufers gespeichert; `issue` selbst speichert der Aufrufer.
#[allow(clippy::too_many_arguments)]
pub async fn change_state(
    tx: &mut dyn Tx,
    team: &Team,
    issue: &mut Issue,
    new: &WorkflowState,
    source: TransitionSource,
    explicit_sort: Option<f64>,
    now: DateTime<Utc>,
    check: bool,
) -> AppResult<()> {
    if new.id == issue.state_id {
        return Ok(());
    }
    let old = issue.state_type;
    change_state_core(tx, team, issue, new, source, explicit_sort, now, check).await?;
    crate::usecases::triage::automation::after_state_change(tx, team, issue, old, now).await
}

/// Kern des Statuswechsels ohne Parent-/Child- und Triage-Automationen (Zeitstempel,
/// `sortOrder`, Auto-Cycle). Wird auch für automatisch geänderte Issues benutzt.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn change_state_core(
    tx: &mut dyn Tx,
    team: &Team,
    issue: &mut Issue,
    new: &WorkflowState,
    source: TransitionSource,
    explicit_sort: Option<f64>,
    now: DateTime<Utc>,
    check: bool,
) -> AppResult<()> {
    if new.id == issue.state_id {
        return Ok(());
    }
    if new.team_id != issue.team_id {
        return Err(
            AppError::invalid_request("state does not belong to the issue's team")
                .with_details(json!({ "field": "state", "reason": "state_not_in_team" })),
        );
    }
    if new.archived_at.is_some() {
        return Err(AppError::invalid_request("state is archived")
            .with_details(json!({ "field": "state", "reason": "state_archived" })));
    }
    let old = issue.state_type;
    if check {
        check_transition(
            old,
            new.state_type,
            source,
            team.require_priority_to_leave_triage,
            issue.priority,
        )
        .map_err(transition_error)?;
    }
    let ts = apply_transition(
        old,
        new.state_type,
        now,
        IssueTimestamps {
            started_triage_at: issue.started_triage_at,
            triaged_at: issue.triaged_at,
            started_at: issue.started_at,
            completed_at: issue.completed_at,
            canceled_at: issue.canceled_at,
            auto_closed_at: issue.auto_closed_at,
            snoozed_until_at: issue.snoozed_until_at,
            snoozed_by: issue.snoozed_by_id,
        },
    );
    issue.started_triage_at = ts.started_triage_at;
    issue.triaged_at = ts.triaged_at;
    issue.started_at = ts.started_at;
    issue.completed_at = ts.completed_at;
    issue.canceled_at = ts.canceled_at;
    issue.auto_closed_at = ts.auto_closed_at;
    issue.snoozed_until_at = ts.snoozed_until_at;
    issue.snoozed_by_id = ts.snoozed_by;
    let setting: SortOrderOnStateChange = team
        .set_issue_sort_order_on_state_change
        .parse()
        .map_err(transition_error)?;
    let (min, max) = tx.issues().sort_extent(team.id, new.id).await?;
    issue.sort_order =
        sort_order::on_state_change(setting, explicit_sort, issue.sort_order, min, max);
    issue.state_id = new.id;
    issue.state_type = new.state_type;
    issue.state_entered_at = now;
    // Auto-Cycle (docs/04 §1.6): nur Issues ohne Cycle.
    if issue.cycle_id.is_none()
        && let Some(c) =
            crate::usecases::cycle::auto_cycle_for_state(tx, team, new.state_type, now).await?
    {
        issue.cycle_id = Some(c.id);
        issue.added_to_cycle_at = Some(now);
    }
    Ok(())
}

/// Standardstatus für neue Issues eines Teams (die eigene Kopie bei vererbtem Workflow).
pub async fn default_state(tx: &mut dyn Tx, team: &Team) -> AppResult<WorkflowState> {
    if let Some(id) = team.default_issue_state_id
        && let Some(s) = tx.states().get(id).await?
        && s.archived_at.is_none()
        && s.team_id == team.id
    {
        return Ok(s);
    }
    let states = tx.states().list_for_team(team.id, false).await?;
    states
        .iter()
        .find(|s| s.state_type == StateType::Backlog)
        .or_else(|| states.iter().find(|s| s.state_type == StateType::Unstarted))
        .cloned()
        .ok_or_else(|| {
            AppError::invalid_request("the team has no state that can hold new issues")
                .with_details(json!({ "field": "state", "reason": "no_default_state" }))
        })
}

/// Triage-Status des Teams, falls vorhanden.
pub async fn triage_state(tx: &mut dyn Tx, team: &Team) -> AppResult<Option<WorkflowState>> {
    if let Some(id) = team.triage_issue_state_id
        && let Some(s) = tx.states().get(id).await?
        && s.archived_at.is_none()
        && s.team_id == team.id
    {
        return Ok(Some(s));
    }
    let states = tx.states().list_for_team(team.id, false).await?;
    Ok(states
        .into_iter()
        .find(|s| s.state_type == StateType::Triage))
}

/// Erster aktiver Status einer Kategorie.
pub async fn first_state_of(
    tx: &mut dyn Tx,
    team: TeamId,
    kind: StateType,
) -> AppResult<Option<WorkflowState>> {
    Ok(tx
        .states()
        .list_for_team(team, false)
        .await?
        .into_iter()
        .find(|s| s.state_type == kind))
}

/// `mapState` beim Teamwechsel (docs/02 §7.2 Schritt 7).
pub async fn map_state(
    tx: &mut dyn Tx,
    target: &Team,
    src: &WorkflowState,
    actor_is_member_of_target: bool,
) -> AppResult<WorkflowState> {
    let open = matches!(
        src.state_type,
        StateType::Triage | StateType::Backlog | StateType::Unstarted | StateType::Started
    );
    if src.state_type == StateType::Duplicate
        && let Some(s) = first_state_of(tx, target.id, StateType::Duplicate).await?
    {
        return Ok(s);
    }
    if target.triage_enabled
        && !actor_is_member_of_target
        && open
        && let Some(s) = triage_state(tx, target).await?
    {
        return Ok(s);
    }
    if src.state_type == StateType::Triage && !target.triage_enabled {
        return default_state(tx, target).await;
    }
    let states = tx.states().list_for_team(target.id, false).await?;
    let root = src.inherited_from_id.unwrap_or(src.id);
    if let Some(s) = states
        .iter()
        .find(|s| s.id == root || s.inherited_from_id == Some(root))
    {
        return Ok(s.clone());
    }
    if let Some(s) = states
        .iter()
        .find(|s| s.state_type == src.state_type && s.name.eq_ignore_ascii_case(&src.name))
    {
        return Ok(s.clone());
    }
    states
        .into_iter()
        .find(|s| s.state_type == src.state_type)
        .ok_or_else(|| AppError::internal("target team has no state of the required category"))
}

fn no_access(reason: &str) -> AppError {
    AppError::invalid_request("the user has no access to this team")
        .with_details(json!({ "field": "assignee", "reason": reason }))
}

/// Ob der User das Team sehen darf.
pub async fn user_can_access_team(
    tx: &mut dyn Tx,
    tree: &TeamTree,
    user: &User,
    team: TeamId,
) -> AppResult<bool> {
    let memberships = tx.teams().list_memberships_of_user(user.id).await?;
    let view = ActorView::new(
        Actor::User {
            id: user.id,
            role: user.role,
        },
        &memberships,
    );
    Ok(compute_visible_team_ids(&view, tree).contains(&team))
}

/// Assignee: aktiver User mit Zugriff auf das Team (docs/03 §2.7).
pub async fn resolve_assignee(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    team: &Team,
    reference: &str,
) -> AppResult<User> {
    let user = resolve::user::resolve_in(tx, snap, ctx, reference).await?;
    if !user.active {
        return Err(no_access("assignee_suspended"));
    }
    if !user_can_access_team(tx, &snap.tree, &user, team.id).await? {
        return Err(no_access("assignee_has_no_access"));
    }
    Ok(user)
}

/// Delegate: nur User mit Rolle `app`.
pub async fn resolve_delegate(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    team: &Team,
    reference: &str,
) -> AppResult<User> {
    let user = resolve::user::resolve_in(tx, snap, ctx, reference).await?;
    if !user.active || !user_can_access_team(tx, &snap.tree, &user, team.id).await? {
        return Err(no_access("delegate_has_no_access"));
    }
    if user.role != lineal_domain::enums::UserRole::App {
        return Err(AppError::invalid_request("only app users can be delegates")
            .with_details(json!({ "field": "delegate", "reason": "delegate_must_be_app" })));
    }
    Ok(user)
}

fn ambiguous(kind: &str, reference: &str, candidates: Vec<serde_json::Value>) -> AppError {
    AppError::ambiguous_reference(format!("{kind} \"{reference}\" matches several objects."))
        .with_details(json!({ "reference": reference, "type": kind, "candidates": candidates }))
}

fn not_found(kind: &str, reference: &str) -> AppError {
    AppError::not_found(format!("Could not find referenced {kind}."))
        .with_details(json!({ "reference": reference, "type": kind }))
}

/// Cycle des Teams (Name, Nummer oder UUID); nicht abgeschlossen (docs/03 §2.3).
pub async fn resolve_cycle(
    tx: &mut dyn Tx,
    team: TeamId,
    reference: &str,
) -> AppResult<CycleLookup> {
    let mut found = tx.issues().find_cycles(team, reference).await?;
    match found.len() {
        0 => Err(not_found("Cycle", reference)),
        1 => {
            let c = found.remove(0);
            if c.completed_at.is_some() {
                return Err(AppError::invalid_request("the cycle is completed")
                    .with_details(json!({ "field": "cycle", "reason": "cycle_completed" })));
            }
            Ok(c)
        }
        _ => Err(ambiguous(
            "Cycle",
            reference,
            found
                .iter()
                .map(|c| json!({ "id": c.id.to_string(), "number": c.number, "name": c.name }))
                .collect(),
        )),
    }
}

/// Projekt (UUID, `P-KEY-n`, Slug, Name); sichtbar, wenn mindestens eines seiner Teams sichtbar ist,
/// und das Team des Issues muss daran beteiligt sein.
pub async fn resolve_project(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    team: TeamId,
    reference: &str,
) -> AppResult<ProjectLookup> {
    let visible = snap.visible_team_ids();
    let mut found: Vec<ProjectLookup> = tx
        .issues()
        .find_projects(reference)
        .await?
        .into_iter()
        .filter(|p| p.team_ids.iter().any(|t| visible.contains(t)))
        .collect();
    match found.len() {
        0 => Err(not_found("Project", reference)),
        1 => {
            let p = found.remove(0);
            if !p.team_ids.contains(&team) {
                return Err(AppError::invalid_request(
                    "the issue's team does not take part in this project",
                )
                .with_details(json!({ "field": "project", "reason": "team_not_in_project" })));
            }
            Ok(p)
        }
        _ => Err(ambiguous(
            "Project",
            reference,
            found
                .iter()
                .map(|p| json!({ "id": p.id.to_string(), "name": p.name, "identifier": p.identifier }))
                .collect(),
        )),
    }
}

/// Meilenstein im Projekt.
pub async fn resolve_milestone(
    tx: &mut dyn Tx,
    project: ProjectId,
    reference: &str,
) -> AppResult<MilestoneLookup> {
    let mut found = tx.issues().find_milestones(project, reference).await?;
    match found.len() {
        0 => Err(not_found("ProjectMilestone", reference)),
        1 => Ok(found.remove(0)),
        _ => Err(ambiguous(
            "ProjectMilestone",
            reference,
            found
                .iter()
                .map(|m| json!({ "id": m.id.to_string(), "name": m.name }))
                .collect(),
        )),
    }
}

/// Wirksame Zuordnung nach einem Update: Projekt, Meilenstein, Cycle.
pub struct PlanningChange {
    pub project: Option<Option<String>>,
    pub milestone: Option<Option<String>>,
    pub cycle: Option<Option<String>>,
}

/// Wendet Cycle-, Projekt- und Meilenstein-Zuordnung an und prüft die Invarianten
/// (docs/01 Issue-Invarianten 2–4): Cycle gehört zum Team, das Team nimmt am Projekt teil,
/// der Meilenstein gehört zum Projekt. Liefert `true`, wenn der Status wegen eines aktiven
/// Cycles von `backlog` auf `unstarted` gewechselt ist (`state_set_explicitly = false`).
#[allow(clippy::too_many_arguments)]
pub async fn apply_planning(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    team: &Team,
    issue: &mut Issue,
    change: &PlanningChange,
    state_set_explicitly: bool,
    now: DateTime<Utc>,
) -> AppResult<()> {
    // Projekt
    let old_project = issue.project_id;
    match &change.project {
        Some(None) => {
            issue.project_id = None;
        }
        Some(Some(r)) => {
            snap.require_team(ctx, crate::policy::Action::ManageProject, team.id)?;
            let p = resolve_project(tx, snap, team.id, r).await?;
            issue.project_id = Some(p.id);
        }
        None => {}
    }
    if issue.project_id != old_project {
        issue.added_to_project_at = issue.project_id.map(|_| now);
        // Meilenstein gehört zum alten Projekt.
        issue.project_milestone_id = None;
    }
    // Meilenstein
    match &change.milestone {
        Some(None) => issue.project_milestone_id = None,
        Some(Some(r)) => {
            let Some(project) = issue.project_id else {
                return Err(AppError::invalid_request("a milestone requires a project")
                    .with_details(
                        json!({ "field": "milestone", "reason": "milestone_requires_project" }),
                    ));
            };
            issue.project_milestone_id = Some(resolve_milestone(tx, project, r).await?.id);
        }
        None => {}
    }
    // Cycle
    let old_cycle = issue.cycle_id;
    match &change.cycle {
        Some(None) => issue.cycle_id = None,
        Some(Some(r)) => {
            let c = resolve_cycle(tx, team.id, r).await?;
            issue.cycle_id = Some(c.id);
            let active = c.starts_at <= now && now < c.ends_at;
            if active
                && !state_set_explicitly
                && issue.state_type == StateType::Backlog
                && let Some(s) = first_state_of(tx, team.id, StateType::Unstarted).await?
            {
                change_state(
                    tx,
                    team,
                    issue,
                    &s,
                    TransitionSource::Update,
                    None,
                    now,
                    false,
                )
                .await?;
            }
        }
        None => {}
    }
    if issue.cycle_id != old_cycle {
        issue.added_to_cycle_at = issue.cycle_id.map(|_| now);
    }
    Ok(())
}

/// Scope der Advisory-Sperre für Änderungen am Issue-Graphen (Parent-Beziehungen, Auto-Close).
pub const ISSUE_GRAPH_LOCK: &str = "issue_graph";

/// Parent setzen: kein Zyklus, nicht sich selbst, Zugriff auf den Parent, Tiefe ≤ 10 (docs/03 §3.1).
pub async fn resolve_parent(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    issue: &Issue,
    reference: &str,
) -> AppResult<Issue> {
    // Zyklus- und Tiefenprüfung lesen die Ahnenkette: Parent-Änderungen im Workspace seriell
    // ausführen (sonst: A unter B und B unter A gleichzeitig, Tiefe 10 umgehbar).
    tx.advisory_lock(ISSUE_GRAPH_LOCK, None).await?;
    let parent = resolve::issue::resolve_in(tx, snap, ctx, reference).await?;
    if parent.trashed {
        return Err(resolve::issue::issue_not_found(reference));
    }
    // Die Zuordnung verändert auch den Parent (Children, Fortschritt): Schreibrecht nötig.
    snap.require_team(ctx, crate::policy::Action::UpdateIssue, parent.team_id)?;
    let bad = |reason: &str, msg: &str| {
        AppError::invalid_request(msg.to_owned())
            .with_details(json!({ "field": "parentId", "reason": reason }))
    };
    if parent.id == issue.id {
        return Err(bad("parent_cycle", "an issue cannot be its own parent"));
    }
    let ancestors = tx.issues().ancestor_ids(parent.id).await?;
    if ancestors.contains(&issue.id) {
        return Err(bad(
            "parent_cycle",
            "the parent is a descendant of this issue",
        ));
    }
    let depth = ancestors.len() as i64 + 2 + tx.issues().descendant_depth(issue.id).await?;
    if depth > MAX_SUB_ISSUE_DEPTH {
        return Err(AppError::limit_exceeded("sub-issue nesting is too deep")
            .with_details(json!({ "field": "parentId", "limit": MAX_SUB_ISSUE_DEPTH })));
    }
    Ok(parent)
}

/// Ergebnis der Label-Berechnung.
#[derive(Debug, Default)]
pub struct LabelPlan {
    pub added: Vec<(LabelId, Option<LabelId>)>,
    pub removed: Vec<LabelId>,
}

async fn groups_for(tx: &mut dyn Tx, set: &[Label]) -> AppResult<Vec<LabelGroup>> {
    let mut seen: BTreeSet<LabelId> = BTreeSet::new();
    let mut groups = Vec::new();
    for l in set {
        let Some(gid) = l.parent_id else { continue };
        if !seen.insert(gid) {
            continue;
        }
        let Some(g) = tx.labels().get(LabelKind::Issue, gid).await? else {
            continue;
        };
        let children = tx
            .labels()
            .children(LabelKind::Issue, gid)
            .await?
            .into_iter()
            .map(|c| c.id)
            .collect();
        groups.push(LabelGroup {
            id: g.id,
            group_type: g.effective_group_type(),
            children,
        });
    }
    Ok(groups)
}

fn label_error(e: DomainError, reason: &str) -> AppError {
    let DomainError::Validation { reason: text, .. } = e;
    AppError::invalid_request(format!("invalid labels: {text}"))
        .with_details(json!({ "field": "labels", "reason": reason }))
}

/// Berechnet die Label-Änderung eines Issues (docs/03 §5.5).
///
/// * `replace` (ersetzend) und `add`/`remove` (inkrementell) schließen sich aus (Aufrufer prüft),
/// * jedes neue Label muss im Team verfügbar, keine Gruppe und nicht retired sein,
/// * singleSelect-Gruppen: ersetzend → Fehler bei mehr als einem Kind, `add` → ersetzt Geschwister.
///
/// `baseline` ist die Menge vor der Änderung (beim Teamwechsel bereits um nicht verfügbare Labels
/// bereinigt), `current` die tatsächlich gespeicherte (für „Retired bleibt erhalten“).
#[allow(clippy::too_many_arguments)]
pub async fn plan_labels(
    tx: &mut dyn Tx,
    tree: &TeamTree,
    team: TeamId,
    current: &[LabelId],
    baseline: &[LabelId],
    replace: Option<&[String]>,
    add: &[String],
    remove: &[String],
) -> AppResult<LabelPlan> {
    // Ziel-Menge bestimmen.
    let mut resolved_new: Vec<Label> = Vec::new();
    let mut removed_refs: Vec<LabelId> = Vec::new();
    for r in remove {
        let scopes = resolve::label::available_scopes(tree, team);
        let found: Vec<_> = resolve::label::candidates(tx, LabelKind::Issue, r)
            .await?
            .into_iter()
            .filter(|l| l.team_id.is_none_or(|t| scopes.contains(&t)))
            .collect();
        if found.is_empty() {
            return Err(resolve::label::label_not_found(LabelKind::Issue, r));
        }
        removed_refs.extend(found.iter().map(|l| l.id));
    }
    let mut target_refs: Vec<&String> = Vec::new();
    match replace {
        Some(list) => target_refs.extend(list.iter()),
        None => target_refs.extend(add.iter()),
    }
    for r in target_refs {
        resolved_new.push(resolve::label::resolve_for_team(tx, tree, team, r).await?);
    }
    for l in &resolved_new {
        if l.is_group {
            return Err(
                AppError::invalid_request("label groups cannot be applied to issues").with_details(
                    json!({ "field": "labels", "reason": "label_is_group", "label": l.name }),
                ),
            );
        }
        if l.retired_at.is_some() && !current.contains(&l.id) {
            return Err(
                AppError::invalid_request("retired labels cannot be applied").with_details(
                    json!({ "field": "labels", "reason": "label_retired", "label": l.name }),
                ),
            );
        }
    }
    if resolved_new.iter().any(|l| removed_refs.contains(&l.id)) {
        return Err(AppError::invalid_request(
            "a label cannot be added and removed in the same request",
        )
        .with_details(json!({ "field": "labels", "reason": "label_added_and_removed" })));
    }

    let final_set: Vec<LabelId> = match replace {
        Some(_) => {
            let selected: Vec<LabelId> = resolved_new.iter().map(|l| l.id).collect();
            let groups = groups_for(tx, &resolved_new).await?;
            labels::check_group_constraints(&selected, &groups, None)
                .map_err(|e| label_error(e, "single_select_violation"))?
        }
        None => {
            let to_add: Vec<LabelId> = resolved_new.iter().map(|l| l.id).collect();
            let kept: Vec<LabelId> = baseline
                .iter()
                .copied()
                .filter(|l| !removed_refs.contains(l))
                .collect();
            // Gruppen aller beteiligten Labels (auch der bestehenden).
            let mut all_ids = kept.clone();
            all_ids.extend(to_add.iter().copied());
            let all = tx.labels().get_many(LabelKind::Issue, &all_ids).await?;
            let groups = groups_for(tx, &all).await?;
            labels::add_labels(&kept, &to_add, &groups)
                .map_err(|e| label_error(e, "single_select_violation"))?
        }
    };
    plan_from_sets(tx, current, &final_set, &resolved_new).await
}

async fn plan_from_sets(
    tx: &mut dyn Tx,
    current: &[LabelId],
    final_set: &[LabelId],
    resolved_new: &[Label],
) -> AppResult<LabelPlan> {
    let removed: Vec<LabelId> = current
        .iter()
        .copied()
        .filter(|l| !final_set.contains(l))
        .collect();
    let mut added = Vec::new();
    for id in final_set.iter().filter(|l| !current.contains(l)) {
        let label = match resolved_new.iter().find(|l| l.id == *id) {
            Some(l) => l.clone(),
            None => tx
                .labels()
                .get(LabelKind::Issue, *id)
                .await?
                .ok_or_else(|| AppError::internal("label vanished"))?,
        };
        added.push((*id, single_select_group(tx, &label).await?));
    }
    Ok(LabelPlan { added, removed })
}

/// Gruppen-ID, wenn die Gruppe des Labels `singleSelect` ist.
async fn single_select_group(tx: &mut dyn Tx, label: &Label) -> AppResult<Option<LabelId>> {
    let Some(gid) = label.parent_id else {
        return Ok(None);
    };
    let Some(g) = tx.labels().get(LabelKind::Issue, gid).await? else {
        return Ok(None);
    };
    Ok((g.effective_group_type() == LabelGroupType::SingleSelect).then_some(g.id))
}

/// Teamwechsel: nur die im Zielteam verfügbaren Labels bleiben (docs/02 §7.2 Schritt 9).
pub async fn labels_available_in(
    tx: &mut dyn Tx,
    tree: &TeamTree,
    target: TeamId,
    current: &[LabelId],
) -> AppResult<Vec<LabelId>> {
    let scopes = resolve::label::available_scopes(tree, target);
    let all = tx.labels().get_many(LabelKind::Issue, current).await?;
    Ok(current
        .iter()
        .copied()
        .filter(|id| {
            all.iter()
                .find(|l| l.id == *id)
                .is_some_and(|l| l.team_id.is_none_or(|t| scopes.contains(&t)))
        })
        .collect())
}

/// Ob ein Issue-Cycle im Zielteam eine Entsprechung hat (docs/02 §7.2 Schritt 8).
pub async fn map_cycle(
    tx: &mut dyn Tx,
    target: TeamId,
    cycle: Option<CycleId>,
) -> AppResult<Option<CycleId>> {
    let Some(c) = cycle else { return Ok(None) };
    let Some(src) = tx.issues().get_cycle(c).await? else {
        return Ok(None);
    };
    let root = src.inherited_from_id.unwrap_or(src.id);
    Ok(tx
        .issues()
        .find_cycle_inheriting(target, root)
        .await?
        .map(|c| c.id))
}

/// Kennzeichen für Referenzen auf Teammitglieder beim Teamwechsel (Assignee/Delegate).
pub async fn keep_person_after_move(
    tx: &mut dyn Tx,
    target: &Team,
    user: Option<lineal_domain::ids::UserId>,
) -> AppResult<Option<lineal_domain::ids::UserId>> {
    let Some(id) = user else { return Ok(None) };
    let Some(u) = tx.users().get(id).await? else {
        return Ok(None);
    };
    if !u.active {
        return Ok(None);
    }
    if target.visibility == lineal_domain::enums::TeamVisibility::Public {
        return Ok(Some(id));
    }
    Ok(tx.teams().get_membership(target.id, id).await?.map(|_| id))
}
