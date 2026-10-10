//! usecases – Workflow-Status (docs/03 §1): Default-Workflow, CRUD, Löschen mit Migrationsziel,
//! Vererbung an Sub-Teams mit `inheritWorkflowStatuses`.

use super::team::{emit, validate_color};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::state::{NewState, StatePatch, WorkflowState};
use crate::ports::repos::team::{Team, TeamPatch};
use crate::resolve;
use chrono::{DateTime, Utc};
use lineal_domain::enums::StateType;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{StateId, TeamId};
use serde_json::json;
use uuid::Uuid;

/// Abstand der Positionen (docs/03 §1.2).
pub const POSITION_STEP: f64 = 1000.0;

/// Der Default-Workflow eines neuen Teams ohne Triage (docs/03 §1.2).
pub const DEFAULT_WORKFLOW: [(&str, StateType, &str); 6] = [
    ("Backlog", StateType::Backlog, "#BEC2C8"),
    ("Todo", StateType::Unstarted, "#E2E2E2"),
    ("In Progress", StateType::Started, "#F2C94C"),
    ("Done", StateType::Completed, "#5E6AD2"),
    ("Canceled", StateType::Canceled, "#95A2B3"),
    ("Duplicate", StateType::Duplicate, "#95A2B3"),
];

/// Name und Farbe des Triage-Status.
pub const TRIAGE_STATE: (&str, &str) = ("Triage", "#FC7840");

/// Die Status, auf die ein Team zeigt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkflowPointers {
    pub default_issue: Option<StateId>,
    pub auto_close: Option<StateId>,
    pub triage: Option<StateId>,
}

impl WorkflowPointers {
    /// Als Patch für `TeamRepo::update`.
    pub fn into_patch(self) -> TeamPatch {
        TeamPatch {
            default_issue_state_id: self.default_issue.map(Some),
            auto_close_state_id: self.auto_close.map(Some),
            triage_issue_state_id: self.triage.map(Some),
            ..Default::default()
        }
    }
}

/// Ermittelt Default-, Auto-Close- und Triage-Status aus einer Statusliste.
pub fn pointers_of(states: &[WorkflowState]) -> WorkflowPointers {
    let first = |t: StateType| {
        states
            .iter()
            .filter(|s| s.state_type == t && s.archived_at.is_none())
            .min_by(|a, b| a.position.total_cmp(&b.position))
            .map(|s| s.id)
    };
    WorkflowPointers {
        default_issue: first(StateType::Backlog),
        auto_close: first(StateType::Canceled),
        triage: first(StateType::Triage),
    }
}

/// Legt den Default-Workflow (optional mit Triage) an und liefert die Zeiger für das Team.
pub(crate) async fn create_default_workflow(
    app: &App,
    tx: &mut dyn Tx,
    team: TeamId,
    triage: bool,
    now: DateTime<Utc>,
) -> AppResult<WorkflowPointers> {
    let mut created = Vec::new();
    if triage {
        created.push(insert_state(app, tx, team, TRIAGE_STATE.0, StateType::Triage, TRIAGE_STATE.1, None, POSITION_STEP, None, now).await?);
    }
    for (name, kind, color) in DEFAULT_WORKFLOW {
        created.push(insert_state(app, tx, team, name, kind, color, None, POSITION_STEP, None, now).await?);
    }
    Ok(pointers_of(&created))
}

#[allow(clippy::too_many_arguments)]
async fn insert_state(
    app: &App,
    tx: &mut dyn Tx,
    team: TeamId,
    name: &str,
    kind: StateType,
    color: &str,
    description: Option<String>,
    position: f64,
    inherited_from: Option<StateId>,
    now: DateTime<Utc>,
) -> AppResult<WorkflowState> {
    let id = StateId(app.ids.new_uuid());
    tx.states()
        .insert(&NewState {
            id,
            team_id: team,
            name: name.to_string(),
            state_type: kind,
            color: color.to_string(),
            description,
            position,
            inherited_from_id: inherited_from,
            now,
        })
        .await?;
    tx.states()
        .get(id)
        .await?
        .ok_or_else(|| AppError::internal("state vanished after insert"))
}

/// Kopiert die aktiven Status von `source` (ohne Triage) in `team`. Mit `inherit` entstehen
/// verknüpfte Kopien (`inheritedFrom`), sonst eigenständige.
pub(crate) async fn copy_states(
    app: &App,
    tx: &mut dyn Tx,
    source: TeamId,
    team: TeamId,
    inherit: bool,
    now: DateTime<Utc>,
) -> AppResult<Vec<WorkflowState>> {
    let states = tx.states().list_for_team(source, false).await?;
    let mut created = Vec::new();
    for s in states.iter().filter(|s| s.state_type != StateType::Triage) {
        created.push(
            insert_state(
                app,
                tx,
                team,
                &s.name,
                s.state_type,
                &s.color,
                s.description.clone(),
                s.position,
                inherit.then_some(s.id),
                now,
            )
            .await?,
        );
    }
    Ok(created)
}

/// Legt (bei Bedarf) den Triage-Status des Teams an und liefert ihn.
pub(crate) async fn ensure_triage_state(
    app: &App,
    tx: &mut dyn Tx,
    team: TeamId,
    now: DateTime<Utc>,
) -> AppResult<WorkflowState> {
    let states = tx.states().list_for_team(team, false).await?;
    if let Some(s) = states.into_iter().find(|s| s.state_type == StateType::Triage) {
        return Ok(s);
    }
    insert_state(app, tx, team, TRIAGE_STATE.0, StateType::Triage, TRIAGE_STATE.1, None, POSITION_STEP, None, now).await
}

/// Eingabe für `create_workflow_state`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateStateInput {
    pub team: String,
    pub name: String,
    pub state_type: StateType,
    pub color: String,
    pub description: Option<String>,
    pub position: Option<f64>,
}

/// Eingabe für `update_workflow_state` (`type` ist nicht änderbar).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateStateInput {
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: Option<Option<String>>,
    pub position: Option<f64>,
}

/// Ein Status mit seinem Team (für Antworten).
#[derive(Debug, Clone, PartialEq)]
pub struct StateView {
    pub state: WorkflowState,
    pub team: Team,
    /// `defaultIssueState` des Teams.
    pub is_default: bool,
}

fn validate_name(name: &str) -> AppResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 80 {
        return Err(AppError::invalid_request("name must be 1-80 characters")
            .with_details(json!({ "field": "name" })));
    }
    Ok(n.to_string())
}

fn reason(message: &str, reason: &str) -> AppError {
    AppError::invalid_request(message).with_details(json!({ "reason": reason }))
}

/// Status eines Teams mit geerbtem Workflow dürfen (außer Triage) nur im Parent geändert werden.
fn ensure_editable(team: &Team, state: Option<&WorkflowState>, kind: StateType) -> AppResult<()> {
    let inherited = state.is_some_and(|s| s.inherited_from_id.is_some());
    if kind != StateType::Triage && (team.inherit_workflow_statuses || inherited) {
        return Err(reason(
            "The workflow of this team is inherited from its parent team; change it there",
            "workflow_inherited",
        ));
    }
    Ok(())
}

impl App {
    /// `list_issue_statuses`: Status eines Teams, sortiert nach Kategorie und `position`.
    /// Der Triage-Status erscheint nur bei aktiver Triage.
    pub async fn list_workflow_states(
        &self,
        ctx: &RequestCtx,
        team: &str,
    ) -> AppResult<Vec<StateView>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, resolve::team::Visibility::Content).await?;
        let states = tx.states().list_for_team(team.id, false).await?;
        Ok(states
            .into_iter()
            .filter(|s| s.state_type != StateType::Triage || team.triage_enabled)
            .map(|state| StateView {
                is_default: team.default_issue_state_id == Some(state.id),
                state,
                team: team.clone(),
            })
            .collect())
    }

    /// `get_issue_status`: Status per ID oder Name (im Team). Leere Strings gelten als „nicht angegeben“.
    pub async fn get_workflow_state(
        &self,
        ctx: &RequestCtx,
        id: Option<&str>,
        name: Option<&str>,
        team: Option<&str>,
    ) -> AppResult<StateView> {
        let id = id.map(str::trim).filter(|s| !s.is_empty());
        let name = name.map(str::trim).filter(|s| !s.is_empty());
        let team = team.map(str::trim).filter(|s| !s.is_empty());
        if id.is_none() && name.is_none() {
            return Err(AppError::invalid_request("either id or name is required"));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team_row = match team {
            Some(t) => Some(
                resolve::team::resolve_in(&mut *tx, &snap, t, resolve::team::Visibility::Content)
                    .await?,
            ),
            None => None,
        };
        let state = match (id, &team_row) {
            (Some(id), Some(team)) => resolve::state::resolve(&mut *tx, team, id).await?,
            (Some(id), None) => {
                let uuid = Uuid::parse_str(id).map_err(|_| {
                    AppError::invalid_request("a status id without a team must be a UUID")
                })?;
                resolve::state::by_id(&mut *tx, StateId(uuid)).await?
            }
            (None, Some(team)) => {
                resolve::state::resolve(&mut *tx, team, name.unwrap_or_default()).await?
            }
            (None, None) => {
                return Err(AppError::invalid_request("a team is required to resolve a status by name"));
            }
        };
        // Das Team des Status muss sichtbar sein (Review-Fokus 2).
        let team = match team_row {
            Some(t) => t,
            None => {
                snap.view(state.team_id)
                    .and_then(|v| {
                        if v.can_access {
                            Ok(v.team)
                        } else {
                            Err(crate::policy::team_not_found())
                        }
                    })
                    .map_err(|_| {
                        AppError::not_found("Could not find status")
                            .with_details(json!({ "reference": state.id.to_string(), "type": "WorkflowState" }))
                    })?
            }
        };
        Ok(StateView {
            is_default: team.default_issue_state_id == Some(state.id),
            state,
            team,
        })
    }

    /// Legt einen Status an (docs/03 §1.3) und verteilt ihn an erbende Sub-Teams.
    pub async fn create_workflow_state(
        &self,
        ctx: &RequestCtx,
        input: CreateStateInput,
    ) -> AppResult<StateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, &input.team, resolve::team::Visibility::Management).await?;
        snap.require_team(ctx, Action::UpdateTeamSettings, team.id)?;
        let name = validate_name(&input.name)?;
        validate_color(&input.color)?;
        if input.state_type == StateType::Duplicate {
            return Err(reason("The duplicate status is managed by the system", "duplicate_is_system_managed"));
        }
        ensure_editable(&team, None, input.state_type)?;
        let existing = tx.states().list_for_team(team.id, false).await?;
        if existing.iter().any(|s| s.name.to_lowercase() == name.to_lowercase()) {
            return Err(reason("A status with this name already exists in the team", "duplicate_name"));
        }
        if input.state_type == StateType::Triage && existing.iter().any(|s| s.state_type == StateType::Triage) {
            return Err(reason("The team already has a triage status", "triage_exists"));
        }
        let position = input.position.unwrap_or_else(|| {
            existing
                .iter()
                .filter(|s| s.state_type == input.state_type)
                .map(|s| s.position)
                .fold(None, |acc: Option<f64>, p| Some(acc.map_or(p, |a| a.max(p))))
                .map_or(POSITION_STEP, |m| m + POSITION_STEP)
        });
        let now = self.clock.now();
        let created = insert_state(self, &mut *tx, team.id, &name, input.state_type, &input.color, input.description.clone(), position, None, now).await?;
        // Triage-Status: Team-Zeiger setzen.
        if created.state_type == StateType::Triage {
            tx.teams()
                .update(team.id, &TeamPatch { triage_issue_state_id: Some(Some(created.id)), triage_enabled: Some(true), ..Default::default() }, now)
                .await?;
        }
        propagate_create(self, &mut *tx, &created, now).await?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        let team = tx.teams().get(team.id).await?.unwrap_or(team);
        tx.commit().await?;
        Ok(StateView { is_default: team.default_issue_state_id == Some(created.id), state: created, team })
    }

    /// Ändert Name, Farbe, Beschreibung oder Position (docs/03 §1.3).
    pub async fn update_workflow_state(
        &self,
        ctx: &RequestCtx,
        id: StateId,
        input: UpdateStateInput,
    ) -> AppResult<StateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let state = visible_state(&mut *tx, &snap, id).await?;
        snap.require_team(ctx, Action::UpdateTeamSettings, state.team_id)?;
        let team = snap.view(state.team_id)?.team;
        ensure_editable(&team, Some(&state), state.state_type)?;
        let name = input.name.as_deref().map(validate_name).transpose()?;
        if let Some(c) = &input.color {
            validate_color(c)?;
        }
        if state.state_type == StateType::Duplicate
            && (name.is_some() || input.description.is_some() || input.position.is_some())
        {
            return Err(reason("Only the color of the duplicate status can be changed", "duplicate_is_system_managed"));
        }
        if let Some(n) = &name {
            let existing = tx.states().list_for_team(team.id, false).await?;
            if existing.iter().any(|s| s.id != state.id && s.name.to_lowercase() == n.to_lowercase()) {
                return Err(reason("A status with this name already exists in the team", "duplicate_name"));
            }
        }
        let patch = StatePatch {
            name,
            color: input.color,
            description: input.description,
            position: input.position,
            inherited_from: None,
        };
        let now = self.clock.now();
        let updated = tx.states().update(state.id, &patch, now).await?;
        propagate_update(&mut *tx, state.id, &patch, now).await?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(StateView { is_default: team.default_issue_state_id == Some(updated.id), state: updated, team })
    }

    /// Löscht (archiviert) einen Status; hängen Issues daran, ist ein Migrationsziel nötig
    /// (docs/03 §1.4).
    pub async fn delete_workflow_state(
        &self,
        ctx: &RequestCtx,
        id: StateId,
        target: Option<StateId>,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let state = visible_state(&mut *tx, &snap, id).await?;
        snap.require_team(ctx, Action::UpdateTeamSettings, state.team_id)?;
        let team = snap.view(state.team_id)?.team;
        ensure_editable(&team, Some(&state), state.state_type)?;
        if state.archived_at.is_some() {
            return Err(AppError::not_found("Could not find status")
                .with_details(json!({ "reference": id.to_string(), "type": "WorkflowState" })));
        }
        let now = self.clock.now();
        delete_state_inner(&mut *tx, ctx, &team, &state, target, now).await?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Lädt einen Status; Status unsichtbarer Teams sind `not_found`.
pub(crate) async fn visible_state(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    id: StateId,
) -> AppResult<WorkflowState> {
    let not_found = || {
        AppError::not_found("Could not find status")
            .with_details(json!({ "reference": id.to_string(), "type": "WorkflowState" }))
    };
    let state = tx.states().get(id).await?.ok_or_else(not_found)?;
    let can_see = snap
        .view(state.team_id)
        .map(|v| v.can_see_metadata)
        .unwrap_or(false);
    if !can_see {
        return Err(not_found());
    }
    Ok(state)
}

async fn delete_state_inner(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    team: &Team,
    state: &WorkflowState,
    target: Option<StateId>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if state.state_type == StateType::Duplicate {
        return Err(reason("The duplicate status cannot be deleted", "duplicate_is_system_managed"));
    }
    if team.default_issue_state_id == Some(state.id) {
        return Err(reason("The default issue status cannot be deleted; choose another default first", "default_state"));
    }
    if state.state_type != StateType::Triage
        && tx.states().count_active_of_type(team.id, state.state_type).await? <= 1
    {
        return Err(reason("Every category needs at least one status", "last_state_of_type"));
    }
    let affected = tx.states().count_issues(state.id).await?;
    if affected > 0 {
        let Some(target_id) = target else {
            return Err(AppError::invalid_request("A migration target status is required")
                .with_details(json!({ "reason": "migration_target_required", "issueCount": affected })));
        };
        let target = tx
            .states()
            .get(target_id)
            .await?
            .ok_or_else(|| reason("Migration target not found", "invalid_migration_target"))?;
        if target.team_id != state.team_id || target.archived_at.is_some() || target.id == state.id {
            return Err(reason("Invalid migration target", "invalid_migration_target"));
        }
        let actor = crate::resolve::user::actor_user_id(ctx);
        tx.states().migrate_issues(state.id, target.id, actor, now).await?;
    }
    let mut patch = TeamPatch::default();
    if state.state_type == StateType::Triage {
        patch.triage_enabled = Some(false);
        patch.triage_issue_state_id = Some(None);
    }
    if team.auto_close_state_id == Some(state.id) {
        let states = tx.states().list_for_team(team.id, false).await?;
        patch.auto_close_state_id = Some(
            states
                .iter()
                .find(|s| s.state_type == StateType::Canceled && s.id != state.id)
                .map(|s| s.id),
        );
    }
    if patch != TeamPatch::default() {
        tx.teams().update(team.id, &patch, now).await?;
    }
    tx.states().archive(state.id, now).await?;
    // Erbende Kopien in Sub-Teams folgen (Ziel = Kopie des Migrationsziels im jeweiligen Team).
    let copies = tx.states().list_copies_of(state.id).await?;
    for copy in copies {
        let Some(child_team) = tx.teams().get(copy.team_id).await? else { continue };
        let child_target = match target {
            Some(t) => tx
                .states()
                .list_copies_of(t)
                .await?
                .into_iter()
                .find(|c| c.team_id == copy.team_id)
                .map(|c| c.id),
            None => None,
        };
        Box::pin(delete_state_inner(tx, ctx, &child_team, &copy, child_target, now)).await?;
    }
    Ok(())
}

async fn propagate_create(
    app: &App,
    tx: &mut dyn Tx,
    created: &WorkflowState,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if created.state_type == StateType::Triage {
        return Ok(()); // Triage bleibt team-eigen (docs/02 §3.2).
    }
    // Direkte Kinder mit geerbtem Workflow, rekursiv (Kette Parent -> Kind -> Enkel).
    let teams = tx.teams().list_all().await?;
    let mut stack: Vec<(TeamId, StateId)> = vec![(created.team_id, created.id)];
    while let Some((parent, source)) = stack.pop() {
        for child in teams
            .iter()
            .filter(|t| t.parent_id == Some(parent) && t.inherit_workflow_statuses && !t.trashed)
        {
            let copy = insert_state(
                app,
                tx,
                child.id,
                &created.name,
                created.state_type,
                &created.color,
                created.description.clone(),
                created.position,
                Some(source),
                now,
            )
            .await?;
            stack.push((child.id, copy.id));
        }
    }
    Ok(())
}

async fn propagate_update(
    tx: &mut dyn Tx,
    source: StateId,
    patch: &StatePatch,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let mut stack = vec![source];
    while let Some(src) = stack.pop() {
        for copy in tx.states().list_copies_of(src).await? {
            tx.states().update(copy.id, patch, now).await?;
            stack.push(copy.id);
        }
    }
    Ok(())
}

/// Ersetzt die eigenen Status von `team` durch verknüpfte Kopien der Status von `parent`
/// (Nesting, docs/02 §3.3 Schritt 5) und hängt die Issues um. Liefert die neuen Zeiger.
pub(crate) async fn normalize_to_parent(
    app: &App,
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    team: &Team,
    parent: TeamId,
    mapping_override: &[(StateId, StateId)],
    now: DateTime<Utc>,
) -> AppResult<WorkflowPointers> {
    let own: Vec<WorkflowState> = tx
        .states()
        .list_for_team(team.id, false)
        .await?
        .into_iter()
        .filter(|s| s.state_type != StateType::Triage)
        .collect();
    let targets: Vec<WorkflowState> = tx
        .states()
        .list_for_team(parent, false)
        .await?
        .into_iter()
        .filter(|s| s.state_type != StateType::Triage)
        .collect();
    // Mapping je eigenem Status: Override, sonst gleicher Typ + Name, sonst erster der Kategorie.
    let mut mapping: Vec<(StateId, StateId)> = Vec::new();
    for s in &own {
        let target = mapping_override
            .iter()
            .find(|(from, _)| *from == s.id)
            .and_then(|(_, to)| targets.iter().find(|t| t.id == *to))
            .or_else(|| {
                targets
                    .iter()
                    .find(|t| t.state_type == s.state_type && t.name.to_lowercase() == s.name.to_lowercase())
            })
            .or_else(|| {
                targets
                    .iter()
                    .filter(|t| t.state_type == s.state_type)
                    .min_by(|a, b| a.position.total_cmp(&b.position))
            })
            .ok_or_else(|| AppError::internal("parent team misses a workflow category"))?;
        mapping.push((s.id, target.id));
    }
    // Alte Status zuerst archivieren (gibt die Namen frei), dann Kopien anlegen, dann Issues umhängen.
    // Der Duplicate-Status ist je Team eindeutig (auch archiviert): er wird verknüpft statt kopiert.
    for s in own.iter().filter(|s| s.state_type != StateType::Duplicate) {
        tx.states().archive(s.id, now).await?;
    }
    let mut copy_of: Vec<(StateId, StateId)> = Vec::new(); // parent state -> copy
    for t in &targets {
        if t.state_type == StateType::Duplicate {
            if let Some(dup) = own.iter().find(|s| s.state_type == StateType::Duplicate) {
                tx.states()
                    .update(
                        dup.id,
                        &StatePatch {
                            name: Some(t.name.clone()),
                            color: Some(t.color.clone()),
                            description: Some(t.description.clone()),
                            position: Some(t.position),
                            inherited_from: Some(Some(t.id)),
                        },
                        now,
                    )
                    .await?;
                copy_of.push((t.id, dup.id));
                continue;
            }
        }
        let c = insert_state(
            app,
            tx,
            team.id,
            &t.name,
            t.state_type,
            &t.color,
            t.description.clone(),
            t.position,
            Some(t.id),
            now,
        )
        .await?;
        copy_of.push((t.id, c.id));
    }
    let actor = crate::resolve::user::actor_user_id(ctx);
    let lookup = |parent_state: StateId| copy_of.iter().find(|(p, _)| *p == parent_state).map(|(_, c)| *c);
    for (from, to_parent) in &mapping {
        if let Some(to) = lookup(*to_parent) {
            tx.states().migrate_issues(*from, to, actor, now).await?;
        }
    }
    let new_states = tx.states().list_for_team(team.id, false).await?;
    let mut pointers = pointers_of(&new_states);
    // Zeiger des Teams auf die Kopie des bisherigen Ziels umbiegen, sonst Default.
    let remap = |old: Option<StateId>| {
        old.and_then(|o| mapping.iter().find(|(f, _)| *f == o))
            .and_then(|(_, p)| lookup(*p))
    };
    pointers.default_issue = remap(team.default_issue_state_id).or(pointers.default_issue);
    pointers.auto_close = remap(team.auto_close_state_id).or(pointers.auto_close);
    pointers.triage = team.triage_issue_state_id;
    Ok(pointers)
}

/// Löst die Kopplung beim Un-Nesting: benutzte geerbte Status werden eigenständig, unbenutzte
/// archiviert; fehlende Kategorien werden aus dem Default nachgelegt (docs/02 §3.5 Schritt 2).
pub(crate) async fn detach_from_parent(
    app: &App,
    tx: &mut dyn Tx,
    team: &Team,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let states = tx.states().list_for_team(team.id, false).await?;
    for s in states.iter().filter(|s| s.inherited_from_id.is_some()) {
        let used = tx.states().count_issues(s.id).await? > 0
            || team.default_issue_state_id == Some(s.id)
            || team.auto_close_state_id == Some(s.id)
            || s.state_type == StateType::Duplicate;
        if used {
            tx.states()
                .update(s.id, &StatePatch { inherited_from: Some(None), ..Default::default() }, now)
                .await?;
        } else {
            tx.states().archive(s.id, now).await?;
        }
    }
    // Pflicht-Kategorien prüfen.
    let remaining = tx.states().list_for_team(team.id, false).await?;
    for (name, kind, color) in DEFAULT_WORKFLOW {
        if !remaining.iter().any(|s| s.state_type == kind) {
            insert_state(app, tx, team.id, name, kind, color, None, POSITION_STEP, None, now).await?;
        }
    }
    Ok(())
}
