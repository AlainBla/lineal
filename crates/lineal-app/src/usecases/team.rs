//! usecases – Teams (docs/02 §2–5): Anlage mit Key-Generierung und Default-Workflow, Einstellungen,
//! Key-Wechsel, Mitgliedschaften, Sub-Team-Nesting inkl. Vorschau.
//!
//! Stilllegen/Löschen/Wiederherstellen (`team_lifecycle`) liegt bei WP16.

use super::audit::AuditEvent;
use super::state::{
    WorkflowPointers, copy_states, create_default_workflow, detach_from_parent,
    ensure_triage_state, normalize_to_parent, pointers_of,
};
use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{
    Action, PolicySnapshot, TeamTree, team_not_found, visibility_allowed_under_parent,
};
use crate::ports::Tx;
use crate::ports::repos::label::{Label, LabelKind, LabelListFilter};
use crate::ports::repos::team::{Membership, NewMembership, NewTeam, Team, TeamPatch};
use crate::ports::repos::user::{OrderBy, Page, PageCursor, PageRequest, User};
use crate::resolve;
use crate::resolve::team::Visibility;
use chrono::{DateTime, Utc};
use lineal_domain::enums::{StateType, TeamVisibility, UserRole};
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::identifier::TeamKey;
use lineal_domain::ids::{StateId, TeamId, UserId};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::str::FromStr;

/// Schreibt ein Domain-Event in die Outbox der laufenden Transaktion.
pub(crate) async fn emit(tx: &mut dyn Tx, ctx: &RequestCtx, event: DomainEvent) -> AppResult<()> {
    let origin = match ctx.actor {
        Actor::System => EventOrigin::System,
        _ => EventOrigin::User,
    };
    tx.outbox()
        .push(crate::ports::repos::NewEvent::new(event, origin))
        .await?;
    Ok(())
}

/// `#RRGGBB`.
pub(crate) fn validate_color(color: &str) -> AppResult<()> {
    let ok = color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(AppError::invalid_request("color must be #RRGGBB")
            .with_details(json!({ "field": "color" })))
    }
}

fn invalid(field: &str, reason: &str) -> AppError {
    AppError::invalid_request(format!("invalid value for `{field}`: {reason}"))
        .with_details(json!({ "field": field, "reason": reason }))
}

fn limit(name: &str, message: &str) -> AppError {
    AppError::limit_exceeded(message).with_details(json!({ "limit": name }))
}

/// Default für `Team.securitySettings` (docs/02 §6.3).
pub fn default_team_security() -> Value {
    json!({
        "teamManagement": "member",
        "labelManagement": "member",
        "templateManagement": "member",
        "memberManagement": "member",
        "issueSharing": "owner",
        "pinnedViewManagement": "member"
    })
}

/// Normalisiert einen Team-Key (trimmen, Großbuchstaben) und validiert ihn
/// (`^[A-Z][A-Z0-9]{0,6}$`, nicht `P`/`I`).
pub fn normalize_team_key(input: &str) -> AppResult<String> {
    let key = input.trim().to_uppercase();
    let parsed = TeamKey::parse(&key)?;
    if matches!(parsed.as_str(), "P" | "I") {
        return Err(invalid("key", "P and I are reserved"));
    }
    Ok(key)
}

/// Basis des generierten Keys aus dem Namen (docs/02 §2.2).
pub fn team_key_base(name: &str) -> String {
    let upper: String = name
        .chars()
        .flat_map(|c| match c {
            'ä' | 'Ä' => vec!['A', 'E'],
            'ö' | 'Ö' => vec!['O', 'E'],
            'ü' | 'Ü' => vec!['U', 'E'],
            'ß' => vec!['S', 'S'],
            other => other.to_uppercase().collect(),
        })
        .collect();
    let words: Vec<String> = upper
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let mut base = match words.as_slice() {
        [] => "TEAM".to_string(),
        [one] => one.chars().take(3).collect(),
        many => many
            .iter()
            .take(4)
            .filter_map(|w| w.chars().next())
            .collect(),
    };
    if base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert(0, 'T');
    }
    base
}

/// Der `i`-te Kandidat (`i = 1` ist die Basis, danach `BASE2`, `BASE3`, …, höchstens 7 Zeichen).
pub fn team_key_candidate(base: &str, i: u32) -> String {
    if i <= 1 {
        return base.to_string();
    }
    let suffix = i.to_string();
    let keep = 7usize.saturating_sub(suffix.len());
    format!("{}{}", base.chars().take(keep).collect::<String>(), suffix)
}

async fn generate_team_key(tx: &mut dyn Tx, name: &str) -> AppResult<String> {
    let base = team_key_base(name);
    for i in 1..10_000u32 {
        let cand = team_key_candidate(&base, i);
        if matches!(cand.as_str(), "P" | "I") {
            continue;
        }
        if !tx.teams().key_reserved(&cand, None).await? {
            return Ok(cand);
        }
    }
    Err(AppError::internal("could not generate a team key"))
}

fn validate_team_name(name: &str) -> AppResult<String> {
    Ok(lineal_domain::rules::text::validate_single_line_name(
        "name", name, 80,
    )?)
}

pub(crate) fn validate_timezone(tz: &str) -> AppResult<()> {
    chrono_tz::Tz::from_str(tz)
        .map(|_| ())
        .map_err(|_| invalid("timezone", "must be an IANA time zone"))
}

fn validate_period(field: &str, v: i32) -> AppResult<()> {
    if [1, 3, 6, 9, 12].contains(&v) {
        Ok(())
    } else {
        Err(invalid(field, "must be one of 1, 3, 6, 9, 12"))
    }
}

/// Ein Team mit berechnetem Anzeigenamen (`Engineering › Web`).
#[derive(Debug, Clone, PartialEq)]
pub struct TeamView {
    pub team: Team,
    pub display_name: String,
}

/// Mitglied eines Teams.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamMember {
    pub user: User,
    pub membership: Membership,
}

/// Filter für `list_teams`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListTeamsInput {
    pub query: Option<String>,
    /// Nur direkte Kinder dieses Teams.
    pub parent_team: Option<String>,
    /// Alle Nachfahren dieses Teams (ohne es selbst).
    pub ancestor_team: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    /// Stillgelegte Teams einbeziehen.
    pub include_archived: bool,
    pub page: PageRequest,
}

/// Eingabe für `create_team` (docs/02 §2.3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateTeamInput {
    pub name: String,
    pub key: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// Parent-Team (Referenz); macht das Team zum Sub-Team.
    pub parent: Option<String>,
    pub visibility: Option<TeamVisibility>,
    pub all_members_can_join: Option<bool>,
    pub timezone: Option<String>,
    pub triage_enabled: Option<bool>,
    /// Konfiguration eines anderen Teams kopieren (Status inklusive).
    pub copy_settings_from: Option<String>,
    /// Sub-Team erbt die Workflow-Status des Parents (Default `true`).
    pub inherit_workflow_statuses: Option<bool>,
}

/// Eingabe für `update_team`; `None` = unverändert, `Some(None)` leert nullbare Felder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateTeamInput {
    pub name: Option<String>,
    pub key: Option<String>,
    pub description: Option<Option<String>>,
    pub icon: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub timezone: Option<String>,
    pub visibility: Option<TeamVisibility>,
    pub all_members_can_join: Option<bool>,
    pub security_settings: Option<Value>,
    pub triage_enabled: Option<bool>,
    pub require_priority_to_leave_triage: Option<bool>,
    /// Status-Referenz (`backlog`/`unstarted`).
    pub default_issue_state: Option<String>,
    /// Status-Referenz (`canceled`); leerer Text = Default (erster `canceled`-Status).
    pub auto_close_state: Option<String>,
    pub auto_close_period: Option<Option<i32>>,
    pub auto_archive_period: Option<i32>,
    pub auto_close_parent_issues: Option<bool>,
    pub auto_close_child_issues: Option<bool>,
    pub set_issue_sort_order_on_state_change: Option<String>,
    pub group_issue_history: Option<bool>,
    pub issue_estimation_type: Option<lineal_domain::enums::EstimationType>,
    pub issue_estimation_extended: Option<bool>,
    pub issue_estimation_allow_zero: Option<bool>,
    pub default_issue_estimate: Option<i32>,
    pub cycles_enabled: Option<bool>,
    pub cycle_duration: Option<i32>,
    pub cycle_cooldown_time: Option<i32>,
    pub cycle_start_day: Option<i32>,
    pub upcoming_cycle_count: Option<i32>,
    pub cycle_issue_auto_assign_started: Option<bool>,
    pub cycle_issue_auto_assign_completed: Option<bool>,
    pub cycle_lock_to_active: Option<bool>,
}

/// Optionen für `nest_team` (docs/02 §3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct NestOptions {
    pub add_missing_members_to_parent: bool,
    pub downgrade_visibility: bool,
    pub inherit_workflow_statuses: bool,
    pub inherit_issue_estimation: bool,
    pub state_mapping: Vec<(StateId, StateId)>,
}

impl Default for NestOptions {
    fn default() -> Self {
        Self {
            add_missing_members_to_parent: true,
            downgrade_visibility: false,
            inherit_workflow_statuses: true,
            inherit_issue_estimation: true,
            state_mapping: Vec::new(),
        }
    }
}

/// Ein Konflikt der Nesting-Vorschau.
#[derive(Debug, Clone, PartialEq)]
pub struct NestingConflict {
    /// `membership`, `visibility`, `workflowState`.
    pub kind: String,
    pub message: String,
    pub details: Value,
}

/// Ergebnis von `preview_team_nesting`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NestingPreview {
    pub conflicts: Vec<NestingConflict>,
}

fn view_of(tree: &TeamTree, team: &Team) -> TeamView {
    TeamView {
        display_name: tree.display_name(team.id),
        team: team.clone(),
    }
}

async fn reload_view(tx: &mut dyn Tx, id: TeamId) -> AppResult<TeamView> {
    let tree = TeamTree::new(tx.teams().list_all().await?);
    let team = tree.get(id).cloned().ok_or_else(team_not_found)?;
    Ok(view_of(&tree, &team))
}

fn actor_id(ctx: &RequestCtx) -> Option<UserId> {
    resolve::user::actor_user_id(ctx)
}

/// Fügt `user` in `team` ein (falls noch nicht Mitglied).
async fn ensure_member(
    app: &App,
    tx: &mut dyn Tx,
    team: TeamId,
    user: UserId,
    owner: bool,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    if tx.teams().get_membership(team, user).await?.is_some() {
        return Ok(false);
    }
    tx.teams()
        .insert_membership(&NewMembership {
            id: app.ids.new_uuid(),
            team_id: team,
            user_id: user,
            owner,
            now,
        })
        .await?;
    Ok(true)
}

impl App {
    /// `list_teams`: sichtbare Teams, gefiltert und paginiert (docs/02 §3.6, §6.6).
    pub async fn list_teams(
        &self,
        ctx: &RequestCtx,
        input: ListTeamsInput,
    ) -> AppResult<Page<TeamView>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let visible = snap.visible_team_ids();
        let mut scope: Option<BTreeSet<TeamId>> = None;
        if let Some(p) = &input.parent_team {
            let parent = resolve::team::resolve_in(&mut *tx, &snap, p, Visibility::Content).await?;
            scope = Some(snap.tree.children(parent.id).iter().map(|t| t.id).collect());
        }
        if let Some(a) = &input.ancestor_team {
            let anc = resolve::team::resolve_in(&mut *tx, &snap, a, Visibility::Content).await?;
            let ids: BTreeSet<TeamId> =
                snap.tree.descendants(anc.id).iter().map(|t| t.id).collect();
            scope = Some(match scope {
                Some(s) => s.intersection(&ids).copied().collect(),
                None => ids,
            });
        }
        let q = input
            .query
            .as_deref()
            .map(str::to_lowercase)
            .filter(|q| !q.is_empty());
        let sort_key = |t: &Team| match input.page.order_by {
            OrderBy::CreatedAt => t.created_at,
            OrderBy::UpdatedAt => t.updated_at,
        };
        let mut teams: Vec<&Team> = snap
            .tree
            .all()
            .filter(|t| visible.contains(&t.id) && !t.trashed)
            .filter(|t| input.include_archived || t.retired_at.is_none())
            .filter(|t| scope.as_ref().is_none_or(|s| s.contains(&t.id)))
            .filter(|t| {
                q.as_ref().is_none_or(|q| {
                    t.name.to_lowercase().contains(q.as_str())
                        || t.key.to_lowercase().contains(q.as_str())
                })
            })
            .filter(|t| input.created_after.is_none_or(|a| t.created_at > a))
            .filter(|t| input.updated_after.is_none_or(|a| t.updated_at > a))
            .collect();
        teams.sort_by(|a, b| (sort_key(b), b.id.0).cmp(&(sort_key(a), a.id.0)));
        if let Some(c) = input.page.cursor {
            teams.retain(|t| (sort_key(t), t.id.0) < (c.key, c.id));
        }
        let limit = input.page.limit.max(1) as usize;
        let has_next = teams.len() > limit;
        teams.truncate(limit);
        let next = if has_next {
            teams.last().map(|t| PageCursor {
                key: sort_key(t),
                id: t.id.0,
            })
        } else {
            None
        };
        Ok(Page {
            items: teams.into_iter().map(|t| view_of(&snap.tree, t)).collect(),
            next,
        })
    }

    /// `get_team`: Referenz = UUID, Key (auch früherer Key) oder Name.
    pub async fn get_team(&self, ctx: &RequestCtx, reference: &str) -> AppResult<TeamView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, reference, Visibility::Content).await?;
        Ok(view_of(&snap.tree, &team))
    }

    /// Team für die Verwaltung (auch private Teams für Admins als Metadaten).
    pub async fn get_team_for_management(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<TeamView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, reference, Visibility::Management).await?;
        snap.require_team(ctx, Action::ViewTeamMetadata, team.id)?;
        Ok(view_of(&snap.tree, &team))
    }

    /// Legt ein Team an (docs/02 §2.3), in einer Transaktion.
    pub async fn create_team(
        &self,
        ctx: &RequestCtx,
        input: CreateTeamInput,
    ) -> AppResult<TeamView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let id = self.create_team_in(&mut *tx, ctx, input).await?;
        let view = reload_view(&mut *tx, id).await?;
        tx.commit().await?;
        Ok(view)
    }

    pub(crate) async fn create_team_in(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        input: CreateTeamInput,
    ) -> AppResult<TeamId> {
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let wants_private = matches!(
            input.visibility,
            Some(TeamVisibility::Private | TeamVisibility::Restricted)
        );
        snap.require_workspace(
            ctx,
            if wants_private {
                Action::CreatePrivateTeam
            } else {
                Action::CreateTeam
            },
        )?;
        if let Some(max) = snap.org.limits.max_teams
            && snap.tree.all().filter(|t| !t.trashed).count() as i64 >= max
        {
            return Err(limit(
                "maxTeams",
                "The workspace has reached its team limit.",
            ));
        }
        let name = validate_team_name(&input.name)?;
        if snap
            .tree
            .all()
            .any(|t| !t.trashed && t.name.to_lowercase() == name.to_lowercase())
        {
            return Err(AppError::conflict("A team with this name already exists")
                .with_details(json!({ "field": "name" })));
        }
        if let Some(d) = &input.description
            && d.chars().count() > 2000
        {
            return Err(invalid("description", "must be at most 2000 characters"));
        }
        if let Some(c) = &input.color {
            validate_color(c)?;
        }
        let timezone = input
            .timezone
            .clone()
            .unwrap_or_else(|| "America/Los_Angeles".into());
        validate_timezone(&timezone)?;

        // Parent
        let parent = match &input.parent {
            Some(p) => {
                let parent =
                    resolve::team::resolve_in(tx, &snap, p, Visibility::Management).await?;
                snap.require_team(ctx, Action::SetTeamParent, parent.id)?;
                if !snap.org.limits.sub_teams_enabled {
                    return Err(limit("subTeamsEnabled", "Sub-teams are disabled."));
                }
                if parent.retired_at.is_some() || parent.trashed {
                    return Err(invalid("parent", "the parent team is retired or deleted"));
                }
                if snap.tree.depth(parent.id) + 1 > snap.org.limits.max_team_depth {
                    return Err(limit("maxTeamDepth", "The maximum team depth is exceeded."));
                }
                Some(parent)
            }
            None => None,
        };
        // Sichtbarkeit (docs/02 §5.2)
        let visibility = match (input.visibility, &parent) {
            (Some(v), _) => v,
            (None, Some(p)) if p.visibility != TeamVisibility::Public => TeamVisibility::Restricted,
            (None, _) => TeamVisibility::Public,
        };
        if visibility == TeamVisibility::Restricted && parent.is_none() {
            return Err(invalid(
                "visibility",
                "restricted is only allowed for sub-teams",
            ));
        }
        if let Some(p) = &parent
            && !visibility_allowed_under_parent(p.visibility, visibility)
        {
            return Err(invalid(
                "visibility",
                "a sub-team cannot be more open than its parent",
            ));
        }
        // Key
        let key = match &input.key {
            Some(k) => {
                let k = normalize_team_key(k)?;
                if tx.teams().key_reserved(&k, None).await? {
                    return Err(AppError::conflict("This team key is already in use")
                        .with_details(json!({ "field": "key" })));
                }
                k
            }
            None => generate_team_key(tx, &name).await?,
        };
        let source = match &input.copy_settings_from {
            Some(s) => Some(resolve::team::resolve_in(tx, &snap, s, Visibility::Content).await?),
            None => None,
        };

        let now = self.clock.now();
        let id = TeamId(self.ids.new_uuid());
        let inherit_workflow =
            parent.is_some() && source.is_none() && input.inherit_workflow_statuses.unwrap_or(true);
        let inherit_estimation = parent.is_some() && source.is_none();
        let triage = input.triage_enabled.unwrap_or(false);
        tx.teams()
            .insert(&NewTeam {
                id,
                name,
                key,
                description: input.description.clone(),
                icon: input.icon.clone(),
                color: input.color.clone(),
                parent_id: parent.as_ref().map(|p| p.id),
                visibility,
                all_members_can_join: input.all_members_can_join.unwrap_or(true),
                timezone,
                security_settings: source
                    .as_ref()
                    .map(|s| s.security_settings.clone())
                    .unwrap_or_else(default_team_security),
                triage_enabled: triage,
                inherit_workflow_statuses: inherit_workflow,
                inherit_issue_estimation: inherit_estimation,
                now,
            })
            .await?;

        // Workflow
        let pointers: WorkflowPointers = if let Some(src) = &source {
            copy_states(self, tx, src.id, id, false, now).await?;
            if triage {
                ensure_triage_state(self, tx, id, now).await?;
            }
            pointers_of(&tx.states().list_for_team(id, false).await?)
        } else if inherit_workflow {
            let parent_id = parent.as_ref().map(|p| p.id).unwrap_or(id);
            copy_states(self, tx, parent_id, id, true, now).await?;
            if triage {
                ensure_triage_state(self, tx, id, now).await?;
            }
            pointers_of(&tx.states().list_for_team(id, false).await?)
        } else {
            create_default_workflow(self, tx, id, triage, now).await?
        };
        let mut patch = pointers.into_patch();
        if let Some(src) = &source {
            // „Copy settings from team“ (docs/02 §2.4): Konfiguration, keine Inhalte.
            patch.require_priority_to_leave_triage = Some(src.require_priority_to_leave_triage);
            patch.auto_close_period = Some(src.auto_close_period);
            patch.auto_archive_period = Some(src.auto_archive_period);
            patch.auto_close_parent_issues = Some(src.auto_close_parent_issues);
            patch.auto_close_child_issues = Some(src.auto_close_child_issues);
            patch.set_issue_sort_order_on_state_change =
                Some(src.set_issue_sort_order_on_state_change.clone());
            patch.group_issue_history = Some(src.group_issue_history);
            patch.issue_estimation_type = Some(src.issue_estimation_type);
            patch.issue_estimation_extended = Some(src.issue_estimation_extended);
            patch.issue_estimation_allow_zero = Some(src.issue_estimation_allow_zero);
            patch.default_issue_estimate = Some(src.default_issue_estimate);
            patch.cycles_enabled = Some(src.cycles_enabled);
            patch.cycle_duration = Some(src.cycle_duration);
            patch.cycle_cooldown_time = Some(src.cycle_cooldown_time);
            patch.cycle_start_day = Some(src.cycle_start_day);
            patch.upcoming_cycle_count = Some(src.upcoming_cycle_count);
            patch.cycle_issue_auto_assign_started = Some(src.cycle_issue_auto_assign_started);
            patch.cycle_issue_auto_assign_completed = Some(src.cycle_issue_auto_assign_completed);
            patch.cycle_lock_to_active = Some(src.cycle_lock_to_active);
        } else if let Some(p) = &parent {
            // Sub-Team erbt Schätzung (und Cycle-Schedule, falls der Parent Cycles nutzt).
            patch.issue_estimation_type = Some(p.issue_estimation_type);
            patch.issue_estimation_extended = Some(p.issue_estimation_extended);
            patch.issue_estimation_allow_zero = Some(p.issue_estimation_allow_zero);
            patch.default_issue_estimate = Some(p.default_issue_estimate);
            if p.cycles_enabled {
                patch.cycles_enabled = Some(true);
                patch.cycle_duration = Some(p.cycle_duration);
                patch.cycle_cooldown_time = Some(p.cycle_cooldown_time);
                patch.cycle_start_day = Some(p.cycle_start_day);
                patch.upcoming_cycle_count = Some(p.upcoming_cycle_count);
            }
        }
        tx.teams().update(id, &patch, now).await?;

        // Ersteller: Mitglied (und Owner) des Teams; in Vorfahren zumindest Mitglied.
        if let Some(creator) = actor_id(ctx) {
            let may_own =
                matches!(ctx.actor, Actor::User { role, .. } if !matches!(role, UserRole::Guest));
            ensure_member(self, tx, id, creator, may_own, now).await?;
            if let Some(p) = &parent {
                for anc in std::iter::once(p).chain(snap.tree.ancestors(p.id)) {
                    ensure_member(self, tx, anc.id, creator, false, now).await?;
                }
            }
        }
        emit(tx, ctx, DomainEvent::TeamCreated { team_id: id }).await?;
        self.record_audit(
            tx,
            ctx,
            AuditEvent::new("team.created", "team", Some(id.0), json!({})),
        )
        .await?;
        Ok(id)
    }
}

/// Alle Teams, die beim Verlassen/Entfernen mit betroffen sind: das Team selbst, optional seine
/// Vorfahren, jeweils samt Sub-Teams (Nicht-Gäste verlassen Sub-Teams automatisch).
fn leave_closure(tree: &TeamTree, team: TeamId, also_parents: bool, guest: bool) -> Vec<TeamId> {
    let mut roots = vec![team];
    if also_parents {
        roots.extend(tree.ancestors(team).iter().map(|t| t.id));
    }
    let mut out: Vec<TeamId> = Vec::new();
    for r in roots {
        if !out.contains(&r) {
            out.push(r);
        }
        if !guest {
            for d in tree.descendants(r) {
                if !out.contains(&d.id) {
                    out.push(d.id);
                }
            }
        }
    }
    out
}

impl App {
    /// Ändert Team-Einstellungen (docs/02 §2.5). Rechte je Feldgruppe: `key` → Owner,
    /// `visibility`/`allMembersCanJoin`/`securitySettings` → Owner, Rest → `teamManagement`.
    pub async fn update_team(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        input: UpdateTeamInput,
    ) -> AppResult<TeamView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, reference, Visibility::Management).await?;
        let owner_fields = input.visibility.is_some()
            || input.all_members_can_join.is_some()
            || input.security_settings.is_some();
        let setting_fields = input.name.is_some()
            || input.description.is_some()
            || input.icon.is_some()
            || input.color.is_some()
            || input.timezone.is_some()
            || input.triage_enabled.is_some()
            || input.require_priority_to_leave_triage.is_some()
            || input.default_issue_state.is_some()
            || input.auto_close_state.is_some()
            || input.auto_close_period.is_some()
            || input.auto_archive_period.is_some()
            || input.auto_close_parent_issues.is_some()
            || input.auto_close_child_issues.is_some()
            || input.set_issue_sort_order_on_state_change.is_some()
            || input.group_issue_history.is_some()
            || input.issue_estimation_type.is_some()
            || input.issue_estimation_extended.is_some()
            || input.issue_estimation_allow_zero.is_some()
            || input.default_issue_estimate.is_some()
            || input.cycles_enabled.is_some()
            || input.cycle_duration.is_some()
            || input.cycle_cooldown_time.is_some()
            || input.cycle_start_day.is_some()
            || input.upcoming_cycle_count.is_some()
            || input.cycle_issue_auto_assign_started.is_some()
            || input.cycle_issue_auto_assign_completed.is_some()
            || input.cycle_lock_to_active.is_some();
        // Existenz zuerst (not_found), dann die Rechte der betroffenen Feldgruppen.
        snap.require_team(ctx, Action::ViewTeamMetadata, team.id)?;
        if input.key.is_some() {
            snap.require_team(ctx, Action::ChangeTeamKey, team.id)?;
        }
        if owner_fields {
            snap.require_team(ctx, Action::ChangeTeamVisibility, team.id)?;
        }
        if setting_fields || !(owner_fields || input.key.is_some()) {
            snap.require_team(ctx, Action::UpdateTeamSettings, team.id)?;
        }
        if team.trashed {
            return Err(team_not_found());
        }
        let now = self.clock.now();
        let mut patch = TeamPatch::default();

        if let Some(n) = &input.name {
            let n = validate_team_name(n)?;
            if snap
                .tree
                .all()
                .any(|t| t.id != team.id && !t.trashed && t.name.to_lowercase() == n.to_lowercase())
            {
                return Err(AppError::conflict("A team with this name already exists")
                    .with_details(json!({ "field": "name" })));
            }
            patch.name = Some(n);
        }
        if let Some(Some(d)) = &input.description
            && d.chars().count() > 2000
        {
            return Err(invalid("description", "must be at most 2000 characters"));
        }
        patch.description = input.description.clone();
        patch.icon = input.icon.clone();
        if let Some(Some(c)) = &input.color {
            validate_color(c)?;
        }
        patch.color = input.color.clone();
        if let Some(tz) = &input.timezone {
            validate_timezone(tz)?;
            patch.timezone = Some(tz.clone());
        }
        patch.all_members_can_join = input.all_members_can_join;
        if let Some(sec) = &input.security_settings {
            let ok = sec.as_object().is_some_and(|o| {
                o.values()
                    .all(|v| matches!(v.as_str(), Some("member" | "owner")))
            });
            if !ok {
                return Err(invalid(
                    "securitySettings",
                    "values must be `member` or `owner`",
                ));
            }
            let mut merged = team
                .security_settings
                .as_object()
                .cloned()
                .unwrap_or_default();
            if let Some(o) = sec.as_object() {
                merged.extend(o.clone());
            }
            patch.security_settings = Some(Value::Object(merged));
        }
        patch.require_priority_to_leave_triage = input.require_priority_to_leave_triage;
        if let Some(v) = input.auto_close_period {
            if let Some(v) = v {
                validate_period("autoClosePeriod", v)?;
            }
            patch.auto_close_period = Some(v);
        }
        if let Some(v) = input.auto_archive_period {
            validate_period("autoArchivePeriod", v)?;
            patch.auto_archive_period = Some(v);
        }
        patch.auto_close_parent_issues = input.auto_close_parent_issues;
        patch.auto_close_child_issues = input.auto_close_child_issues;
        if let Some(v) = &input.set_issue_sort_order_on_state_change {
            if !["top", "bottom", "none"].contains(&v.as_str()) {
                return Err(invalid(
                    "setIssueSortOrderOnStateChange",
                    "must be top, bottom or none",
                ));
            }
            patch.set_issue_sort_order_on_state_change = Some(v.clone());
        }
        patch.group_issue_history = input.group_issue_history;

        // Schätzung: bei erbender Schätzung read-only.
        let estimation_touched = input.issue_estimation_type.is_some()
            || input.issue_estimation_extended.is_some()
            || input.issue_estimation_allow_zero.is_some()
            || input.default_issue_estimate.is_some();
        if estimation_touched && team.inherit_issue_estimation {
            return Err(invalid(
                "issueEstimationType",
                "the estimation settings are inherited from the parent team",
            ));
        }
        patch.issue_estimation_type = input.issue_estimation_type;
        patch.issue_estimation_extended = input.issue_estimation_extended;
        patch.issue_estimation_allow_zero = input.issue_estimation_allow_zero;
        if let Some(v) = input.default_issue_estimate {
            if v < 0 {
                return Err(invalid("defaultIssueEstimate", "must not be negative"));
            }
            patch.default_issue_estimate = Some(v);
        }

        // Cycles: bei Parent mit Cycles read-only (docs/02 §2.5).
        let cycles_touched = input.cycles_enabled.is_some()
            || input.cycle_duration.is_some()
            || input.cycle_cooldown_time.is_some()
            || input.cycle_start_day.is_some()
            || input.upcoming_cycle_count.is_some()
            || input.cycle_issue_auto_assign_started.is_some()
            || input.cycle_issue_auto_assign_completed.is_some()
            || input.cycle_lock_to_active.is_some();
        if cycles_touched
            && team
                .parent_id
                .and_then(|p| snap.tree.get(p))
                .is_some_and(|p| p.cycles_enabled)
        {
            return Err(invalid(
                "cyclesEnabled",
                "the cycle settings are inherited from the parent team",
            ));
        }
        let range = |field: &str, v: Option<i32>, lo: i32, hi: i32| -> AppResult<Option<i32>> {
            match v {
                Some(v) if v < lo || v > hi => {
                    Err(invalid(field, &format!("must be between {lo} and {hi}")))
                }
                other => Ok(other),
            }
        };
        patch.cycles_enabled = input.cycles_enabled;
        patch.cycle_duration = range("cycleDuration", input.cycle_duration, 1, 8)?;
        patch.cycle_cooldown_time = range("cycleCooldownTime", input.cycle_cooldown_time, 0, 8)?;
        patch.cycle_start_day = range("cycleStartDay", input.cycle_start_day, 0, 6)?;
        patch.upcoming_cycle_count = range(
            "upcomingCycleCount",
            input.upcoming_cycle_count,
            1,
            snap.org.limits.max_upcoming_cycles as i32,
        )?;
        patch.cycle_issue_auto_assign_started = input.cycle_issue_auto_assign_started;
        patch.cycle_issue_auto_assign_completed = input.cycle_issue_auto_assign_completed;
        patch.cycle_lock_to_active = input.cycle_lock_to_active;

        // Status-Zeiger (Kategorie wird geprüft).
        if let Some(r) = &input.default_issue_state {
            let s = resolve::state::resolve(&mut *tx, &team, r).await?;
            if !matches!(s.state_type, StateType::Backlog | StateType::Unstarted) {
                return Err(invalid(
                    "defaultIssueState",
                    "must be a backlog or unstarted status",
                ));
            }
            patch.default_issue_state_id = Some(Some(s.id));
        }
        if let Some(r) = &input.auto_close_state {
            if r.trim().is_empty() {
                // Leere Referenz = zurück auf den Default (erster Status der Kategorie `canceled`).
                patch.auto_close_state_id = Some(None);
            } else {
                let s = resolve::state::resolve(&mut *tx, &team, r).await?;
                if s.state_type != StateType::Canceled {
                    return Err(invalid("autoCloseState", "must be a canceled status"));
                }
                patch.auto_close_state_id = Some(Some(s.id));
            }
        }

        // Sichtbarkeit (docs/02 §5.2, §4.5)
        let mut cascade: Vec<TeamId> = Vec::new();
        if let Some(v) = input.visibility {
            if v == TeamVisibility::Restricted && team.parent_id.is_none() {
                return Err(invalid(
                    "visibility",
                    "restricted is only allowed for sub-teams",
                ));
            }
            if v != TeamVisibility::Public && !snap.org.limits.private_teams_enabled {
                return Err(limit("privateTeamsEnabled", "Private teams are disabled."));
            }
            if let Some(p) = team.parent_id.and_then(|p| snap.tree.get(p))
                && !visibility_allowed_under_parent(p.visibility, v)
            {
                return Err(invalid(
                    "visibility",
                    "a sub-team cannot be more open than its parent",
                ));
            }
            patch.visibility = Some(v);
            // Kaskade auf Sub-Teams, die jetzt offener wären als erlaubt.
            let mut stack = vec![(team.id, v)];
            while let Some((parent, pv)) = stack.pop() {
                for c in snap.tree.children(parent) {
                    if !visibility_allowed_under_parent(pv, c.visibility) {
                        cascade.push(c.id);
                        stack.push((c.id, TeamVisibility::Restricted));
                    } else {
                        stack.push((c.id, c.visibility));
                    }
                }
            }
        }

        // Triage (docs/02 §2.5)
        let mut triage_patch: Option<bool> = None;
        if let Some(enabled) = input.triage_enabled
            && enabled != team.triage_enabled
        {
            triage_patch = Some(enabled);
            patch.triage_enabled = Some(enabled);
        }

        // Key-Wechsel
        if let Some(k) = &input.key {
            let new_key = normalize_team_key(k)?;
            if new_key != team.key {
                if tx.teams().key_reserved(&new_key, Some(team.id)).await? {
                    return Err(AppError::conflict("This team key is already in use")
                        .with_details(json!({ "field": "key" })));
                }
                // Eigener früherer Key wird reaktiviert; der bisherige Key wandert in die Historie.
                tx.teams().remove_key_history(team.id, &new_key).await?;
                tx.teams()
                    .insert_key_history(team.id, &team.key, now)
                    .await?;
                patch.key = Some(new_key);
            }
        }

        let noop = patch == TeamPatch::default() && cascade.is_empty() && triage_patch.is_none();
        if noop {
            return reload_view(&mut *tx, team.id).await;
        }
        tx.teams().update(team.id, &patch, now).await?;
        if cycles_touched || patch.timezone.is_some() {
            sync_team_cycles(self, &mut *tx, ctx, &team, now).await?;
        }

        if let Some(enabled) = triage_patch {
            let mut p = TeamPatch::default();
            if enabled {
                let s = ensure_triage_state(self, &mut *tx, team.id, now).await?;
                p.triage_issue_state_id = Some(Some(s.id));
            } else {
                // Issues in Triage wandern nach defaultIssueState (docs/02 §2.5).
                let states = tx.states().list_for_team(team.id, false).await?;
                let default = patch
                    .default_issue_state_id
                    .flatten()
                    .or(team.default_issue_state_id);
                if let (Some(tri), Some(default)) = (
                    states.iter().find(|s| s.state_type == StateType::Triage),
                    default,
                ) {
                    tx.states()
                        .migrate_issues(tri.id, default, actor_id(ctx), now)
                        .await?;
                    tx.states().archive(tri.id, now).await?;
                }
                p.triage_issue_state_id = Some(None);
            }
            tx.teams().update(team.id, &p, now).await?;
        }
        if input
            .visibility
            .is_some_and(|v| v != TeamVisibility::Public)
            && let Some(me) = actor_id(ctx)
            && matches!(ctx.actor, Actor::User { role, .. } if role != UserRole::Guest)
            && input.visibility != Some(team.visibility)
        {
            // Wer ein Team privat macht, wird Mitglied und Owner (docs/02 §4.5).
            if ensure_member(self, &mut *tx, team.id, me, true, now).await? {
                // neu angelegt
            } else {
                tx.teams()
                    .set_membership_owner(team.id, me, true, now)
                    .await?;
            }
        }
        for c in cascade {
            tx.teams()
                .update(
                    c,
                    &TeamPatch {
                        visibility: Some(TeamVisibility::Restricted),
                        ..Default::default()
                    },
                    now,
                )
                .await?;
        }
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        let view = reload_view(&mut *tx, team.id).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Mitglieder eines Teams (Verwaltungssicht).
    pub async fn list_team_members(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<Vec<TeamMember>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, reference, Visibility::Management).await?;
        snap.require_team(ctx, Action::ViewTeamMetadata, team.id)?;
        let mut out = Vec::new();
        for m in tx.teams().list_memberships(team.id).await? {
            if let Some(user) = tx.users().get(m.user_id).await? {
                out.push(TeamMember {
                    user,
                    membership: m,
                });
            }
        }
        Ok(out)
    }

    /// Fügt einen User einem Team hinzu (docs/02 §4.2 `addTeamMember`); Nicht-Gäste werden
    /// automatisch auch in alle Vorfahren aufgenommen.
    pub async fn add_team_member(
        &self,
        ctx: &RequestCtx,
        team: &str,
        user: &str,
        owner: bool,
    ) -> AppResult<TeamMember> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Management).await?;
        let target = resolve::user::resolve_in(&mut *tx, &snap, ctx, user).await?;
        if !target.active {
            return Err(invalid("user", "the user is suspended"));
        }
        let guest = target.role == UserRole::Guest;
        snap.require_team(
            ctx,
            if guest {
                Action::AddGuest
            } else {
                Action::AddMember
            },
            team.id,
        )?;
        if owner {
            snap.require_team(ctx, Action::SetTeamOwner, team.id)?;
            if matches!(target.role, UserRole::Guest | UserRole::App) {
                return Err(invalid("owner", "guests and apps cannot be team owners"));
            }
        }
        if tx
            .teams()
            .get_membership(team.id, target.id)
            .await?
            .is_some()
        {
            return Err(AppError::conflict(
                "The user is already a member of this team",
            ));
        }
        if guest && !snap.org.limits.guests_enabled {
            return Err(limit("guestsEnabled", "Guests are disabled."));
        }
        let now = self.clock.now();
        if !guest {
            // Vorfahren: der Aufrufer braucht dort ebenfalls memberManagement.
            for anc in snap.tree.ancestors(team.id) {
                if tx
                    .teams()
                    .get_membership(anc.id, target.id)
                    .await?
                    .is_none()
                {
                    snap.require_team(ctx, Action::AddMember, anc.id)?;
                    ensure_member(self, &mut *tx, anc.id, target.id, false, now).await?;
                }
            }
        }
        ensure_member(self, &mut *tx, team.id, target.id, owner, now).await?;
        let membership = tx
            .teams()
            .get_membership(team.id, target.id)
            .await?
            .ok_or_else(|| AppError::internal("membership vanished"))?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(TeamMember {
            user: target,
            membership,
        })
    }

    /// Selbst beitreten (`canJoin`, docs/02 §4.2). Admins brauchen für private Teams
    /// `confirm_private_join`.
    pub async fn join_team(
        &self,
        ctx: &RequestCtx,
        team: &str,
        confirm_private_join: bool,
    ) -> AppResult<TeamMember> {
        let me = actor_id(ctx).ok_or_else(|| AppError::forbidden("Only users can join teams"))?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Management).await?;
        let view = snap.require_team(ctx, Action::JoinTeam, team.id)?;
        if view.team.visibility == TeamVisibility::Private && !confirm_private_join {
            return Err(AppError::invalid_request(
                "Joining a private team needs confirmPrivateJoin = true",
            )
            .with_details(json!({ "field": "confirmPrivateJoin" })));
        }
        let now = self.clock.now();
        // Vorfahren, in denen der User noch nicht Mitglied ist, müssen beitretbar sein.
        for anc in snap.tree.ancestors(team.id) {
            if !snap.actor.is_member(anc.id) {
                let av = snap.view(anc.id)?;
                if !av.can_join {
                    return Err(if av.can_access {
                        AppError::forbidden("Not allowed to join the parent team")
                    } else {
                        team_not_found()
                    });
                }
                ensure_member(self, &mut *tx, anc.id, me, false, now).await?;
            }
        }
        ensure_member(self, &mut *tx, team.id, me, false, now).await?;
        let user = tx
            .users()
            .get(me)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find user"))?;
        let membership = tx
            .teams()
            .get_membership(team.id, me)
            .await?
            .ok_or_else(|| AppError::internal("membership vanished"))?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(TeamMember { user, membership })
    }

    /// Eigene Mitgliedschaft beenden (docs/02 §4.2); Nicht-Gäste verlassen Sub-Teams mit.
    pub async fn leave_team(
        &self,
        ctx: &RequestCtx,
        team: &str,
        also_leave_parent_teams: bool,
    ) -> AppResult<()> {
        let me = actor_id(ctx).ok_or_else(|| AppError::forbidden("Only users can leave teams"))?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Content).await?;
        snap.require_team(ctx, Action::LeaveTeam, team.id)?;
        if !snap.actor.is_member(team.id) {
            return Err(invalid("team", "you are not a member of this team"));
        }
        let guest = snap.actor.role == UserRole::Guest;
        for t in leave_closure(&snap.tree, team.id, also_leave_parent_teams, guest) {
            tx.teams().delete_membership(t, me).await?;
        }
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Entfernt ein Mitglied (Team-Owner und Admins; sich selbst darf jeder entfernen).
    pub async fn remove_team_member(
        &self,
        ctx: &RequestCtx,
        team: &str,
        user: &str,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Management).await?;
        let target = resolve::user::resolve_in(&mut *tx, &snap, ctx, user).await?;
        let own = actor_id(ctx) == Some(target.id);
        if own {
            snap.require_team(ctx, Action::LeaveTeam, team.id)?;
        } else {
            snap.require_team(ctx, Action::RemoveMember, team.id)?;
        }
        if tx
            .teams()
            .get_membership(team.id, target.id)
            .await?
            .is_none()
        {
            return Err(AppError::not_found("The user is not a member of this team")
                .with_details(json!({ "type": "TeamMembership" })));
        }
        let guest = target.role == UserRole::Guest;
        for t in leave_closure(&snap.tree, team.id, false, guest) {
            tx.teams().delete_membership(t, target.id).await?;
        }
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Ernennt oder entzieht Team-Owner (docs/02 §4.3).
    pub async fn set_team_owner(
        &self,
        ctx: &RequestCtx,
        team: &str,
        user: &str,
        owner: bool,
    ) -> AppResult<TeamMember> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Management).await?;
        let target = resolve::user::resolve_in(&mut *tx, &snap, ctx, user).await?;
        snap.require_team(ctx, Action::SetTeamOwner, team.id)?;
        if owner && matches!(target.role, UserRole::Guest | UserRole::App) {
            return Err(invalid("owner", "guests and apps cannot be team owners"));
        }
        if tx
            .teams()
            .get_membership(team.id, target.id)
            .await?
            .is_none()
        {
            return Err(invalid("user", "the user is not a member of this team"));
        }
        tx.teams()
            .set_membership_owner(team.id, target.id, owner, self.clock.now())
            .await?;
        let membership = tx
            .teams()
            .get_membership(team.id, target.id)
            .await?
            .ok_or_else(|| AppError::internal("membership vanished"))?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(TeamMember {
            user: target,
            membership,
        })
    }

    /// Vorschau für `nest_team` (docs/02 §3.3): Konflikte, ohne zu schreiben.
    pub async fn preview_team_nesting(
        &self,
        ctx: &RequestCtx,
        team: &str,
        parent: Option<&str>,
    ) -> AppResult<NestingPreview> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Management).await?;
        snap.require_team(ctx, Action::SetTeamParent, team.id)?;
        let Some(parent_ref) = parent else {
            return Ok(NestingPreview::default());
        };
        let parent =
            resolve::team::resolve_in(&mut *tx, &snap, parent_ref, Visibility::Management).await?;
        nesting_checks(&snap, &team, &parent)?;
        let plan = plan_nesting(&mut *tx, &snap, &team, &parent, self.clock.now()).await?;
        Ok(plan.into_preview())
    }

    /// Hängt ein Team unter einen Parent oder löst es (`parent = None`) – docs/02 §3.3/§3.5.
    ///
    /// Umgesetzt: Rechte, Zyklus-/Tiefenprüfung, Sichtbarkeit (inkl. Herabstufung),
    /// Mitglieder, Status-Normalisierung samt Issue-Umhängen, Schätzungs-/Cycle-Einstellungen.
    /// Offen (Daten anderer Bereiche): Label-Zusammenführung, Cycle-Instanzen, Projektstatus.
    pub async fn nest_team(
        &self,
        ctx: &RequestCtx,
        team: &str,
        parent: Option<&str>,
        options: NestOptions,
    ) -> AppResult<TeamView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Management).await?;
        snap.require_team(ctx, Action::SetTeamParent, team.id)?;
        let now = self.clock.now();
        let Some(parent_ref) = parent else {
            // Un-Nesting
            if team.parent_id.is_none() {
                return reload_view(&mut *tx, team.id).await;
            }
            detach_from_parent(self, &mut *tx, &team, now).await?;
            let states = tx.states().list_for_team(team.id, false).await?;
            let mut patch = pointers_of(&states).into_patch();
            // bestehende Zeiger behalten, solange sie aktiv sind
            let active = |id: Option<StateId>| id.filter(|i| states.iter().any(|s| s.id == *i));
            if let Some(d) = active(team.default_issue_state_id) {
                patch.default_issue_state_id = Some(Some(d));
            }
            if let Some(a) = active(team.auto_close_state_id) {
                patch.auto_close_state_id = Some(Some(a));
            }
            patch.triage_issue_state_id = None;
            patch.parent_id = Some(None);
            patch.inherit_workflow_statuses = Some(false);
            patch.inherit_issue_estimation = Some(false);
            if team.visibility == TeamVisibility::Restricted {
                patch.visibility = Some(TeamVisibility::Private);
            }
            tx.teams().update(team.id, &patch, now).await?;
            // Cycles: geerbte Cycles des Teams werden eigenständig (docs/04 §1.7).
            for c in tx.cycles().list_for_team(team.id, true).await? {
                if c.inherited_from_id.is_some() {
                    tx.cycles().set_inherited_from(c.id, None, now).await?;
                }
            }
            tx.cycles().set_team_inherits(team.id, true, now).await?;
            emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
            let view = reload_view(&mut *tx, team.id).await?;
            tx.commit().await?;
            return Ok(view);
        };
        let parent =
            resolve::team::resolve_in(&mut *tx, &snap, parent_ref, Visibility::Management).await?;
        nesting_checks(&snap, &team, &parent)?;
        let plan = plan_nesting(&mut *tx, &snap, &team, &parent, self.clock.now()).await?;
        if !plan.visibility_changes.is_empty() && !options.downgrade_visibility {
            return Err(AppError::conflict(
                "The visibility of the team conflicts with its new parent",
            )
            .with_details(json!({ "kind": "visibility", "teams": plan.visibility_details() })));
        }
        if !plan.missing_members.is_empty() && !options.add_missing_members_to_parent {
            return Err(AppError::conflict("Team members are missing in the parent team")
                .with_details(json!({ "kind": "membership", "userIds": plan.missing_members.iter().map(|u| u.to_string()).collect::<Vec<_>>() })));
        }
        // Mitglieder in Parent und Vorfahren ergänzen (ohne owner).
        for user in &plan.missing_members {
            for t in std::iter::once(&parent).chain(snap.tree.ancestors(parent.id)) {
                ensure_member(self, &mut *tx, t.id, *user, false, now).await?;
            }
        }
        // Sichtbarkeit herabstufen.
        for (id, _from, to) in &plan.visibility_changes {
            tx.teams()
                .update(
                    *id,
                    &TeamPatch {
                        visibility: Some(*to),
                        ..Default::default()
                    },
                    now,
                )
                .await?;
        }
        let mut patch = TeamPatch {
            parent_id: Some(Some(parent.id)),
            ..Default::default()
        };
        if options.inherit_workflow_statuses && !options.state_mapping.is_empty() {
            // Jede Zuordnung muss von einem eigenen Status auf einen Status des Parents zeigen.
            let own = tx.states().list_for_team(team.id, false).await?;
            let targets = tx.states().list_for_team(parent.id, false).await?;
            let bad = options.state_mapping.iter().any(|(from, to)| {
                !own.iter()
                    .any(|s| s.id == *from && s.state_type != StateType::Triage)
                    || !targets
                        .iter()
                        .any(|s| s.id == *to && s.state_type != StateType::Triage)
            });
            if bad {
                return Err(invalid(
                    "stateMapping",
                    "every entry must map a status of the team to a status of the parent team",
                ));
            }
        }
        if options.inherit_workflow_statuses {
            let pointers = normalize_to_parent(
                self,
                &mut *tx,
                ctx,
                &team,
                parent.id,
                &options.state_mapping,
                now,
            )
            .await?;
            let mut p = pointers.into_patch();
            p.parent_id = patch.parent_id;
            p.inherit_workflow_statuses = Some(true);
            patch = p;
        }
        if options.inherit_issue_estimation {
            patch.inherit_issue_estimation = Some(true);
            patch.issue_estimation_type = Some(parent.issue_estimation_type);
            patch.issue_estimation_extended = Some(parent.issue_estimation_extended);
            patch.issue_estimation_allow_zero = Some(parent.issue_estimation_allow_zero);
            patch.default_issue_estimate = Some(parent.default_issue_estimate);
        }
        if parent.cycles_enabled {
            patch.cycles_enabled = Some(true);
            patch.cycle_duration = Some(parent.cycle_duration);
            patch.cycle_cooldown_time = Some(parent.cycle_cooldown_time);
            patch.cycle_start_day = Some(parent.cycle_start_day);
            patch.upcoming_cycle_count = Some(parent.upcoming_cycle_count);
        }
        tx.teams().update(team.id, &patch, now).await?;
        if parent.cycles_enabled {
            // Das Team übernimmt den Zeitplan des Parents (Cycles spiegeln, eigene künftige löschen).
            tx.cycles().set_team_inherits(team.id, true, now).await?;
            sync_team_cycles(self, &mut *tx, ctx, &team, now).await?;
        }
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        let view = reload_view(&mut *tx, team.id).await?;
        tx.commit().await?;
        Ok(view)
    }
}

/// Cycle-Engine nach Änderung der Cycle-Einstellungen oder der Hierarchie (WP07-Anbindung):
/// Aktivieren/Deaktivieren, Zeitplan neu berechnen und erbenden Sub-Teams die Cycles spiegeln.
async fn sync_team_cycles(
    app: &App,
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    before: &Team,
    now: DateTime<Utc>,
) -> AppResult<()> {
    use super::cycle::engine::{self, Env, inherits};
    let Some(after) = tx.teams().get(before.id).await? else {
        return Ok(());
    };
    let tree = TeamTree::new(tx.teams().list_all().await?);
    let non_inheriting: std::collections::HashSet<TeamId> = tx
        .cycles()
        .teams_not_inheriting()
        .await?
        .into_iter()
        .collect();
    let env = Env {
        tree: &tree,
        non_inheriting: &non_inheriting,
        now,
        ctx,
        ids: &*app.ids,
    };
    if after.cycles_enabled && !before.cycles_enabled {
        engine::ensure_upcoming(tx, &env, &after, None).await?;
        engine::run_team(tx, &env, &after).await?;
    } else if !after.cycles_enabled && before.cycles_enabled {
        engine::deactivate(tx, &env, &after).await?;
    } else if after.cycles_enabled {
        // Auch ohne Änderung idempotent (Zeitplan neu berechnen, Sub-Teams nachziehen).
        engine::ensure_upcoming(tx, &env, &after, None).await?;
    }
    for child in tree.children(after.id) {
        if inherits(&env, child) {
            engine::sync_inherited(tx, &env, child).await?;
        }
    }
    engine::emit_team_updated(tx, &env, after.id).await?;
    Ok(())
}

/// Harte Voraussetzungen des Nestings (docs/02 §3.1, §3.3 Schritt 1).
fn nesting_checks(snap: &PolicySnapshot, team: &Team, parent: &Team) -> AppResult<()> {
    if !snap.org.limits.sub_teams_enabled {
        return Err(limit("subTeamsEnabled", "Sub-teams are disabled."));
    }
    if snap.tree.is_self_or_descendant(team.id, parent.id) {
        return Err(
            AppError::invalid_request("A team cannot become its own descendant")
                .with_details(json!({ "reason": "cycle" })),
        );
    }
    if team.retired_at.is_some() || team.trashed || parent.retired_at.is_some() || parent.trashed {
        return Err(invalid("parent", "the team and its parent must be active"));
    }
    if snap.tree.depth(parent.id) + snap.tree.height(team.id) > snap.org.limits.max_team_depth {
        return Err(limit("maxTeamDepth", "The maximum team depth is exceeded."));
    }
    Ok(())
}

/// Ein eigener Status des Teams und der Status des Parents, auf den er automatisch fällt.
struct StateMapEntry {
    from_id: StateId,
    from_name: String,
    from_type: StateType,
    to_id: StateId,
    to_name: String,
}

/// Label des Teams, das im Workspace oder in Parent/Vorfahren schon existiert.
struct LabelDuplicate {
    id: String,
    name: String,
    existing_id: String,
}

struct NestingPlan {
    visibility_changes: Vec<(TeamId, TeamVisibility, TeamVisibility)>,
    missing_members: Vec<UserId>,
    states: Vec<StateMapEntry>,
    /// Der Parent hat Cycles: das Team übernimmt dessen Zeitplan.
    cycles: Option<CyclePlan>,
    estimate: Option<(String, String)>,
    labels: Vec<LabelDuplicate>,
}

struct CyclePlan {
    team_cycles_enabled: bool,
    future_cycles: usize,
}

impl NestingPlan {
    fn visibility_details(&self) -> Value {
        Value::Array(
            self.visibility_changes
                .iter()
                .map(|(id, from, to)| json!({ "teamId": id.to_string(), "from": from.as_str(), "to": to.as_str() }))
                .collect(),
        )
    }

    fn into_preview(self) -> NestingPreview {
        let mut conflicts = Vec::new();
        if !self.visibility_changes.is_empty() {
            conflicts.push(NestingConflict {
                kind: "visibility".into(),
                message: "Teams would be more open than their parent; they have to be downgraded to restricted.".into(),
                details: json!({ "teams": self.visibility_details() }),
            });
        }
        if !self.missing_members.is_empty() {
            conflicts.push(NestingConflict {
                kind: "membership".into(),
                message: "Members of the team are missing in the parent team and would be added.".into(),
                details: json!({ "userIds": self.missing_members.iter().map(|u| u.to_string()).collect::<Vec<_>>() }),
            });
        }
        if !self.states.is_empty() {
            conflicts.push(NestingConflict {
                kind: "workflowState".into(),
                message: "Workflow statuses are mapped to the parent's statuses.".into(),
                details: json!({ "states": self.states.iter().map(|m| json!({
                    "id": m.from_id.to_string(),
                    "name": m.from_name,
                    "type": m.from_type.as_str(),
                    "targetId": m.to_id.to_string(),
                    "targetName": m.to_name,
                })).collect::<Vec<_>>() }),
            });
        }
        if !self.labels.is_empty() {
            conflicts.push(NestingConflict {
                kind: "label".into(),
                message: "Labels with the same name exist in the workspace or the parent team. They stay team labels (merging is not supported yet).".into(),
                details: json!({ "labels": self.labels.iter().map(|l| json!({
                    "id": l.id, "name": l.name, "existingId": l.existing_id,
                })).collect::<Vec<_>>() }),
            });
        }
        if let Some(c) = &self.cycles {
            conflicts.push(NestingConflict {
                kind: "cycle".into(),
                message: "The team adopts the cycle schedule of its parent; its own upcoming cycles are replaced.".into(),
                details: json!({
                    "teamCyclesEnabled": c.team_cycles_enabled,
                    "upcomingCycles": c.future_cycles,
                }),
            });
        }
        if let Some((from, to)) = &self.estimate {
            conflicts.push(NestingConflict {
                kind: "estimate".into(),
                message: "The estimation scale of the parent replaces the team's scale.".into(),
                details: json!({ "from": from, "to": to }),
            });
        }
        NestingPreview { conflicts }
    }
}

async fn plan_nesting(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    team: &Team,
    parent: &Team,
    now: DateTime<Utc>,
) -> AppResult<NestingPlan> {
    // Sichtbarkeit: T gegen P, dann jeder Nachfahre gegen seinen (ggf. herabgestuften) Parent.
    let mut changes = Vec::new();
    let mut new_vis = team.visibility;
    if !visibility_allowed_under_parent(parent.visibility, team.visibility) {
        new_vis = TeamVisibility::Restricted;
        changes.push((team.id, team.visibility, new_vis));
    }
    let mut stack = vec![(team.id, new_vis)];
    while let Some((id, pv)) = stack.pop() {
        for c in snap.tree.children(id) {
            if !visibility_allowed_under_parent(pv, c.visibility) {
                changes.push((c.id, c.visibility, TeamVisibility::Restricted));
                stack.push((c.id, TeamVisibility::Restricted));
            } else {
                stack.push((c.id, c.visibility));
            }
        }
    }
    // Mitglieder: Nicht-Gäste von T und Nachfahren, die in P oder einem Vorfahren von P fehlen.
    let mut users: BTreeSet<UserId> = BTreeSet::new();
    for t in std::iter::once(team).chain(snap.tree.descendants(team.id)) {
        for m in tx.teams().list_memberships(t.id).await? {
            users.insert(m.user_id);
        }
    }
    let mut missing = Vec::new();
    for u in users {
        let is_guest = tx
            .users()
            .get(u)
            .await?
            .is_some_and(|x| x.role == UserRole::Guest);
        if is_guest {
            continue;
        }
        let mut lacks = false;
        for t in std::iter::once(parent).chain(snap.tree.ancestors(parent.id)) {
            if tx.teams().get_membership(t.id, u).await?.is_none() {
                lacks = true;
                break;
            }
        }
        if lacks {
            missing.push(u);
        }
    }
    let own: Vec<_> = tx
        .states()
        .list_for_team(team.id, false)
        .await?
        .into_iter()
        .filter(|s| s.state_type != StateType::Triage && s.inherited_from_id.is_none())
        .collect();
    let targets: Vec<_> = tx
        .states()
        .list_for_team(parent.id, false)
        .await?
        .into_iter()
        .filter(|s| s.state_type != StateType::Triage)
        .collect();
    let mut states = Vec::new();
    for s in &own {
        // Dieselbe Regel wie `normalize_to_parent`: gleicher Typ + Name, sonst erster der Kategorie.
        let target = targets
            .iter()
            .find(|t| {
                t.state_type == s.state_type && t.name.to_lowercase() == s.name.to_lowercase()
            })
            .or_else(|| {
                targets
                    .iter()
                    .filter(|t| t.state_type == s.state_type)
                    .min_by(|a, b| a.position.total_cmp(&b.position))
            });
        if let Some(t) = target {
            states.push(StateMapEntry {
                from_id: s.id,
                from_name: s.name.clone(),
                from_type: s.state_type,
                to_id: t.id,
                to_name: t.name.clone(),
            });
        }
    }
    let cycles = if parent.cycles_enabled {
        let future = tx
            .cycles()
            .list_for_team(team.id, true)
            .await?
            .into_iter()
            .filter(|c| c.inherited_from_id.is_none() && c.starts_at > now)
            .count();
        Some(CyclePlan {
            team_cycles_enabled: team.cycles_enabled,
            future_cycles: future,
        })
    } else {
        None
    };
    let scale = |t: &Team| {
        format!(
            "{}{}",
            t.issue_estimation_type.as_str(),
            if t.issue_estimation_extended {
                " (extended)"
            } else {
                ""
            }
        )
    };
    let estimate = (scale(team) != scale(parent)
        || team.issue_estimation_allow_zero != parent.issue_estimation_allow_zero)
        .then(|| (scale(team), scale(parent)));
    // Gleichnamige Labels: Workspace, Parent und Vorfahren gegen die Team-Labels.
    let mut scopes = vec![None, Some(parent.id)];
    scopes.extend(snap.tree.ancestors(parent.id).iter().map(|a| Some(a.id)));
    let page = PageRequest {
        limit: 250,
        ..Default::default()
    };
    let available = tx
        .labels()
        .list(
            LabelKind::Issue,
            &LabelListFilter {
                scopes,
                include_groups: true,
                ..Default::default()
            },
            &page,
        )
        .await?
        .items;
    let own_labels = tx
        .labels()
        .list(
            LabelKind::Issue,
            &LabelListFilter {
                scopes: vec![Some(team.id)],
                include_groups: true,
                ..Default::default()
            },
            &page,
        )
        .await?
        .items;
    let key = |l: &Label, all: &[Label]| {
        let group = l
            .parent_id
            .and_then(|p| all.iter().find(|g| g.id == p))
            .map(|g| g.name.to_lowercase())
            .unwrap_or_default();
        (group, l.name.to_lowercase(), l.is_group)
    };
    let labels = own_labels
        .iter()
        .filter_map(|l| {
            let k = key(l, &own_labels);
            available
                .iter()
                .find(|a| key(a, &available) == k)
                .map(|a| LabelDuplicate {
                    id: l.id.to_string(),
                    name: l.name.clone(),
                    existing_id: a.id.to_string(),
                })
        })
        .collect();
    Ok(NestingPlan {
        visibility_changes: changes,
        missing_members: missing,
        states,
        cycles,
        estimate,
        labels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_generation_follows_docs_02_examples() {
        assert_eq!(team_key_base("Engineering"), "ENG");
        assert_eq!(team_key_base("Design"), "DES");
        assert_eq!(team_key_base("Web"), "WEB");
        assert_eq!(team_key_base("Customer Success Ops"), "CSO");
        assert_eq!(team_key_base("Über Größe"), "UG");
        assert_eq!(team_key_base("3D Printing"), "T3P");
        assert_eq!(team_key_base("!!!"), "TEAM");
        assert_eq!(team_key_base("Äpfel"), "AEP");
    }

    #[test]
    fn key_candidates_stay_within_seven_chars() {
        assert_eq!(team_key_candidate("ENG", 1), "ENG");
        assert_eq!(team_key_candidate("ENG", 2), "ENG2");
        assert_eq!(team_key_candidate("ABCDEFG", 2), "ABCDEF2");
        assert_eq!(team_key_candidate("ABCDEFG", 10), "ABCDE10");
    }

    #[test]
    fn explicit_keys_are_normalized_and_validated() {
        assert_eq!(normalize_team_key(" eng ").unwrap(), "ENG");
        for bad in ["", "1AB", "AB-C", "ABCDEFGH", "P", "I", "p", "ÄB"] {
            assert!(normalize_team_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn leaving_a_parent_removes_non_guests_from_sub_teams() {
        use crate::policy::team::fixtures::*;
        let t = tree();
        assert_eq!(leave_closure(&t, eng(), false, false), vec![eng(), web()]);
        assert_eq!(leave_closure(&t, web(), true, false), vec![web(), eng()]);
        // Gäste verlassen nur das Team selbst.
        assert_eq!(leave_closure(&t, web(), true, true), vec![web(), eng()]);
        assert_eq!(leave_closure(&t, eng(), false, true), vec![eng()]);
    }
}
