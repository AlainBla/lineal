//! usecases – Views (docs/05 §2–4): Custom Views (personal/Team/Workspace), Anzeigeoptionen,
//! Favoriten und die Anwendung einer View auf Listen (`customView`).
//!
//! Sicherheitsregeln:
//! * Unsichtbare Views existieren nicht (`not_found`, nie `forbidden`) – die Sichtbarkeit steht in
//!   jeder Abfrage des Repositories ([`ViewAccess`]).
//! * Schreiben: Owner; geteilte Team-Views zusätzlich Team-Owner und Admins; Workspace-Views
//!   Admins (docs/05 §4.2). Gäste dürfen nur persönliche Views in ihren Teams anlegen.
//! * Filter werden beim Speichern validiert und normalisiert (docs/05 §1.8); die Referenzen
//!   müssen für den Speichernden sichtbar sein.

pub mod prefs;

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::user::{Page, PageRequest};
use crate::ports::repos::view::{
    AnyFilter, CustomView, Favorite, FilterEval, NewCustomView, NewFavorite, PrefLevel, SearchHitRow,
    SearchKind, SearchOrder, SearchQuery, ViewAccess, ViewKey, ViewListFilter, ViewModel,
};
use crate::usecases::issue::IssueView;
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::actor_user_id;
use chrono::{DateTime, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{InitiativeId, IssueId, ProjectId, TeamId, UserId, ViewId};
use lineal_domain::rules::filter::{
    self, FilterError, InitiativeFilter, IssueFilter, ProjectFilter,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Längenlimits (docs/05 §4.1).
pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_DESCRIPTION_CHARS: usize = 255;

/// Eine Ansicht samt Rechten des Betrachters.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewInfo {
    pub view: CustomView,
    /// Darf der Betrachter bearbeiten/löschen (docs/05 §4.2)?
    pub can_edit: bool,
}

/// Eingabe für `create_view`.
#[derive(Debug, Clone)]
pub struct CreateViewInput {
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub model: ViewModel,
    /// Filter-JSON passend zu `model` (`IssueFilter`, `ProjectFilter`, `InitiativeFilter`).
    pub filter: Value,
    /// `true`: geteilt (Workspace, bei `team` im Team).
    pub shared: bool,
    pub team: Option<String>,
    pub project: Option<String>,
    pub initiative: Option<String>,
    /// Anzeigeoptionen zum Zeitpunkt des Anlegens (werden die `organization`-Zeile der View).
    pub preferences: Option<Value>,
}

/// Eingabe für `update_view` (nur gesetzte Felder ändern sich).
#[derive(Debug, Clone, Default)]
pub struct UpdateViewInput {
    pub reference: String,
    /// Optimistic Locking (`CustomView.version`).
    pub expected_version: Option<i32>,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub icon: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub filter: Option<Value>,
    pub shared: Option<bool>,
    /// `Some(None)` = Team entfernen, `Some(Some(ref))` = Team setzen.
    pub team: Option<Option<String>>,
    pub owner: Option<String>,
    pub sort_order: Option<f64>,
    pub archived: Option<bool>,
}

/// Eingabe für `list_views`.
#[derive(Debug, Clone, Default)]
pub struct ListViewsInput {
    pub model: Option<ViewModel>,
    pub team: Option<String>,
    pub shared: Option<bool>,
    pub query: Option<String>,
    pub page: PageRequest,
}

/// Schlüssel einer Ansicht für Anzeigeoptionen, wie ihn Adapter übergeben.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewKeyInput {
    /// Custom View (UUID, Slug oder Name).
    pub custom_view: Option<String>,
    /// Vordefinierte Ansicht (`teamActive`, `myIssuesAssigned` …).
    pub view_type: Option<String>,
    /// Team (Referenz) hinter einer vordefinierten Team-Ansicht.
    pub team: Option<String>,
}

/// Wirksame Anzeigeoptionen einer Ansicht.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewPreferencesView {
    pub key: ViewKey,
    /// `shallowMerge(sys, org, mine)` nach `sanitize`.
    pub effective: Map<String, Value>,
    /// Hat der Aufrufer eigene Overrides?
    pub has_personal: bool,
    /// Gibt es einen Workspace-/Team-Default?
    pub has_default: bool,
}

/// Eingabe für `create_favorite`.
#[derive(Debug, Clone, Default)]
pub struct CreateFavoriteInput {
    /// `folder`, `issue`, `project`, `cycle`, `customView`, `predefinedView`, `document`,
    /// `initiative`, `label`, `projectLabel`, `team`, `user`.
    pub kind: String,
    /// Ziel (UUID; bei `customView` auch Slug/Name; bei `issue` auch Kennung).
    pub target: Option<String>,
    pub folder_name: Option<String>,
    pub parent: Option<Uuid>,
    pub predefined_view_type: Option<String>,
    pub predefined_view_team: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------------------------

fn filter_error(e: FilterError) -> AppError {
    AppError::invalid_request(format!("Invalid filter: {}", e.reason))
        .with_details(json!({ "path": e.path, "reason": e.reason }))
}

fn view_not_found(reference: &str) -> AppError {
    AppError::not_found(format!("Could not find custom view \"{reference}\""))
        .with_details(json!({ "reference": reference, "type": "CustomView" }))
}

fn require_write(ctx: &RequestCtx) -> AppResult<()> {
    if ctx.scopes.write {
        Ok(())
    } else {
        Err(AppError::forbidden("This token has no write access.")
            .with_details(json!({ "reason": "scope" })))
    }
}

fn invalid(field: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field }))
}

fn clean_name(name: &str) -> AppResult<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > MAX_NAME_CHARS {
        return Err(invalid("name", "name must be 1-80 characters"));
    }
    Ok(n.to_owned())
}

fn clean_description(d: &str) -> AppResult<Option<String>> {
    let d = d.trim();
    if d.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(invalid("description", "description must be at most 255 characters"));
    }
    Ok((!d.is_empty()).then(|| d.to_owned()))
}

fn clean_icon(i: &str) -> AppResult<Option<String>> {
    let i = i.trim();
    if i.is_empty() {
        return Ok(None);
    }
    let ok = i.len() <= 64
        && i.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-'));
    if ok {
        Ok(Some(i.to_owned()))
    } else {
        Err(invalid("icon", "icon must be an icon name or emoji code"))
    }
}

fn clean_color(c: &str) -> AppResult<Option<String>> {
    let c = c.trim();
    if c.is_empty() {
        return Ok(None);
    }
    let ok = c.len() == 7
        && c.starts_with('#')
        && c[1..].chars().all(|x| x.is_ascii_hexdigit());
    if ok {
        Ok(Some(c.to_owned()))
    } else {
        Err(invalid("color", "color must match #RRGGBB"))
    }
}

/// Parst, validiert und normalisiert einen Filter für `model`.
/// Liefert den typisierten Filter und die gespeicherte (normalisierte) JSON-Form.
pub fn parse_view_filter(model: ViewModel, json: &Value) -> AppResult<(AnyFilter, Value)> {
    match model {
        ViewModel::Issue => {
            let f: IssueFilter = filter::from_json_value(json).map_err(filter_error)?;
            filter::validate(&f).map_err(filter_error)?;
            let norm = filter::normalize_json(serde_json::to_value(&f).unwrap_or(Value::Null));
            let f: IssueFilter = filter::from_json_value(&norm).map_err(filter_error)?;
            Ok((AnyFilter::Issue(f), norm))
        }
        ViewModel::Project => {
            let f: ProjectFilter = filter::from_json_value(json).map_err(filter_error)?;
            filter::validate_project(&f).map_err(filter_error)?;
            let norm = filter::normalize_json(serde_json::to_value(&f).unwrap_or(Value::Null));
            let f: ProjectFilter = filter::from_json_value(&norm).map_err(filter_error)?;
            Ok((AnyFilter::Project(f), norm))
        }
        ViewModel::Initiative => {
            let f: InitiativeFilter = filter::from_json_value(json).map_err(filter_error)?;
            filter::validate_initiative(&f).map_err(filter_error)?;
            let norm = filter::normalize_json(serde_json::to_value(&f).unwrap_or(Value::Null));
            let f: InitiativeFilter = filter::from_json_value(&norm).map_err(filter_error)?;
            Ok((AnyFilter::Initiative(f), norm))
        }
    }
}

/// Wer handelt (Views sind persönliche Daten: immer ein User).
pub(crate) fn view_access(snap: &PolicySnapshot, ctx: &RequestCtx) -> AppResult<ViewAccess> {
    let user_id = actor_user_id(ctx).ok_or_else(|| {
        AppError::forbidden("Views require a user.").with_details(json!({ "reason": "actor" }))
    })?;
    Ok(ViewAccess {
        user_id,
        is_guest: snap.actor.role == UserRole::Guest,
        visible_teams: snap.visible_team_ids().into_iter().collect(),
    })
}

/// Baut den Auswertungskontext für Filter (`strict`: Referenzen müssen sichtbar existieren).
pub(crate) async fn build_eval(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    now: DateTime<Utc>,
    strict: bool,
    with_initiative_edges: bool,
) -> AppResult<FilterEval> {
    let me = actor_user_id(ctx);
    let timezone = match me {
        Some(u) => tx
            .users()
            .get(u)
            .await?
            .and_then(|u| u.timezone)
            .unwrap_or_else(|| "UTC".into()),
        None => "UTC".into(),
    };
    let visible = snap.visible_team_ids();
    let mut descendants: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    for t in &visible {
        let d: Vec<Uuid> = snap
            .tree
            .descendants(*t)
            .iter()
            .map(|x| x.id)
            .filter(|id| visible.contains(id))
            .map(|id| id.0)
            .collect();
        descendants.insert(t.0, d);
    }
    let initiative_edges = if with_initiative_edges {
        tx.initiatives()
            .list_edges()
            .await?
            .into_iter()
            .map(|(p, c)| (p.0, c.0))
            .collect()
    } else {
        Vec::new()
    };
    Ok(FilterEval {
        org: ctx.org_id,
        is_guest: snap.actor.role == UserRole::Guest,
        now,
        timezone,
        me,
        visible_teams: visible.into_iter().collect(),
        descendants,
        initiative_edges,
        strict_refs: strict,
    })
}

/// Kandidaten für eine View-Referenz: roh, letztes URL-Segment, Slug am Ende (`name-<slug>`).
fn view_ref_candidates(reference: &str) -> Vec<String> {
    let raw = reference.trim();
    let mut out = vec![raw.to_owned()];
    let seg = raw
        .split(['?', '#'])
        .next()
        .unwrap_or(raw)
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(raw)
        .to_owned();
    if seg != raw && !seg.is_empty() {
        out.push(seg.clone());
    }
    if let Some((_, tail)) = seg.rsplit_once('-')
        && tail.len() == 12
        && tail.bytes().all(|b| b.is_ascii_hexdigit())
    {
        out.push(tail.to_lowercase());
    }
    out
}

/// Löst eine View-Referenz auf (UUID, `slugId`, URL, exakter Name). Unsichtbare Views und
/// nicht vorhandene → `not_found`; mehrere Namenstreffer → `ambiguous_reference`.
pub(crate) async fn resolve_view_in(
    tx: &mut dyn Tx,
    access: &ViewAccess,
    reference: &str,
) -> AppResult<CustomView> {
    for cand in view_ref_candidates(reference) {
        let mut found = tx.views().find_views(access, &cand).await?;
        match found.len() {
            0 => continue,
            1 => return Ok(found.remove(0)),
            _ => {
                return Err(AppError::ambiguous_reference(format!(
                    "Custom view \"{reference}\" matches several views; use the ID or slug."
                ))
                .with_details(json!({
                    "reference": reference,
                    "type": "CustomView",
                    "candidates": found.iter().take(10).map(|v| json!({
                        "id": v.id.to_string(), "name": v.name, "slugId": v.slug_id
                    })).collect::<Vec<_>>(),
                })));
            }
        }
    }
    Err(view_not_found(reference))
}

/// Darf der Betrachter die View bearbeiten/löschen? (docs/05 §4.2)
fn can_edit(snap: &PolicySnapshot, me: UserId, v: &CustomView) -> bool {
    if v.owner_id == me {
        return true;
    }
    if !v.shared {
        return false;
    }
    if snap.actor.is_admin() {
        return true;
    }
    v.team_id
        .and_then(|t| snap.view(t).ok())
        .is_some_and(|p| p.is_team_owner)
}

fn require_edit(snap: &PolicySnapshot, me: UserId, v: &CustomView) -> AppResult<()> {
    if can_edit(snap, me, v) {
        Ok(())
    } else {
        Err(
            AppError::forbidden("You cannot change this custom view.").with_details(json!({
                "action": "UpdateCustomView",
                "reason": "role"
            })),
        )
    }
}

/// Scope-Bedingungen einer View als Filter-JSON (docs/05 §4.1): Team-Views sind auf ihr Team
/// beschränkt, auch wenn der Filter kein Teamkriterium enthält (ohne Sub-Teams, außer der
/// Filter nennt `ancestors`); Projekt-Tabs auf ihr Projekt.
pub fn scope_conditions(v: &CustomView) -> Vec<Value> {
    let mut out = Vec::new();
    match v.model {
        ViewModel::Issue => {
            if let Some(t) = v.team_id {
                out.push(json!({ "team": { "id": { "in": [t.to_string()] } } }));
            }
            if let Some(p) = v.project_id {
                out.push(json!({ "project": { "id": { "in": [p.to_string()] } } }));
            }
        }
        ViewModel::Project => {
            if let Some(t) = v.team_id {
                out.push(
                    json!({ "teams": { "some": { "id": { "in": [t.to_string()] } } } }),
                );
            }
        }
        ViewModel::Initiative => {
            if let Some(t) = v.team_id {
                out.push(json!({ "leadTeam": { "id": { "in": [t.to_string()] } } }));
            }
        }
    }
    out
}

/// Die View als Filter-JSON `{ and: [view, scope…, extra…] }` für die Anwendung auf Listen.
pub fn combined_filter(v: &CustomView, extra: Vec<Value>) -> Value {
    let mut parts = vec![v.filter.clone()];
    parts.extend(scope_conditions(v));
    parts.extend(extra);
    json!({ "and": parts })
}

/// Lädt die View einer Liste (`customView`) und prüft den Typ (`modelName`).
pub(crate) async fn view_for_list(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    reference: &str,
    model: ViewModel,
) -> AppResult<CustomView> {
    let access = view_access(snap, ctx)?;
    let v = resolve_view_in(tx, &access, reference).await?;
    if v.model != model {
        return Err(AppError::invalid_request(format!(
            "Custom view \"{}\" shows {}s, not {}s.",
            v.name,
            v.model.as_str().to_lowercase(),
            model.as_str().to_lowercase()
        ))
        .with_details(json!({
            "field": "customView",
            "reason": "model_mismatch",
            "expected": model.as_str(),
            "actual": v.model.as_str(),
        })));
    }
    Ok(v)
}

async fn emit(tx: &mut dyn Tx, ctx: &RequestCtx, event: DomainEvent) -> AppResult<()> {
    tx.outbox().push(NewEvent::from_ctx(event, ctx)).await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Use-Cases: Custom Views
// ---------------------------------------------------------------------------------------------

impl App {
    /// Legt eine Custom View an (docs/05 §4.5).
    pub async fn create_view(
        &self,
        ctx: &RequestCtx,
        input: CreateViewInput,
    ) -> AppResult<ViewInfo> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let now = self.clock.now();
        let name = clean_name(&input.name)?;
        let description = match input.description.as_deref() {
            Some(d) => clean_description(d)?,
            None => None,
        };
        let icon = match input.icon.as_deref() {
            Some(i) => clean_icon(i)?,
            None => None,
        };
        let color = match input.color.as_deref() {
            Some(c) => clean_color(c)?,
            None => None,
        };
        let (any, norm) = parse_view_filter(input.model, &input.filter)?;
        let eval = build_eval(&mut *tx, &snap, ctx, now, true, true).await?;
        tx.views().verify_filter_refs(&any, &eval).await?;

        if [
            input.team.is_some(),
            input.project.is_some(),
            input.initiative.is_some(),
        ]
        .iter()
        .filter(|b| **b)
        .count()
            > 1
        {
            return Err(invalid(
                "scope",
                "a view can be scoped to one of team, project or initiative",
            ));
        }
        let mut team_id: Option<TeamId> = None;
        let mut project_id: Option<ProjectId> = None;
        let mut initiative_id: Option<InitiativeId> = None;
        if let Some(t) = &input.team {
            let team = resolve::team::resolve_in(&mut *tx, &snap, t, Visibility::Content).await?;
            team_id = Some(team.id);
        }
        if let Some(p) = &input.project {
            if access.is_guest {
                return Err(guest_scope_error());
            }
            let rp = resolve::project::resolve_in(&mut *tx, &snap, p).await?;
            if input.model != ViewModel::Issue {
                return Err(invalid("project", "only issue views can be scoped to a project"));
            }
            project_id = Some(rp.project.id);
        }
        if let Some(i) = &input.initiative {
            if access.is_guest {
                return Err(guest_scope_error());
            }
            let ini = resolve::initiative::resolve_in(&mut *tx, &snap, i).await?;
            initiative_id = Some(ini.id);
        }
        // Rechte je Art (docs/05 §4.2/§4.4).
        if input.shared {
            if access.is_guest {
                return Err(guest_scope_error());
            }
            match team_id {
                Some(t) => {
                    snap.require_team(ctx, Action::ManageTeamSharedViews, t)?;
                    if !snap.actor.is_member(t) {
                        return Err(AppError::forbidden(
                            "Only team members can share a view with the team.",
                        )
                        .with_details(json!({ "action": "ManageTeamSharedViews", "reason": "membership" })));
                    }
                }
                None if project_id.is_none() && initiative_id.is_none() => {
                    snap.require_workspace(ctx, Action::ManageWorkspaceViews)?;
                }
                None => {}
            }
        }
        let preferences = match &input.preferences {
            Some(p) => Some(prefs::validate_patch(
                p,
                kind_of_model(input.model),
                None,
            )?),
            None => None,
        };
        let sort_order = 0.0;
        let id = ViewId(self.ids.new_uuid());
        let view = tx
            .views()
            .insert_view(&NewCustomView {
                id,
                name,
                description,
                icon,
                color,
                model: input.model,
                filter: norm,
                owner_id: access.user_id,
                creator_id: access.user_id,
                shared: input.shared,
                team_id,
                project_id,
                initiative_id,
                sort_order,
                now,
            })
            .await?;
        if let Some(p) = preferences {
            let key = ViewKey {
                custom_view_id: Some(view.id),
                view_type: None,
                view_target_id: None,
            };
            tx.views()
                .put_preferences(&key, PrefLevel::Organization, None, &Value::Object(p), now)
                .await?;
        }
        emit(&mut *tx, ctx, DomainEvent::CustomViewCreated { view_id: view.id }).await?;
        tx.commit().await?;
        Ok(ViewInfo {
            can_edit: true,
            view,
        })
    }

    /// Liest eine sichtbare View.
    pub async fn get_view(&self, ctx: &RequestCtx, reference: &str) -> AppResult<ViewInfo> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let view = resolve_view_in(&mut *tx, &access, reference).await?;
        Ok(ViewInfo {
            can_edit: can_edit(&snap, access.user_id, &view),
            view,
        })
    }

    /// Sichtbare, nicht archivierte Views (Metadaten); keine an Projekte/Initiativen gebundenen.
    pub async fn list_views(
        &self,
        ctx: &RequestCtx,
        input: ListViewsInput,
    ) -> AppResult<Page<ViewInfo>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let team = match &input.team {
            Some(t) => Some(
                resolve::team::resolve_in(&mut *tx, &snap, t, Visibility::Content)
                    .await?
                    .id,
            ),
            None => None,
        };
        let page = PageRequest {
            limit: input.page.limit.clamp(1, 250),
            ..input.page
        };
        let res = tx
            .views()
            .list_views(
                &access,
                &ViewListFilter {
                    model: input.model,
                    team,
                    shared: input.shared,
                    query: input.query.clone(),
                    include_scoped: false,
                },
                &page,
            )
            .await?;
        let me = access.user_id;
        Ok(Page {
            items: res
                .items
                .into_iter()
                .map(|view| ViewInfo {
                    can_edit: can_edit(&snap, me, &view),
                    view,
                })
                .collect(),
            next: res.next,
        })
    }

    /// Ändert eine View (Name, Filter, Scope, Besitzer …; `modelName` ist unveränderlich).
    pub async fn update_view(
        &self,
        ctx: &RequestCtx,
        input: UpdateViewInput,
    ) -> AppResult<ViewInfo> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let now = self.clock.now();
        let mut view = resolve_view_in(&mut *tx, &access, &input.reference).await?;
        require_edit(&snap, access.user_id, &view)?;
        if let Some(v) = input.expected_version
            && v != view.version
        {
            return Err(AppError::conflict("The custom view was changed by someone else.")
                .with_details(json!({ "expectedVersion": v, "currentVersion": view.version })));
        }
        if let Some(n) = &input.name {
            view.name = clean_name(n)?;
        }
        if let Some(d) = &input.description {
            view.description = match d {
                Some(d) => clean_description(d)?,
                None => None,
            };
        }
        if let Some(i) = &input.icon {
            view.icon = match i {
                Some(i) => clean_icon(i)?,
                None => None,
            };
        }
        if let Some(c) = &input.color {
            view.color = match c {
                Some(c) => clean_color(c)?,
                None => None,
            };
        }
        if let Some(f) = &input.filter {
            let (any, norm) = parse_view_filter(view.model, f)?;
            let eval = build_eval(&mut *tx, &snap, ctx, now, true, true).await?;
            tx.views().verify_filter_refs(&any, &eval).await?;
            view.filter = norm;
        }
        if let Some(s) = input.sort_order {
            if !s.is_finite() {
                return Err(invalid("sortOrder", "sortOrder must be a finite number"));
            }
            view.sort_order = s;
        }
        // Scope („Save to“: Personal, Workspace, Team).
        if input.shared.is_some() || input.team.is_some() {
            if view.project_id.is_some() || view.initiative_id.is_some() {
                return Err(invalid("shared", "the scope of a project or initiative tab cannot be changed"));
            }
            let shared = input.shared.unwrap_or(view.shared);
            let team = match &input.team {
                Some(Some(t)) => Some(
                    resolve::team::resolve_in(&mut *tx, &snap, t, Visibility::Content)
                        .await?
                        .id,
                ),
                Some(None) => None,
                None => view.team_id,
            };
            // Ziel prüfen wie beim Anlegen; nur der Owner verschiebt den Scope.
            if view.owner_id != access.user_id && !snap.actor.is_admin() {
                return Err(AppError::forbidden("Only the owner can change the scope of a view.")
                    .with_details(json!({ "action": "UpdateCustomView", "reason": "owner" })));
            }
            if shared {
                if access.is_guest {
                    return Err(guest_scope_error());
                }
                match team {
                    Some(t) => {
                        snap.require_team(ctx, Action::ManageTeamSharedViews, t)?;
                        if view.owner_id == access.user_id && !snap.actor.is_member(t) {
                            return Err(AppError::forbidden(
                                "Only team members can share a view with the team.",
                            )
                            .with_details(json!({ "action": "ManageTeamSharedViews", "reason": "membership" })));
                        }
                    }
                    None => snap.require_workspace(ctx, Action::ManageWorkspaceViews)?,
                }
            }
            view.shared = shared;
            view.team_id = team;
        }
        if let Some(o) = &input.owner {
            let new_owner = resolve::user::resolve_in(&mut *tx, &snap, ctx, o).await?;
            if !view.shared {
                return Err(invalid("owner", "a personal view cannot be transferred"));
            }
            let guest = new_owner.role == UserRole::Guest;
            let sees = match view.team_id {
                Some(t) => tx
                    .teams()
                    .list_memberships_of_user(new_owner.id)
                    .await?
                    .iter()
                    .any(|m| m.team_id == t),
                None => !guest,
            };
            if !sees {
                return Err(invalid("owner", "the new owner must be able to see the view"));
            }
            view.owner_id = new_owner.id;
        }
        if let Some(a) = input.archived {
            view.archived_at = if a { Some(now) } else { None };
        }
        let saved = tx.views().save_view(&view, access.user_id, now).await?;
        emit(&mut *tx, ctx, DomainEvent::CustomViewUpdated { view_id: saved.id }).await?;
        let can = can_edit(&snap, access.user_id, &saved);
        tx.commit().await?;
        Ok(ViewInfo {
            view: saved,
            can_edit: can,
        })
    }

    /// Löscht eine View endgültig (inkl. Preferences und Favoriten).
    pub async fn delete_view(&self, ctx: &RequestCtx, reference: &str) -> AppResult<ViewId> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let view = resolve_view_in(&mut *tx, &access, reference).await?;
        require_edit(&snap, access.user_id, &view)?;
        tx.views().delete_view(view.id).await?;
        emit(&mut *tx, ctx, DomainEvent::CustomViewRemoved { view_id: view.id }).await?;
        tx.commit().await?;
        Ok(view.id)
    }

    /// Dupliziert eine sichtbare View: Kopie gehört dem Duplizierenden, `shared = false`,
    /// Name „<Name> (copy)“ (docs/05 §4.2/§4.4).
    pub async fn duplicate_view(&self, ctx: &RequestCtx, reference: &str) -> AppResult<ViewInfo> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let now = self.clock.now();
        let src = resolve_view_in(&mut *tx, &access, reference).await?;
        let mut name = format!("{} (copy)", src.name);
        if name.chars().count() > MAX_NAME_CHARS {
            name = name.chars().take(MAX_NAME_CHARS).collect();
        }
        let view = tx
            .views()
            .insert_view(&NewCustomView {
                id: ViewId(self.ids.new_uuid()),
                name,
                description: src.description.clone(),
                icon: src.icon.clone(),
                color: src.color.clone(),
                model: src.model,
                filter: src.filter.clone(),
                owner_id: access.user_id,
                creator_id: access.user_id,
                shared: false,
                team_id: src.team_id,
                project_id: None,
                initiative_id: None,
                sort_order: src.sort_order,
                now,
            })
            .await?;
        let src_key = ViewKey {
            custom_view_id: Some(src.id),
            view_type: None,
            view_target_id: None,
        };
        let (org_prefs, _) = tx.views().get_preferences(&src_key, access.user_id).await?;
        if let Some(p) = org_prefs {
            let key = ViewKey {
                custom_view_id: Some(view.id),
                view_type: None,
                view_target_id: None,
            };
            tx.views()
                .put_preferences(&key, PrefLevel::Organization, None, &p, now)
                .await?;
        }
        emit(&mut *tx, ctx, DomainEvent::CustomViewCreated { view_id: view.id }).await?;
        tx.commit().await?;
        Ok(ViewInfo {
            can_edit: true,
            view,
        })
    }
}

fn guest_scope_error() -> AppError {
    AppError::forbidden("Guests can only create personal views in their own teams.")
        .with_details(json!({ "action": "CreateCustomView", "reason": "role" }))
}

fn kind_of_model(m: ViewModel) -> prefs::Kind {
    match m {
        ViewModel::Issue => prefs::Kind::Issues,
        ViewModel::Project => prefs::Kind::Projects,
        ViewModel::Initiative => prefs::Kind::Initiatives,
    }
}

// ---------------------------------------------------------------------------------------------
// Use-Cases: Anzeigeoptionen (docs/05 §2.2)
// ---------------------------------------------------------------------------------------------

/// Aufgelöster Schlüssel samt Kontext für die Rechteprüfung.
struct ResolvedKey {
    key: ViewKey,
    kind: prefs::Kind,
    view_type: Option<String>,
    team: Option<TeamId>,
    view: Option<CustomView>,
}

async fn resolve_key(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    access: &ViewAccess,
    input: &ViewKeyInput,
) -> AppResult<ResolvedKey> {
    match (&input.custom_view, &input.view_type) {
        (Some(r), None) => {
            let v = resolve_view_in(tx, access, r).await?;
            Ok(ResolvedKey {
                key: ViewKey {
                    custom_view_id: Some(v.id),
                    view_type: None,
                    view_target_id: None,
                },
                kind: kind_of_model(v.model),
                view_type: None,
                team: v.team_id,
                view: Some(v),
            })
        }
        (None, Some(t)) => {
            let kind = prefs::kind_of_view_type(t).ok_or_else(|| {
                invalid("viewType", format!("unknown view type `{t}`"))
            })?;
            let team = match &input.team {
                Some(r) => Some(
                    resolve::team::resolve_in(tx, snap, r, Visibility::Content)
                        .await?
                        .id,
                ),
                None => None,
            };
            if t.starts_with("team") && team.is_none() && !matches!(t.as_str(), "teamProjects") {
                return Err(invalid("team", "this view type needs a team"));
            }
            let _ = ctx;
            Ok(ResolvedKey {
                key: ViewKey {
                    custom_view_id: None,
                    view_type: Some(t.clone()),
                    view_target_id: team.map(|x| x.0),
                },
                kind,
                view_type: Some(t.clone()),
                team,
                view: None,
            })
        }
        _ => Err(invalid(
            "customView",
            "pass exactly one of customView and viewType",
        )),
    }
}

async fn load_effective(
    tx: &mut dyn Tx,
    rk: &ResolvedKey,
    me: UserId,
) -> AppResult<ViewPreferencesView> {
    let (org, mine) = tx.views().get_preferences(&rk.key, me).await?;
    let sys = prefs::system_defaults(rk.view_type.as_deref(), rk.kind);
    let effective = prefs::effective(sys, org.as_ref(), mine.as_ref());
    Ok(ViewPreferencesView {
        key: rk.key.clone(),
        effective,
        has_personal: mine.is_some(),
        has_default: org.is_some(),
    })
}

impl App {
    /// Wirksame Anzeigeoptionen (`viewPreferencesValues`).
    pub async fn view_preferences(
        &self,
        ctx: &RequestCtx,
        input: ViewKeyInput,
    ) -> AppResult<ViewPreferencesView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let rk = resolve_key(&mut *tx, &snap, ctx, &access, &input).await?;
        load_effective(&mut *tx, &rk, access.user_id).await
    }

    /// Persönliche Änderung (Upsert der `user`-Zeile, nur geänderte Schlüssel).
    pub async fn update_view_preferences(
        &self,
        ctx: &RequestCtx,
        input: ViewKeyInput,
        patch: Value,
    ) -> AppResult<ViewPreferencesView> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let now = self.clock.now();
        let rk = resolve_key(&mut *tx, &snap, ctx, &access, &input).await?;
        let changes = prefs::validate_patch(&patch, rk.kind, rk.view_type.as_deref())?;
        let (_, mine) = tx.views().get_preferences(&rk.key, access.user_id).await?;
        let mut merged = mine
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        for (k, v) in changes {
            merged.insert(k, v);
        }
        tx.views()
            .put_preferences(
                &rk.key,
                PrefLevel::User,
                Some(access.user_id),
                &Value::Object(merged),
                now,
            )
            .await?;
        let out = load_effective(&mut *tx, &rk, access.user_id).await?;
        tx.commit().await?;
        Ok(out)
    }

    /// „Set default for everyone“: schreibt die wirksamen Optionen in die `organization`-Zeile
    /// und löscht die persönliche Zeile des Ausführenden (docs/05 §2.2 Regel 2).
    pub async fn set_view_preferences_default(
        &self,
        ctx: &RequestCtx,
        input: ViewKeyInput,
    ) -> AppResult<ViewPreferencesView> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let now = self.clock.now();
        let rk = resolve_key(&mut *tx, &snap, ctx, &access, &input).await?;
        let allowed = match (&rk.view, rk.team) {
            (Some(v), _) => can_edit(&snap, access.user_id, v) && (v.shared || v.owner_id == access.user_id),
            (None, Some(t)) => snap.view(t).is_ok_and(|p| p.is_team_owner) || snap.actor.is_admin(),
            (None, None) => snap.actor.is_admin(),
        };
        if !allowed {
            return Err(AppError::forbidden("You cannot set the default for everyone.")
                .with_details(json!({ "action": "SetViewDefault", "reason": "role" })));
        }
        let current = load_effective(&mut *tx, &rk, access.user_id).await?;
        tx.views()
            .put_preferences(
                &rk.key,
                PrefLevel::Organization,
                None,
                &Value::Object(current.effective),
                now,
            )
            .await?;
        tx.views()
            .delete_preferences(&rk.key, PrefLevel::User, Some(access.user_id))
            .await?;
        let out = load_effective(&mut *tx, &rk, access.user_id).await?;
        tx.commit().await?;
        Ok(out)
    }

    /// „Reset“: löscht die persönliche Zeile.
    pub async fn reset_view_preferences(
        &self,
        ctx: &RequestCtx,
        input: ViewKeyInput,
    ) -> AppResult<ViewPreferencesView> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let rk = resolve_key(&mut *tx, &snap, ctx, &access, &input).await?;
        tx.views()
            .delete_preferences(&rk.key, PrefLevel::User, Some(access.user_id))
            .await?;
        let out = load_effective(&mut *tx, &rk, access.user_id).await?;
        tx.commit().await?;
        Ok(out)
    }
}

// ---------------------------------------------------------------------------------------------
// Use-Cases: Favoriten (docs/05 §4.6)
// ---------------------------------------------------------------------------------------------

const FAVORITE_KINDS: [&str; 12] = [
    "folder",
    "issue",
    "project",
    "cycle",
    "customView",
    "predefinedView",
    "document",
    "initiative",
    "label",
    "projectLabel",
    "team",
    "user",
];

impl App {
    /// Favoriten des Aufrufers (nur sichtbare Ziele; unsichtbare sind ausgeblendet).
    pub async fn list_favorites(&self, ctx: &RequestCtx) -> AppResult<Vec<Favorite>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let all = tx.views().list_favorites(access.user_id).await?;
        let mut out = Vec::new();
        for f in all {
            let visible = match (f.kind.as_str(), f.target_id) {
                ("folder", _) => true,
                ("predefinedView", _) => f
                    .predefined_view_team_id
                    .is_none_or(|t| access.visible_teams.contains(&t)),
                ("customView", Some(id)) => tx
                    .views()
                    .get_view(&access, ViewId(id))
                    .await?
                    .is_some(),
                (k, Some(id)) => tx.views().favorite_target_visible(k, id, &access).await?,
                _ => false,
            };
            if visible {
                out.push(f);
            }
        }
        Ok(out)
    }

    /// Legt einen Favoriten an (idempotent je Ziel).
    pub async fn create_favorite(
        &self,
        ctx: &RequestCtx,
        input: CreateFavoriteInput,
    ) -> AppResult<Favorite> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let now = self.clock.now();
        let kind = input.kind.as_str();
        if !FAVORITE_KINDS.contains(&kind) {
            return Err(invalid("type", format!("unknown favorite type `{kind}`")));
        }
        if let Some(p) = input.parent {
            let parent = tx
                .views()
                .get_favorite(access.user_id, p)
                .await?
                .filter(|f| f.kind == "folder")
                .ok_or_else(|| {
                    AppError::not_found("Could not find favorite folder.")
                        .with_details(json!({ "reference": p.to_string(), "type": "Favorite" }))
                })?;
            // Ordner sind eine Ebene tief.
            if kind == "folder" || parent.parent_id.is_some() {
                return Err(invalid("parentId", "folders are only one level deep"));
            }
        }
        let mut new = NewFavorite {
            id: self.ids.new_uuid(),
            owner_id: access.user_id,
            kind: input.kind.clone(),
            parent_id: input.parent,
            folder_name: None,
            sort_order: 0.0,
            target_id: None,
            predefined_view_type: None,
            predefined_view_team_id: None,
            now,
        };
        match kind {
            "folder" => {
                let n = input.folder_name.as_deref().map(str::trim).unwrap_or("");
                if n.is_empty() || n.chars().count() > MAX_NAME_CHARS {
                    return Err(invalid("folderName", "folderName must be 1-80 characters"));
                }
                new.folder_name = Some(n.to_owned());
            }
            "predefinedView" => {
                let t = input
                    .predefined_view_type
                    .as_deref()
                    .filter(|t| prefs::VIEW_TYPES.contains(t))
                    .ok_or_else(|| invalid("predefinedViewType", "unknown predefined view"))?;
                new.predefined_view_type = Some(t.to_owned());
                if let Some(r) = &input.predefined_view_team {
                    let team =
                        resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Content).await?;
                    new.predefined_view_team_id = Some(team.id);
                }
            }
            "customView" => {
                let r = input
                    .target
                    .as_deref()
                    .ok_or_else(|| invalid("target", "target is required"))?;
                let v = resolve_view_in(&mut *tx, &access, r).await?;
                new.target_id = Some(v.id.0);
            }
            "issue" => {
                let r = input
                    .target
                    .as_deref()
                    .ok_or_else(|| invalid("target", "target is required"))?;
                let issue = resolve::issue::resolve_in(&mut *tx, &snap, ctx, r).await?;
                new.target_id = Some(issue.id.0);
            }
            other => {
                let r = input
                    .target
                    .as_deref()
                    .ok_or_else(|| invalid("target", "target is required"))?;
                let id = Uuid::parse_str(r.trim()).map_err(|_| {
                    invalid("target", "target must be an ID for this favorite type")
                })?;
                if !tx.views().favorite_target_visible(other, id, &access).await? {
                    return Err(AppError::not_found(format!("Could not find {other}."))
                        .with_details(json!({ "reference": r, "type": other })));
                }
                new.target_id = Some(id);
            }
        }
        // Idempotent je Ziel: einen vorhandenen Favoriten zurückgeben.
        if kind != "folder" {
            let existing = tx.views().list_favorites(access.user_id).await?;
            if let Some(f) = existing.into_iter().find(|f| {
                f.kind == new.kind
                    && f.target_id == new.target_id
                    && f.predefined_view_type == new.predefined_view_type
                    && f.predefined_view_team_id == new.predefined_view_team_id
            }) {
                return Ok(f);
            }
        }
        // Neue Favoriten kommen ans Ende der Ebene (docs/05 §4.6).
        let max = tx
            .views()
            .max_favorite_sort_order(access.user_id, input.parent)
            .await?;
        new.sort_order = lineal_domain::rules::sort_order::between(max, None);
        let fav = tx.views().insert_favorite(&new).await?;
        tx.commit().await?;
        Ok(fav)
    }

    /// Entfernt einen eigenen Favoriten (fremde IDs → `not_found`).
    pub async fn delete_favorite(&self, ctx: &RequestCtx, id: Uuid) -> AppResult<()> {
        require_write(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        if !tx.views().delete_favorite(access.user_id, id).await? {
            return Err(AppError::not_found("Could not find referenced Favorite.")
                .with_details(json!({ "reference": id.to_string(), "type": "Favorite" })));
        }
        tx.commit().await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Use-Cases: Suche (docs/05 §5)
// ---------------------------------------------------------------------------------------------

/// Eingabe der globalen Suche.
#[derive(Debug, Clone)]
pub struct SearchInput {
    pub term: String,
    /// Leer = alle Arten.
    pub kinds: Vec<SearchKind>,
    /// Zusatzfilter für Issues (`IssueFilter`-JSON), UND mit der Suche.
    pub filter: Option<Value>,
    pub team: Option<String>,
    pub include_comments: bool,
    pub include_archived: bool,
    pub order: SearchOrder,
    pub offset: i64,
    pub limit: i64,
}

impl Default for SearchInput {
    fn default() -> Self {
        Self {
            term: String::new(),
            kinds: Vec::new(),
            filter: None,
            team: None,
            include_comments: false,
            include_archived: true,
            order: SearchOrder::Relevance,
            offset: 0,
            limit: 50,
        }
    }
}

/// Eine Trefferseite.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchPage {
    pub hits: Vec<SearchHitRow>,
    /// Issues der Treffer (für Adapter, die das ganze Issue ausliefern).
    pub issues: std::collections::HashMap<Uuid, IssueView>,
    pub next_offset: Option<i64>,
}

/// Höchstzahl der Treffer pro Suche (docs/05 §5.4).
pub const MAX_SEARCH_HITS: i64 = 500;

impl App {
    /// Globale Volltextsuche: Issues, Projekte, Initiativen, Dokumente; nur Sichtbares.
    pub async fn search(&self, ctx: &RequestCtx, input: SearchInput) -> AppResult<SearchPage> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = view_access(&snap, ctx)?;
        let now = self.clock.now();
        let limit = input.limit.clamp(1, 250);
        let offset = input.offset.clamp(0, MAX_SEARCH_HITS - 1);
        let mut kinds = if input.kinds.is_empty() {
            vec![
                SearchKind::Issue,
                SearchKind::Project,
                SearchKind::Initiative,
                SearchKind::Document,
            ]
        } else {
            input.kinds.clone()
        };
        if kinds.contains(&SearchKind::Initiative) {
            let enabled = tx.orgs().get().await?.is_none_or(|o| o.initiatives_enabled);
            if !enabled || access.is_guest {
                kinds.retain(|k| *k != SearchKind::Initiative);
            }
        }
        let team_scope = match &input.team {
            Some(t) => Some(vec![
                resolve::team::resolve_in(&mut *tx, &snap, t, Visibility::Content)
                    .await?
                    .id,
            ]),
            None => None,
        };
        let mut ast = None;
        let mut eval = None;
        if let Some(fj) = &input.filter {
            let (any, norm) = parse_view_filter(ViewModel::Issue, fj)?;
            let ev = build_eval(&mut *tx, &snap, ctx, now, true, true).await?;
            tx.views().verify_filter_refs(&any, &ev).await?;
            ast = Some(Box::new(
                filter::from_json_value::<IssueFilter>(&norm).map_err(filter_error)?,
            ));
            eval = Some(ev);
        }
        let fetch = (offset + limit + 1).min(MAX_SEARCH_HITS);
        let rows = tx
            .views()
            .search(&SearchQuery {
                term: input.term.clone(),
                kinds,
                access,
                team_scope,
                include_comments: input.include_comments,
                include_archived: input.include_archived,
                order: input.order,
                ast,
                eval,
                limit: fetch,
            })
            .await?;
        let end = (offset + limit) as usize;
        let more = rows.len() > end && (end as i64) < MAX_SEARCH_HITS;
        let hits: Vec<SearchHitRow> = rows.into_iter().skip(offset as usize).take(limit as usize).collect();
        let issue_ids: Vec<IssueId> = hits
            .iter()
            .filter(|h| h.kind == SearchKind::Issue)
            .map(|h| IssueId(h.id))
            .collect();
        let mut issues = std::collections::HashMap::new();
        if !issue_ids.is_empty() {
            let extras = tx.issues().extras(&issue_ids).await?;
            for id in issue_ids {
                if let (Some(issue), Some(ex)) =
                    (tx.issues().get(id, false).await?, extras.get(&id))
                {
                    issues.insert(
                        id.0,
                        IssueView {
                            issue,
                            extras: ex.clone(),
                            previous_identifiers: Vec::new(),
                        },
                    );
                }
            }
        }
        Ok(SearchPage {
            hits,
            issues,
            next_offset: more.then_some(end as i64),
        })
    }
}
