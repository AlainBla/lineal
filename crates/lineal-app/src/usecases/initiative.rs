//! usecases – Initiativen (docs/04 §4): workspaceweit, Hierarchie als DAG (Zyklus- und
//! Tiefenprüfung), Projektzuordnung, Health-Rollup, Sichtbarkeit über das Lead-Team.
//!
//! Status-Updates liegen in [`update`], der Graph in [`graph`].

pub mod graph;
pub mod update;

use super::project::emit;
use super::project::pure::{self, HealthCounts, health_rollup, patch_error};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::initiative::{
    Initiative, InitiativeFilter, InitiativeLabelRow, InitiativePatch, NewInitiative,
};
use crate::ports::repos::project::{Project, ProjectStatus};
use crate::ports::repos::team::Team;
use crate::ports::repos::user::{Page, PageRequest, User};
use crate::resolve;
use crate::resolve::team::Visibility;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_domain::enums::{DateResolution, Health, InitiativeStatus, ProjectStatusType, UserRole};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{InitiativeId, ProjectId};
use lineal_domain::rules::patch::{self, PatchOp};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

/// Ein direkt zugeordnetes, für den Betrachter sichtbares Projekt.
#[derive(Debug, Clone, PartialEq)]
pub struct InitiativeProject {
    pub project: Project,
    pub identifier: String,
    pub status: ProjectStatus,
}

/// Eine Initiative, wie ein Betrachter sie sieht.
#[derive(Debug, Clone, PartialEq)]
pub struct InitiativeView {
    pub initiative: Initiative,
    pub identifier: String,
    pub owner: Option<User>,
    pub creator: Option<User>,
    /// `None`, wenn es kein Lead-Team gibt (die Sichtbarkeit wurde bereits geprüft).
    pub lead_team: Option<Team>,
    /// `public` (kein Lead-Team oder öffentliches Team), `restricted` oder `private`.
    pub visibility: &'static str,
    pub parents: Vec<Initiative>,
    pub sub_initiatives: Vec<Initiative>,
    pub labels: Vec<InitiativeLabelRow>,
    /// Direkt zugeordnete, sichtbare Projekte.
    pub projects: Vec<InitiativeProject>,
    /// Zähler und Farbe des Projekt-Rollups (rekursiv, nur sichtbare aktive Projekte).
    pub health_rollup: HealthCounts,
}

/// Eingabe von `save_initiative`.
#[derive(Debug, Clone, Default)]
pub struct SaveInitiativeInput {
    pub id: Option<String>,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub patch: Option<Vec<PatchOp>>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub priority: Option<i32>,
    /// Name, `id` oder `type` des Status (`Active`, `active`, `started` …).
    pub status: Option<String>,
    pub owner: Option<Option<String>>,
    pub lead_team: Option<Option<String>>,
    /// Append-only: fügt Eltern hinzu.
    pub parent_initiatives: Vec<String>,
    pub labels: Option<Vec<String>>,
    pub target_date: Option<Option<NaiveDate>>,
    pub target_date_resolution: Option<Option<DateResolution>>,
}

/// Filter von `list_initiatives`.
#[derive(Debug, Clone, Default)]
pub struct ListInitiativesInput {
    /// Gespeicherte View (WP11); nicht mit `query` kombinierbar.
    pub custom_view: Option<String>,
    pub status: Option<String>,
    pub owner: Option<String>,
    pub lead_team: Option<String>,
    pub parent_initiative: Option<String>,
    pub label: Option<String>,
    pub query: Option<String>,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub page: PageRequest,
}

/// Status aus Eingabe: Name (case-insensitiv), `id` (`active`) oder `type` (`started`).
pub fn parse_initiative_status(s: &str) -> Option<InitiativeStatus> {
    let l = s.trim().to_lowercase();
    match l.as_str() {
        "proposed" | "backlog" => Some(InitiativeStatus::Proposed),
        "planned" => Some(InitiativeStatus::Planned),
        "active" | "started" => Some(InitiativeStatus::Active),
        "completed" => Some(InitiativeStatus::Completed),
        "canceled" => Some(InitiativeStatus::Canceled),
        _ => None,
    }
}

/// `StatusRef` der Initiative: `(id, name, type)` (docs/07 §3.10).
pub fn initiative_status_ref(s: InitiativeStatus) -> (&'static str, &'static str, &'static str) {
    match s {
        InitiativeStatus::Proposed => ("proposed", "Proposed", "backlog"),
        InitiativeStatus::Planned => ("planned", "Planned", "planned"),
        InitiativeStatus::Active => ("active", "Active", "started"),
        InitiativeStatus::Completed => ("completed", "Completed", "completed"),
        InitiativeStatus::Canceled => ("canceled", "Canceled", "canceled"),
    }
}

fn bad_status(s: &str) -> AppError {
    AppError::not_found(format!("Unknown initiative status \"{s}\""))
        .with_details(json!({ "reference": s, "type": "InitiativeStatus" }))
}

fn max_depth(limits: &serde_json::Value) -> usize {
    limits
        .get("maxInitiativeDepth")
        .and_then(serde_json::Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(5)
}

/// Gäste dürfen keine Initiativen sehen (`forbidden`).
fn require_view(snap: &PolicySnapshot, ctx: &RequestCtx) -> AppResult<()> {
    snap.require_workspace(ctx, Action::ViewInitiatives)
}

async fn require_enabled(tx: &mut dyn Tx) -> AppResult<serde_json::Value> {
    match tx.orgs().get().await? {
        Some(org) if !org.initiatives_enabled => Err(AppError::forbidden(
            "Initiatives are not enabled in this workspace.",
        )
        .with_details(json!({ "reason": "initiatives_disabled" }))),
        Some(org) => Ok(org.limits),
        None => Ok(json!({})),
    }
}

fn graph_error(e: graph::GraphError, ident: impl Fn(InitiativeId) -> String) -> AppError {
    match e {
        graph::GraphError::Cycle(path) => AppError::invalid_request(format!(
            "Adding {} as parent of {} would create a cycle",
            ident(path[path.len().saturating_sub(2)]),
            ident(path[0])
        ))
        .with_details(json!({
            "reason": "cycle",
            "cycle": path.iter().map(|i| ident(*i)).collect::<Vec<_>>(),
        })),
        graph::GraphError::MaxDepth { depth, limit } => AppError::invalid_request(format!(
            "The initiative hierarchy would be {depth} levels deep (limit {limit})"
        ))
        .with_details(json!({ "reason": "max_depth", "depth": depth, "limit": limit })),
    }
}

fn validate_name(name: &str) -> AppResult<String> {
    let n = super::project::clean_text(name);
    Ok(lineal_domain::rules::text::validate_single_line_name(
        "name", &n, 255,
    )?)
}

/// Löst Initiative-Labels auf (UUID, Name oder `Gruppe/Label`); Initiative-Gruppen sind immer multiSelect.
async fn resolve_labels(
    tx: &mut dyn Tx,
    refs: &[String],
    existing: &[InitiativeLabelRow],
) -> AppResult<Vec<InitiativeLabelRow>> {
    let unknown = |r: &str| {
        AppError::not_found(format!("Could not find initiative label \"{r}\""))
            .with_details(json!({ "reference": r, "type": "InitiativeLabel" }))
    };
    let mut out: Vec<InitiativeLabelRow> = Vec::new();
    for r in refs {
        let r = r.trim();
        let label = if let Ok(id) = Uuid::parse_str(r) {
            tx.initiatives().get_label(id).await?
        } else {
            let (group, name) = match r.split_once('/') {
                Some((g, n)) => (Some(g.trim()), n.trim()),
                None => (None, r),
            };
            let mut found = None;
            for l in tx.initiatives().find_labels_by_name(name).await? {
                if l.retired && !existing.iter().any(|e| e.id == l.id) {
                    continue;
                }
                if let Some(g) = group {
                    let parent = match l.parent_id {
                        Some(p) => tx.initiatives().get_label(p).await?,
                        None => None,
                    };
                    if !parent.is_some_and(|p| p.name.eq_ignore_ascii_case(g)) {
                        continue;
                    }
                }
                found = Some(l);
                break;
            }
            found
        };
        let label = label.ok_or_else(|| unknown(r))?;
        if label.is_group {
            return Err(AppError::invalid_request(format!(
                "\"{}\" is a label group; pick one of its labels.",
                label.name
            ))
            .with_details(json!({ "field": "labels" })));
        }
        if !out.iter().any(|o| o.id == label.id) {
            out.push(label);
        }
    }
    Ok(out)
}

impl App {
    /// Baut Sichten (Hierarchie, Projekte, Labels, Rollup) für Initiativen.
    async fn initiative_views(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        initiatives: Vec<Initiative>,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<InitiativeView>> {
        if initiatives.is_empty() {
            return Ok(Vec::new());
        }
        let reminder = tx.projects().reminder_settings().await?;
        let all = tx.initiatives().list_all().await?;
        let edges = tx.initiatives().list_edges().await?;
        let statuses = tx.projects().list_statuses().await?;
        let visible = snap.visible_team_ids();
        let by_id: HashMap<InitiativeId, &Initiative> = all.iter().map(|i| (i.id, i)).collect();
        let mut users: HashMap<_, Option<User>> = HashMap::new();
        let mut out = Vec::with_capacity(initiatives.len());
        for i in initiatives {
            let parents: Vec<Initiative> = graph::parents_of(&edges, i.id)
                .into_iter()
                .filter_map(|p| by_id.get(&p).copied())
                .filter(|p| resolve::initiative::is_visible(snap, p))
                .cloned()
                .collect();
            let sub_initiatives: Vec<Initiative> = graph::children_of(&edges, i.id)
                .into_iter()
                .filter_map(|c| by_id.get(&c).copied())
                .filter(|c| resolve::initiative::is_visible(snap, c))
                .cloned()
                .collect();
            // Direkte und rekursive Projektmenge (sichtbare Projekte).
            let direct = tx.initiatives().project_ids(i.id).await?;
            let mut recursive: Vec<ProjectId> = direct.clone();
            for d in graph::descendants(&edges, i.id) {
                for p in tx.initiatives().project_ids(d).await? {
                    if !recursive.contains(&p) {
                        recursive.push(p);
                    }
                }
            }
            let mut projects_by_id: HashMap<ProjectId, Project> = HashMap::new();
            for pid in &recursive {
                if let Some(p) = tx.projects().get(*pid).await?
                    && !p.trashed
                    && p.archived_at.is_none()
                {
                    projects_by_id.insert(*pid, p);
                }
            }
            let pids: Vec<ProjectId> = projects_by_id.keys().copied().collect();
            let team_rows = tx.projects().teams_of_projects(&pids).await?;
            let project_visible = |pid: ProjectId| {
                team_rows
                    .iter()
                    .any(|(p, t)| *p == pid && visible.contains(t))
            };
            let mut projects = Vec::new();
            let mut health_inputs: Vec<Option<Health>> = Vec::new();
            for pid in &recursive {
                let Some(p) = projects_by_id.get(pid) else {
                    continue;
                };
                if !project_visible(*pid) {
                    continue;
                }
                let Some(status) = statuses.iter().find(|s| s.id == p.status_id) else {
                    continue;
                };
                {
                    let key = snap
                        .tree
                        .get(p.lead_team_id)
                        .map(|t| t.key.clone())
                        .unwrap_or_default();
                    projects.push(InitiativeProject {
                        identifier: format!("P-{}-{}", key, p.number),
                        status: status.clone(),
                        project: p.clone(),
                    });
                }
                if status.status_type == ProjectStatusType::Started {
                    // Veraltete Updates zählen nicht als Health (docs/04 §4.5).
                    let days = super::project::update::reminder_cycle_days(
                        &p.update_reminder_mode,
                        p.update_reminder_frequency_in_weeks,
                        reminder.as_ref().and_then(|s| s.project_frequency_in_weeks),
                    );
                    let st = pure::staleness(
                        !status.indefinite,
                        days,
                        p.health.zip(p.health_updated_at).map(|(h, at)| (at, h)),
                        now,
                    );
                    let stale = matches!(st, pure::Staleness::Missing | pure::Staleness::Grey);
                    health_inputs.push(if stale { None } else { p.health });
                }
            }
            let owner = match i.owner_id {
                Some(id) => {
                    if let std::collections::hash_map::Entry::Vacant(e) = users.entry(id) {
                        e.insert(tx.users().get(id).await?);
                    }
                    users.get(&id).cloned().flatten()
                }
                None => None,
            };
            let creator = match i.creator_id {
                Some(id) => {
                    if let std::collections::hash_map::Entry::Vacant(e) = users.entry(id) {
                        e.insert(tx.users().get(id).await?);
                    }
                    users.get(&id).cloned().flatten()
                }
                None => None,
            };
            let lead_team = i.lead_team_id.and_then(|t| snap.tree.get(t).cloned());
            let visibility = match &lead_team {
                None => "public",
                Some(t) => match t.visibility {
                    lineal_domain::enums::TeamVisibility::Public => "public",
                    lineal_domain::enums::TeamVisibility::Restricted => "restricted",
                    lineal_domain::enums::TeamVisibility::Private => "private",
                },
            };
            let labels = tx.initiatives().labels_of(i.id).await?;
            out.push(InitiativeView {
                identifier: i.identifier(),
                owner,
                creator,
                lead_team,
                visibility,
                parents,
                sub_initiatives,
                labels,
                projects,
                health_rollup: health_rollup(health_inputs),
                initiative: i,
            });
        }
        Ok(out)
    }

    /// `list_initiatives`: Gäste erhalten `forbidden`; private Lead-Teams bleiben unsichtbar.
    pub async fn list_initiatives(
        &self,
        ctx: &RequestCtx,
        input: ListInitiativesInput,
    ) -> AppResult<(Page<Initiative>, Vec<InitiativeView>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        require_view(&snap, ctx)?;
        require_enabled(&mut *tx).await?;
        let mut filter = InitiativeFilter {
            visible_teams: snap.visible_team_ids().into_iter().collect(),
            include_archived: input.include_archived,
            created_after: input.created_after,
            updated_after: input.updated_after,
            query: input.query.clone(),
            ..Default::default()
        };
        if let Some(s) = &input.status {
            filter.status = Some(parse_initiative_status(s).ok_or_else(|| bad_status(s))?);
        }
        if let Some(o) = &input.owner {
            filter.owner = Some(resolve::user::resolve_in(&mut *tx, &snap, ctx, o).await?.id);
        }
        if let Some(t) = &input.lead_team {
            filter.lead_team = Some(
                resolve::team::resolve_in(&mut *tx, &snap, t, Visibility::Content)
                    .await?
                    .id,
            );
        }
        if let Some(p) = &input.parent_initiative {
            filter.parent = Some(
                resolve::initiative::resolve_in(&mut *tx, &snap, p)
                    .await?
                    .id,
            );
        }
        if let Some(l) = &input.label {
            let r = l.trim();
            let ids: Vec<Uuid> = if let Ok(id) = Uuid::parse_str(r) {
                tx.initiatives()
                    .get_label(id)
                    .await?
                    .map(|x| vec![x.id])
                    .unwrap_or_default()
            } else {
                tx.initiatives()
                    .find_labels_by_name(r)
                    .await?
                    .into_iter()
                    .map(|x| x.id)
                    .collect()
            };
            if ids.is_empty() {
                return Err(AppError::not_found(format!(
                    "Could not find initiative label \"{r}\""
                ))
                .with_details(json!({ "reference": r, "type": "InitiativeLabel" })));
            }
            filter.label_ids = Some(ids);
        }
        if let Some(r) = &input.custom_view {
            if input.query.as_deref().is_some_and(|q| !q.trim().is_empty()) {
                return Err(
                    AppError::invalid_request("query cannot be combined with customView")
                        .with_details(
                            json!({ "field": "query", "reason": "conflicts_with_customView" }),
                        ),
                );
            }
            let v = super::view::view_for_list(
                &mut *tx,
                &snap,
                ctx,
                r,
                crate::ports::repos::view::ViewModel::Initiative,
            )
            .await?;
            let eval = super::view::build_eval(&mut *tx, &snap, ctx, self.clock.now(), false, true)
                .await?;
            let ast: lineal_domain::rules::filter::InitiativeFilter =
                lineal_domain::rules::filter::from_json_value(&super::view::combined_filter(
                    &v,
                    Vec::new(),
                ))
                .map_err(|e| AppError::invalid_request(format!("Invalid filter: {}", e.reason)))?;
            filter.ast = Some(Box::new(ast));
            filter.eval = Some(eval);
        }
        let page = tx.initiatives().list(&filter, &input.page).await?;
        let views = self
            .initiative_views(&mut *tx, &snap, page.items.clone(), self.clock.now())
            .await?;
        Ok((page, views))
    }

    /// `get_initiative`.
    pub async fn get_initiative(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<InitiativeView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        require_view(&snap, ctx)?;
        require_enabled(&mut *tx).await?;
        let i = resolve::initiative::resolve_in(&mut *tx, &snap, reference).await?;
        let mut v = self
            .initiative_views(&mut *tx, &snap, vec![i], self.clock.now())
            .await?;
        v.pop()
            .ok_or_else(|| AppError::internal("initiative view missing"))
    }

    /// `save_initiative`: Create ohne `id`, Update mit `id`. `parentInitiatives` fügt Eltern hinzu.
    pub async fn save_initiative(
        &self,
        ctx: &RequestCtx,
        input: SaveInitiativeInput,
    ) -> AppResult<InitiativeView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        let limits = require_enabled(&mut *tx).await?;
        let now = self.clock.now();
        let input = &SaveInitiativeInput {
            summary: input.summary.as_deref().map(super::project::clean_text),
            icon: match input.icon.as_deref() {
                Some(i) => Some(super::project::validate_icon(i)?.unwrap_or_default()),
                None => None,
            },
            ..input.clone()
        };
        if let Some(s) = &input.summary
            && s.chars().count() > 255
        {
            return Err(
                AppError::invalid_request("summary must be at most 255 characters")
                    .with_details(json!({ "field": "summary" })),
            );
        }
        if let Some(p) = input.priority
            && !(0..=4).contains(&p)
        {
            return Err(
                AppError::invalid_request("priority must be between 0 and 4")
                    .with_details(json!({ "field": "priority" })),
            );
        }
        if let Some(c) = &input.color
            && !(c.len() == 7
                && c.starts_with('#')
                && c[1..].chars().all(|x| x.is_ascii_hexdigit()))
        {
            return Err(AppError::invalid_request("color must be #RRGGBB")
                .with_details(json!({ "field": "color" })));
        }
        if input.description.is_some() && input.patch.is_some() {
            return Err(AppError::invalid_request(
                "`description` and `patch` cannot be combined",
            ));
        }
        let status = input
            .status
            .as_deref()
            .map(|s| parse_initiative_status(s).ok_or_else(|| bad_status(s)))
            .transpose()?;
        let lead_team = match &input.lead_team {
            Some(Some(r)) => Some(Some(
                resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Content)
                    .await?
                    .id,
            )),
            Some(None) => Some(None),
            None => None,
        };
        let owner = match &input.owner {
            Some(Some(r)) => {
                let u = resolve::user::resolve_in(&mut *tx, &snap, ctx, r).await?;
                if u.role == UserRole::Guest {
                    return Err(
                        AppError::invalid_request("A guest cannot own an initiative.")
                            .with_details(json!({ "field": "owner" })),
                    );
                }
                Some(Some(u.id))
            }
            Some(None) => Some(None),
            None => None,
        };
        if let Some(d) = &input.description {
            pure::check_len("description", d, 500_000)?;
        }
        let content_from_description = input
            .description
            .as_deref()
            .map(lineal_domain::rules::markdown::sanitize)
            .map(|c| (!c.is_empty()).then_some(c));

        let id = match &input.id {
            None => {
                if input.patch.is_some() {
                    return Err(AppError::invalid_request(
                        "`patch` cannot be used when creating an initiative",
                    )
                    .with_details(json!({ "field": "patch" })));
                }
                if lead_team.flatten().is_none() {
                    snap.require_initiative_without_lead_team(ctx)?;
                }
                let name = validate_name(input.name.as_deref().unwrap_or(""))?;
                let status = status.unwrap_or(InitiativeStatus::Planned);
                let number = tx.initiatives().next_number().await?;
                let id = InitiativeId(self.ids.new_uuid());
                let (td, tdr) = {
                    let (_, _, t, tr) = pure::resolve_dates(
                        None,
                        None,
                        input.target_date.flatten(),
                        input.target_date_resolution.flatten(),
                        super::project::fiscal_month(&mut *tx).await?,
                    )?;
                    (t, tr)
                };
                tx.initiatives()
                    .insert(&NewInitiative {
                        id,
                        number,
                        name,
                        description: input.summary.clone().filter(|s| !s.is_empty()),
                        content: content_from_description.flatten(),
                        color: input.color.clone(),
                        icon: input.icon.clone().filter(|i| !i.is_empty()),
                        status,
                        owner_id: owner.flatten(),
                        creator_id: resolve::user::actor_user_id(ctx),
                        lead_team_id: lead_team.flatten(),
                        priority: input.priority.unwrap_or(0),
                        sort_order: now.timestamp_millis() as f64,
                        target_date: td,
                        target_date_resolution: tdr,
                        started_at: (status == InitiativeStatus::Active).then_some(now),
                        completed_at: (status == InitiativeStatus::Completed).then_some(now),
                        now,
                    })
                    .await?;
                emit(
                    &mut *tx,
                    ctx,
                    DomainEvent::InitiativeCreated { initiative_id: id },
                )
                .await?;
                id
            }
            Some(reference) => {
                let cur = resolve::initiative::resolve_in(&mut *tx, &snap, reference).await?;
                if cur.archived_at.is_some() || cur.trashed {
                    return Err(AppError::invalid_request(
                        "The initiative is archived or deleted and read-only.",
                    )
                    .with_details(json!({ "reason": "initiative_archived" })));
                }
                let mut patch = InitiativePatch::default();
                if let Some(n) = &input.name {
                    patch.name = Some(validate_name(n)?);
                }
                if let Some(s) = &input.summary {
                    patch.description = Some((!s.is_empty()).then(|| s.clone()));
                }
                if let Some(c) = content_from_description {
                    patch.content = Some(c);
                }
                if let Some(ops) = &input.patch {
                    let next = patch::apply(cur.content.as_deref().unwrap_or(""), ops)
                        .map_err(patch_error)?;
                    let clean = lineal_domain::rules::markdown::sanitize(&next);
                    patch.content = Some((!clean.is_empty()).then_some(clean));
                }
                if let Some(c) = &input.color {
                    patch.color = Some(Some(c.clone()));
                }
                if let Some(i) = &input.icon {
                    patch.icon = Some((!i.is_empty()).then(|| i.clone()));
                }
                patch.priority = input.priority;
                patch.owner_id = owner;
                patch.lead_team_id = lead_team;
                if let Some(s) = status
                    && s != cur.status
                {
                    patch.status = Some(s);
                    match s {
                        InitiativeStatus::Active if cur.started_at.is_none() => {
                            patch.started_at = Some(Some(now));
                        }
                        InitiativeStatus::Completed => patch.completed_at = Some(Some(now)),
                        _ => {}
                    }
                    if cur.status == InitiativeStatus::Completed && s != InitiativeStatus::Completed
                    {
                        patch.completed_at = Some(None);
                    }
                }
                if input.target_date.is_some() || input.target_date_resolution.is_some() {
                    let target = input.target_date.unwrap_or(cur.target_date);
                    let res = match (&input.target_date, input.target_date_resolution) {
                        (_, Some(r)) => r,
                        (Some(_), None) => None,
                        (None, None) => cur.target_date_resolution,
                    };
                    let res = if target.is_none() { None } else { res };
                    let (_, _, t, tr) = pure::resolve_dates(
                        None,
                        None,
                        target,
                        res,
                        super::project::fiscal_month(&mut *tx).await?,
                    )?;
                    patch.target_date = Some(t);
                    patch.target_date_resolution = Some(tr);
                }
                tx.initiatives().update(cur.id, &patch, now).await?;
                let owner_changed = patch.owner_id.is_some_and(|o| o != cur.owner_id);
                emit(
                    &mut *tx,
                    ctx,
                    DomainEvent::InitiativeUpdated {
                        initiative_id: cur.id,
                        changed: if owner_changed {
                            vec!["ownerId".to_owned()]
                        } else {
                            vec![]
                        },
                        previous_owner_id: owner_changed.then_some(cur.owner_id).flatten(),
                    },
                )
                .await?;
                cur.id
            }
        };

        // Eltern (append-only) unter Sperre des Graphen.
        if !input.parent_initiatives.is_empty() {
            tx.initiatives().lock_hierarchy().await?;
            let all = tx.initiatives().list_all().await?;
            let ident = |i: InitiativeId| {
                all.iter()
                    .find(|x| x.id == i)
                    .map(|x| x.identifier())
                    .unwrap_or_else(|| i.to_string())
            };
            let mut edges = tx.initiatives().list_edges().await?;
            let limit = max_depth(&limits);
            for r in &input.parent_initiatives {
                let parent = resolve::initiative::resolve_in(&mut *tx, &snap, r).await?;
                graph::check_add_parent(&edges, id, parent.id, limit)
                    .map_err(|e| graph_error(e, ident))?;
                let sort = tx.initiatives().max_child_sort_order(parent.id).await? + 1.0;
                if tx
                    .initiatives()
                    .insert_edge(parent.id, id, sort, now)
                    .await?
                {
                    edges.push((parent.id, id));
                    emit(
                        &mut *tx,
                        ctx,
                        DomainEvent::InitiativeUpdated {
                            initiative_id: parent.id,
                            changed: vec![],
                            previous_owner_id: None,
                        },
                    )
                    .await?;
                }
            }
        }
        if let Some(refs) = &input.labels {
            let existing = tx.initiatives().labels_of(id).await?;
            let rows = resolve_labels(&mut *tx, refs, &existing).await?;
            tx.initiatives().set_labels(id, &rows, now).await?;
        }
        let i = tx
            .initiatives()
            .get(id)
            .await?
            .ok_or_else(|| AppError::internal("initiative missing after save"))?;
        let mut v = self.initiative_views(&mut *tx, &snap, vec![i], now).await?;
        tx.commit().await?;
        v.pop()
            .ok_or_else(|| AppError::internal("initiative view missing"))
    }

    /// Entfernt eine Eltern-Beziehung (nicht über MCP erreichbar; Web-App/API).
    pub async fn remove_initiative_parent(
        &self,
        ctx: &RequestCtx,
        initiative: &str,
        parent: &str,
    ) -> AppResult<bool> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        require_enabled(&mut *tx).await?;
        let child = resolve::initiative::resolve_in(&mut *tx, &snap, initiative).await?;
        let p = resolve::initiative::resolve_in(&mut *tx, &snap, parent).await?;
        tx.initiatives().lock_hierarchy().await?;
        let removed = tx.initiatives().delete_edge(p.id, child.id).await?;
        if removed {
            emit(
                &mut *tx,
                ctx,
                DomainEvent::InitiativeUpdated {
                    initiative_id: p.id,
                    changed: vec![],
                    previous_owner_id: None,
                },
            )
            .await?;
            emit(
                &mut *tx,
                ctx,
                DomainEvent::InitiativeUpdated {
                    initiative_id: child.id,
                    changed: vec![],
                    previous_owner_id: None,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(removed)
    }

    /// Ordnet ein Projekt einer Initiative zu bzw. löst es (manuell, n:m).
    pub async fn set_initiative_project(
        &self,
        ctx: &RequestCtx,
        initiative: &str,
        project: &str,
        linked: bool,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        require_enabled(&mut *tx).await?;
        let i = resolve::initiative::resolve_in(&mut *tx, &snap, initiative).await?;
        let p = resolve::project::resolve_in(&mut *tx, &snap, project).await?;
        let now = self.clock.now();
        let changed = if linked {
            tx.initiatives()
                .link_project(i.id, p.project.id, now)
                .await?
        } else {
            tx.initiatives().unlink_project(i.id, p.project.id).await?
        };
        if changed {
            emit(
                &mut *tx,
                ctx,
                DomainEvent::InitiativeUpdated {
                    initiative_id: i.id,
                    changed: vec![],
                    previous_owner_id: None,
                },
            )
            .await?;
            emit(
                &mut *tx,
                ctx,
                DomainEvent::ProjectUpdated {
                    project_id: p.project.id,
                    changed: vec![],
                    previous_lead_id: None,
                },
            )
            .await?;
        }
        tx.commit().await
    }

    /// Archiviert eine Initiative bzw. stellt sie wieder her.
    pub async fn set_initiative_archived(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        archived: bool,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        let i = resolve::initiative::resolve_in(&mut *tx, &snap, reference).await?;
        let now = self.clock.now();
        tx.initiatives()
            .update(
                i.id,
                &InitiativePatch {
                    archived_at: Some(archived.then_some(now)),
                    ..Default::default()
                },
                now,
            )
            .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::InitiativeUpdated {
                initiative_id: i.id,
                changed: vec![],
                previous_owner_id: None,
            },
        )
        .await?;
        tx.commit().await
    }

    /// Löscht eine Initiative (Papierkorb).
    pub async fn trash_initiative(&self, ctx: &RequestCtx, reference: &str) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        let i = resolve::initiative::resolve_in(&mut *tx, &snap, reference).await?;
        let now = self.clock.now();
        tx.initiatives()
            .update(
                i.id,
                &InitiativePatch {
                    trashed: Some(true),
                    ..Default::default()
                },
                now,
            )
            .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::InitiativeRemoved {
                initiative_id: i.id,
            },
        )
        .await?;
        tx.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parsing_accepts_name_id_and_type() {
        assert_eq!(
            parse_initiative_status("Active"),
            Some(InitiativeStatus::Active)
        );
        assert_eq!(
            parse_initiative_status("started"),
            Some(InitiativeStatus::Active)
        );
        assert_eq!(
            parse_initiative_status("backlog"),
            Some(InitiativeStatus::Proposed)
        );
        assert_eq!(
            parse_initiative_status("CANCELED"),
            Some(InitiativeStatus::Canceled)
        );
        assert_eq!(parse_initiative_status("nope"), None);
        assert_eq!(
            initiative_status_ref(InitiativeStatus::Active),
            ("active", "Active", "started")
        );
    }

    #[test]
    fn depth_limit_defaults_to_five() {
        assert_eq!(max_depth(&json!({})), 5);
        assert_eq!(max_depth(&json!({ "maxInitiativeDepth": 3 })), 3);
    }

    #[test]
    fn cycle_error_lists_the_path() {
        let (a, b, c) = (
            InitiativeId::new_v7(),
            InitiativeId::new_v7(),
            InitiativeId::new_v7(),
        );
        let name = |i: InitiativeId| {
            if i == a {
                "I-1".to_string()
            } else if i == b {
                "I-2".to_string()
            } else {
                assert_eq!(i, c);
                "I-3".to_string()
            }
        };
        let e = graph_error(graph::GraphError::Cycle(vec![a, b, c, a]), name);
        assert_eq!(e.kind, crate::error::ErrorKind::InvalidRequest);
        assert_eq!(e.details["reason"], "cycle");
        assert_eq!(e.details["cycle"], json!(["I-1", "I-2", "I-3", "I-1"]));
    }
}
