//! usecases – Labels (docs/03 §5, docs/07 §3.4): Issue-, Projekt- und Initiative-Labels mit
//! Gruppen, Scopes (Workspace/Team), Retire/Restore. Eine Implementierung für alle drei Familien.

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::label::{Label, LabelKind, LabelListFilter, LabelPatch, NewLabel};
use crate::ports::repos::team::Team;
use crate::ports::repos::user::{OrderBy, Page, PageCursor, PageRequest};
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::actor_user_id;
use lineal_domain::enums::LabelGroupType;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{LabelId, TeamId};
use serde_json::json;
use std::collections::BTreeSet;

/// Reservierte Namen für Issue-Labels (docs/03 §5.3).
pub const RESERVED_NAMES: [&str; 9] = [
    "assignee", "cycle", "effort", "estimate", "hours", "priority", "project", "state", "status",
];

/// Ein Label mit aufgelösten Verweisen.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelView {
    pub label: Label,
    /// `(id, name)` der Gruppe.
    pub parent: Option<(LabelId, String)>,
    /// `(id, key, name)` des Teams.
    pub team: Option<(TeamId, String, String)>,
    /// Kinder, deren Zustand sich durch Retire/Restore geändert hat.
    pub affected_children: Vec<LabelId>,
}

/// Eingabe von `list_labels`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListLabelsInput {
    pub team: Option<String>,
    pub name: Option<String>,
    pub include_groups: bool,
    pub include_archived: bool,
    pub order_by: OrderBy,
    pub limit: i64,
    pub cursor: Option<PageCursor>,
}

/// Eingabe von `save_label`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaveLabelInput {
    /// ID oder Name; fehlt beim Anlegen.
    pub id: Option<String>,
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: Option<Option<String>>,
    /// Beim Anlegen der Scope; bei Updates nur zur Disambiguierung des Namens.
    pub team_id: Option<String>,
    pub parent: Option<Option<String>>,
    pub is_group: Option<bool>,
    pub group_type: Option<LabelGroupType>,
}

/// Seite einer Label-Liste.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelPage {
    pub items: Vec<LabelView>,
    pub next: Option<PageCursor>,
}

/// Standardfarbe neuer Labels.
pub const DEFAULT_COLOR: &str = "#95A2B3";

fn invalid(field: &str, reason: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field, "reason": reason }))
}

fn validate_name(kind: LabelKind, raw: &str) -> AppResult<String> {
    let n = lineal_domain::rules::text::validate_single_line_name("name", raw, 80)?;
    let n = n.as_str();
    if n.is_empty() || n.chars().count() > 80 {
        return Err(invalid(
            "name",
            "length",
            "name must be between 1 and 80 characters",
        ));
    }
    if kind == LabelKind::Issue && RESERVED_NAMES.contains(&n.to_lowercase().as_str()) {
        return Err(invalid(
            "name",
            "reserved_name",
            format!("\"{n}\" is a reserved label name"),
        ));
    }
    Ok(n.to_owned())
}

fn validate_color(color: &str) -> AppResult<()> {
    let ok = color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(invalid("color", "format", "color must be #RRGGBB"))
    }
}

fn manage_action(kind: LabelKind, team: bool) -> Action {
    match (kind, team) {
        (LabelKind::Issue, true) => Action::ManageTeamLabels,
        (LabelKind::Issue, false) => Action::ManageWorkspaceLabels,
        (_, true) => Action::ManageTeamProjectLabels,
        (_, false) => Action::ManageWorkspaceProjectLabels,
    }
}

fn require_manage(snap: &PolicySnapshot, ctx: &RequestCtx, label: &Label) -> AppResult<()> {
    match label.team_id {
        Some(t) => {
            snap.require_team(ctx, manage_action(label.kind, true), t)?;
        }
        None => snap.require_workspace(ctx, manage_action(label.kind, false))?,
    }
    Ok(())
}

/// Endgültiges Löschen (auch als Teil von Merge) nur für Admins bzw. Team-Owner (docs/03 §5.4).
fn require_destroy(snap: &PolicySnapshot, label: &Label) -> AppResult<()> {
    let allowed = snap.actor.is_admin()
        || snap.actor.is_system()
        || label
            .team_id
            .and_then(|t| snap.view(t).ok())
            .is_some_and(|v| v.is_team_owner);
    if allowed {
        Ok(())
    } else {
        Err(
            AppError::forbidden("Only admins or team owners can delete or merge labels.")
                .with_details(json!({ "reason": "role" })),
        )
    }
}

async fn view_of(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    label: Label,
    affected: Vec<LabelId>,
) -> AppResult<LabelView> {
    let parent = match label.parent_id {
        Some(p) => tx
            .labels()
            .get(label.kind, p)
            .await?
            .map(|g| (g.id, g.name)),
        None => None,
    };
    let team = label
        .team_id
        .and_then(|t| snap.tree.get(t))
        .map(|t| (t.id, t.key.clone(), t.name.clone()));
    Ok(LabelView {
        label,
        parent,
        team,
        affected_children: affected,
    })
}

async fn emit(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    kind: LabelKind,
    event: fn(LabelId) -> DomainEvent,
    id: LabelId,
) -> AppResult<()> {
    if kind == LabelKind::Issue {
        tx.outbox().push(NewEvent::from_ctx(event(id), ctx)).await?;
    }
    Ok(())
}

fn created(id: LabelId) -> DomainEvent {
    DomainEvent::IssueLabelCreated { label_id: id }
}
fn updated(id: LabelId) -> DomainEvent {
    DomainEvent::IssueLabelUpdated { label_id: id }
}

/// Scope-Namenskollision (docs/03 §5.3): ein neues Label darf nicht so heißen wie ein Label im
/// selben Sichtbarkeitsraum.
fn name_conflicts<'a>(
    snap: &PolicySnapshot,
    existing: &'a [Label],
    scope: Option<TeamId>,
    parent: Option<LabelId>,
    except: Option<LabelId>,
) -> Vec<&'a Label> {
    existing
        .iter()
        .filter(|l| Some(l.id) != except)
        .filter(|l| match parent {
            // innerhalb einer Gruppe eindeutig
            Some(p) => l.parent_id == Some(p),
            // Gruppen und freie Labels teilen sich den Namensraum ihres Scopes
            None => {
                l.parent_id.is_none()
                    && match (scope, l.team_id) {
                        (None, _) => true,
                        (Some(_), None) => true,
                        (Some(s), Some(t)) => {
                            let mut related: BTreeSet<TeamId> = BTreeSet::from([s]);
                            related.extend(snap.tree.ancestors(s).iter().map(|x| x.id));
                            related.extend(snap.tree.descendants(s).iter().map(|x| x.id));
                            related.contains(&t)
                        }
                    }
            }
        })
        .collect()
}

fn name_conflict_error(snap: &PolicySnapshot, name: &str, found: &[&Label]) -> AppError {
    // Labels unsichtbarer (privater) Teams werden nie genannt.
    let visible = snap.visible_team_ids();
    let found: Vec<&Label> = found
        .iter()
        .copied()
        .filter(|l| l.team_id.is_none_or(|t| visible.contains(&t)))
        .collect();
    let found = found.as_slice();
    AppError::conflict(format!(
        "A label named \"{name}\" already exists in this scope."
    ))
    .with_details(json!({
        "field": "name",
        "conflictingLabels": found.iter().take(10).map(|l| json!({
            "id": l.id.to_string(), "name": l.name,
            "team": l.team_id.map(|t| t.to_string()),
        })).collect::<Vec<_>>(),
    }))
}

/// Gruppe prüfen: muss eine Gruppe sein, ohne eigenen Parent, im passenden Scope.
fn check_parent(snap: &PolicySnapshot, group: &Label, scope: Option<TeamId>) -> AppResult<()> {
    if !group.is_group {
        return Err(invalid(
            "parent",
            "parent_not_a_group",
            "parent must be a label group",
        ));
    }
    let fits = match (group.team_id, scope) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(g), Some(s)) => g == s || snap.tree.ancestors(s).iter().any(|t| t.id == g),
        (Some(_), None) => false,
    };
    if !fits {
        return Err(invalid(
            "parent",
            "parent_scope_mismatch",
            "the parent group belongs to a different scope",
        ));
    }
    Ok(())
}

impl App {
    /// Labels auflisten (docs/07 §3.4): Issue-Labels = Workspace + Team (und Vorfahren), Projekt-Labels
    /// ohne `team` nur Workspace, Initiative-Labels nur Workspace.
    pub async fn list_labels(
        &self,
        ctx: &RequestCtx,
        kind: LabelKind,
        input: ListLabelsInput,
    ) -> AppResult<LabelPage> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let tx = &mut *tx;
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let visible = snap.visible_team_ids();
        let mut scopes: Vec<Option<TeamId>> = vec![None];
        match (&input.team, kind) {
            (_, LabelKind::Initiative) => {}
            (Some(r), _) => {
                let team = resolve::team::resolve_in(tx, &snap, r, Visibility::Content).await?;
                for t in resolve::label::available_scopes(&snap.tree, team.id) {
                    if visible.contains(&t) {
                        scopes.push(Some(t));
                    }
                }
            }
            (None, LabelKind::Issue) => scopes.extend(visible.iter().map(|t| Some(*t))),
            (None, _) => {}
        }
        let filter = LabelListFilter {
            scopes,
            name: input.name,
            include_groups: input.include_groups,
            include_archived: input.include_archived,
        };
        let page: Page<Label> = tx
            .labels()
            .list(
                kind,
                &filter,
                &PageRequest {
                    limit: input.limit.clamp(1, 250),
                    cursor: input.cursor,
                    order_by: input.order_by,
                },
            )
            .await?;
        let mut items = Vec::new();
        for l in page.items {
            items.push(view_of(tx, &snap, l, vec![]).await?);
        }
        Ok(LabelPage {
            items,
            next: page.next,
        })
    }

    /// Legt ein Label an oder ändert es (docs/07 `save_*_label`).
    pub async fn save_label(
        &self,
        ctx: &RequestCtx,
        kind: LabelKind,
        input: SaveLabelInput,
    ) -> AppResult<LabelView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let view = match input.id.clone() {
            Some(id) => {
                self.update_label_in(&mut *tx, ctx, kind, &id, input)
                    .await?
            }
            None => self.create_label_in(&mut *tx, ctx, kind, input).await?,
        };
        tx.commit().await?;
        Ok(view)
    }

    async fn create_label_in(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        kind: LabelKind,
        input: SaveLabelInput,
    ) -> AppResult<LabelView> {
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let now = self.clock.now();
        let name = validate_name(
            kind,
            input
                .name
                .as_deref()
                .ok_or_else(|| invalid("name", "required", "name is required when creating"))?,
        )?;
        let color = input.color.clone().unwrap_or_else(|| DEFAULT_COLOR.into());
        validate_color(&color)?;
        // Scope
        let team: Option<Team> = match (&input.team_id, kind) {
            (Some(_), LabelKind::Initiative) => {
                return Err(invalid(
                    "teamId",
                    "workspace_only",
                    "initiative labels are workspace labels",
                ));
            }
            (Some(r), _) => {
                Some(resolve::team::resolve_in(tx, &snap, r, Visibility::Content).await?)
            }
            (None, _) => None,
        };
        match &team {
            Some(t) => {
                snap.require_team(ctx, manage_action(kind, true), t.id)?;
            }
            None => snap.require_workspace(ctx, manage_action(kind, false))?,
        }
        let scope = team.as_ref().map(|t| t.id);
        // Gruppe/Kind
        let is_group = input.is_group.unwrap_or(false);
        let parent = match &input.parent {
            Some(Some(r)) => {
                if is_group {
                    return Err(invalid(
                        "parent",
                        "group_has_parent",
                        "a label group cannot have a parent",
                    ));
                }
                let visible = snap.visible_team_ids();
                let found = resolve::label::resolve_visible(tx, kind, &visible, r, None).await?;
                let group = resolve::label::exactly_one(kind, r, found)?;
                check_parent(&snap, &group, scope)?;
                if let Some(max) = snap.org.limits.max_labels_per_group
                    && tx.labels().count_children(kind, group.id).await? >= max
                {
                    return Err(AppError::limit_exceeded(
                        "the label group has reached its maximum number of labels",
                    )
                    .with_details(json!({ "limit": "maxLabelsPerGroup", "max": max })));
                }
                Some(group)
            }
            _ => None,
        };
        if input.group_type.is_some() && !is_group {
            return Err(invalid(
                "groupType",
                "not_a_group",
                "groupType is only valid for label groups",
            ));
        }
        let existing = tx.labels().find_by_name(kind, &name).await?;
        let conflicts =
            name_conflicts(&snap, &existing, scope, parent.as_ref().map(|g| g.id), None);
        if !conflicts.is_empty() {
            return Err(name_conflict_error(&snap, &name, &conflicts));
        }
        let group_type = match (kind, is_group) {
            (LabelKind::Issue, true) => {
                Some(input.group_type.unwrap_or(LabelGroupType::SingleSelect))
            }
            _ => None,
        };
        let id = LabelId(self.ids.new_uuid());
        let label = tx
            .labels()
            .insert(
                kind,
                &NewLabel {
                    id,
                    name,
                    color,
                    description: input.description.clone().flatten(),
                    team_id: scope,
                    parent_id: parent.map(|g| g.id),
                    is_group,
                    group_type,
                    creator_id: actor_user_id(ctx),
                    now,
                },
            )
            .await?;
        emit(tx, ctx, kind, created, label.id).await?;
        view_of(tx, &snap, label, vec![]).await
    }

    async fn update_label_in(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        kind: LabelKind,
        reference: &str,
        input: SaveLabelInput,
    ) -> AppResult<LabelView> {
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let now = self.clock.now();
        let label = self
            .find_label(tx, &snap, kind, reference, input.team_id.as_deref())
            .await?;
        require_manage(&snap, ctx, &label)?;
        if let Some(t) = &input.team_id {
            // `teamId` darf bei Updates nicht ändern (docs/03 §5.1).
            let target = resolve::team::resolve_in(tx, &snap, t, Visibility::Content).await?;
            if Some(target.id) != label.team_id {
                return Err(invalid(
                    "teamId",
                    "team_is_immutable",
                    "the scope of a label cannot be changed",
                ));
            }
        }
        let mut patch = LabelPatch::default();
        if let Some(n) = &input.name {
            let n = validate_name(kind, n)?;
            if n.to_lowercase() != label.name.to_lowercase() {
                let existing = tx.labels().find_by_name(kind, &n).await?;
                let conflicts = name_conflicts(
                    &snap,
                    &existing,
                    label.team_id,
                    label.parent_id,
                    Some(label.id),
                );
                if !conflicts.is_empty() {
                    return Err(name_conflict_error(&snap, &n, &conflicts));
                }
            }
            patch.name = Some(n);
        }
        if let Some(c) = &input.color {
            validate_color(c)?;
            patch.color = Some(c.clone());
        }
        patch.description = input.description.clone();
        if let Some(g) = input.is_group {
            if g && label.parent_id.is_some() && !matches!(input.parent, Some(None)) {
                return Err(invalid(
                    "isGroup",
                    "child_cannot_be_group",
                    "a label inside a group cannot become a group",
                ));
            }
            if !g && label.is_group && tx.labels().count_children(kind, label.id).await? > 0 {
                return Err(invalid(
                    "isGroup",
                    "group_has_children",
                    "a group with labels cannot become a normal label",
                ));
            }
            patch.is_group = Some(g);
        }
        if let Some(t) = input.group_type {
            if !(label.is_group || input.is_group == Some(true)) {
                return Err(invalid(
                    "groupType",
                    "not_a_group",
                    "groupType is only valid for label groups",
                ));
            }
            if kind == LabelKind::Issue {
                patch.group_type = Some(t);
            }
        }
        if let Some(p) = &input.parent {
            match p {
                None => patch.parent_id = Some(None),
                Some(r) => {
                    if label.is_group || input.is_group == Some(true) {
                        return Err(invalid(
                            "parent",
                            "group_has_parent",
                            "a label group cannot have a parent",
                        ));
                    }
                    let visible = snap.visible_team_ids();
                    let found =
                        resolve::label::resolve_visible(tx, kind, &visible, r, None).await?;
                    let group = resolve::label::exactly_one(kind, r, found)?;
                    check_parent(&snap, &group, label.team_id)?;
                    if label.parent_id != Some(group.id)
                        && let Some(max) = snap.org.limits.max_labels_per_group
                        && tx.labels().count_children(kind, group.id).await? >= max
                    {
                        return Err(AppError::limit_exceeded(
                            "the label group has reached its maximum number of labels",
                        )
                        .with_details(json!({ "limit": "maxLabelsPerGroup", "max": max })));
                    }
                    patch.parent_id = Some(Some(group.id));
                }
            }
        }
        let saved = tx.labels().update(kind, label.id, &patch, now).await?;
        emit(tx, ctx, kind, updated, saved.id).await?;
        view_of(tx, &snap, saved, vec![]).await
    }

    /// Findet ein Label für Updates/Retire/Restore (ID oder Name; `team_hint` disambiguiert).
    async fn find_label(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        kind: LabelKind,
        reference: &str,
        team_hint: Option<&str>,
    ) -> AppResult<Label> {
        let hint = match team_hint {
            Some(t)
                if kind != LabelKind::Initiative
                    && uuid::Uuid::parse_str(reference.trim()).is_err() =>
            {
                Some(
                    resolve::team::resolve_in(tx, snap, t, Visibility::Content)
                        .await?
                        .id,
                )
            }
            _ => None,
        };
        let visible = snap.visible_team_ids();
        let found = resolve::label::resolve_visible(tx, kind, &visible, reference, hint).await?;
        resolve::label::exactly_one(kind, reference, found)
    }

    /// Setzt ein Label (bei Gruppen samt Kindern) auf retired.
    pub async fn retire_label(
        &self,
        ctx: &RequestCtx,
        kind: LabelKind,
        reference: &str,
    ) -> AppResult<LabelView> {
        self.set_label_retired(ctx, kind, reference, true).await
    }

    /// Stellt ein Label (bei Gruppen alle Kinder) wieder her.
    pub async fn restore_label(
        &self,
        ctx: &RequestCtx,
        kind: LabelKind,
        reference: &str,
    ) -> AppResult<LabelView> {
        self.set_label_retired(ctx, kind, reference, false).await
    }

    async fn set_label_retired(
        &self,
        ctx: &RequestCtx,
        kind: LabelKind,
        reference: &str,
        retire: bool,
    ) -> AppResult<LabelView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let label = self
            .find_label(&mut *tx, &snap, kind, reference, None)
            .await?;
        require_manage(&snap, ctx, &label)?;
        let children = if label.is_group {
            tx.labels().children(kind, label.id).await?
        } else {
            Vec::new()
        };
        if !retire
            && let Some(parent) = label.parent_id
            && let Some(g) = tx.labels().get(kind, parent).await?
            && g.retired_at.is_some()
        {
            return Err(invalid(
                "id",
                "group_is_retired",
                "restore the label group first",
            ));
        }
        // Betroffen sind nur Labels, deren Zustand sich ändert (idempotent).
        let affected: Vec<LabelId> = children
            .iter()
            .filter(|c| c.retired_at.is_some() != retire)
            .map(|c| c.id)
            .collect();
        let mut ids = affected.clone();
        ids.push(label.id);
        tx.labels()
            .set_retired(kind, &ids, actor_user_id(ctx), retire, now)
            .await?;
        let saved = tx
            .labels()
            .get(kind, label.id)
            .await?
            .ok_or_else(|| AppError::internal("label vanished"))?;
        if saved.version != label.version {
            emit(&mut *tx, ctx, kind, updated, saved.id).await?;
        }
        let view = view_of(&mut *tx, &snap, saved, affected).await?;
        tx.commit().await?;
        Ok(view)
    }
}

/// Ergebnis von Merge/Delete: Anzahl betroffener Issues (nur in für den Aufrufer sichtbaren Teams).
#[derive(Debug, Clone, PartialEq)]
pub struct LabelBulkResult {
    /// Das (verbleibende) Ziel-Label beim Merge, sonst `None`.
    pub target: Option<LabelView>,
    pub affected_issues: usize,
}

impl App {
    /// Anzahl der Issues mit diesem Issue-Label, begrenzt auf für den Aufrufer sichtbare Teams
    /// (für die Bestätigung vor „Delete“/„Merge“, docs/03 §5.4).
    pub async fn issue_label_usage(&self, ctx: &RequestCtx, reference: &str) -> AppResult<i64> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let label = self
            .find_label(&mut *tx, &snap, LabelKind::Issue, reference, None)
            .await?;
        require_manage(&snap, ctx, &label)?;
        let mut ids = vec![label.id];
        if label.is_group {
            ids.extend(
                tx.labels()
                    .children(LabelKind::Issue, label.id)
                    .await?
                    .into_iter()
                    .map(|c| c.id),
            );
        }
        let visible = snap.visible_team_ids();
        let issues = tx.labels().active_issues_with_issue_labels(&ids).await?;
        Ok(issues.iter().filter(|(_, t)| visible.contains(t)).count() as i64)
    }

    /// Führt das Issue-Label `source` in `target` zusammen (docs/03 §5.4): Issues, Templates und
    /// Views werden umgehängt, danach wird `source` gelöscht.
    pub async fn merge_issue_label(
        &self,
        ctx: &RequestCtx,
        source: &str,
        target: &str,
    ) -> AppResult<LabelBulkResult> {
        crate::error::with_retry(|| self.merge_issue_label_once(ctx, source, target)).await
    }

    async fn merge_issue_label_once(
        &self,
        ctx: &RequestCtx,
        source: &str,
        target: &str,
    ) -> AppResult<LabelBulkResult> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let src = self
            .find_label(&mut *tx, &snap, LabelKind::Issue, source, None)
            .await?;
        let dst = self
            .find_label(&mut *tx, &snap, LabelKind::Issue, target, None)
            .await?;
        require_manage(&snap, ctx, &src)?;
        require_manage(&snap, ctx, &dst)?;
        require_destroy(&snap, &src)?;
        // Zuordnungen des Quell-Labels anhalten, bis der Merge committet ist (siehe apply_labels).
        tx.advisory_lock("issue_label", Some(src.id.0)).await?;
        if src.id == dst.id {
            return Err(invalid(
                "target",
                "same_label",
                "source and target must differ",
            ));
        }
        if src.is_group || dst.is_group {
            return Err(invalid(
                "id",
                "label_is_group",
                "label groups cannot be merged",
            ));
        }
        if dst.retired_at.is_some() {
            return Err(invalid(
                "target",
                "label_retired",
                "cannot merge into a retired label",
            ));
        }
        if let Some(parent) = dst.parent_id
            && let Some(g) = tx.labels().get(LabelKind::Issue, parent).await?
            && g.effective_group_type() == LabelGroupType::SingleSelect
        {
            return Err(invalid(
                "target",
                "single_select_group",
                "cannot merge into a label of a single-select group",
            ));
        }
        let issues = tx.labels().issues_with_issue_labels(&[src.id]).await?;
        // Ziel muss in allen betroffenen Teams verfügbar sein.
        if let Some(t) = dst.team_id {
            for (_, team) in &issues {
                if !resolve::label::available_scopes(&snap.tree, *team).contains(&t) {
                    return Err(invalid(
                        "target",
                        "target_not_available",
                        "the target label is not available in all affected teams",
                    ));
                }
            }
        }
        let mut had_target = Vec::new();
        for (issue, _) in &issues {
            had_target.push(tx.issues().label_ids(*issue).await?.contains(&dst.id));
        }
        tx.labels().merge_issue_label(src.id, dst.id).await?;
        for ((issue_id, _), had) in issues.iter().zip(had_target) {
            record_label_history(
                self,
                &mut *tx,
                ctx,
                *issue_id,
                if had { vec![] } else { vec![dst.id] },
                vec![src.id],
                now,
            )
            .await?;
        }
        tx.labels().delete_issue_labels(&[src.id]).await?;
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::IssueLabelRemoved { label_id: src.id },
                ctx,
            ))
            .await?;
        let saved = tx
            .labels()
            .get(LabelKind::Issue, dst.id)
            .await?
            .ok_or_else(|| AppError::internal("label vanished"))?;
        let view = view_of(&mut *tx, &snap, saved, Vec::new()).await?;
        let visible = snap.visible_team_ids();
        let visible_count = issues.iter().filter(|(_, t)| visible.contains(t)).count();
        tx.commit().await?;
        Ok(LabelBulkResult {
            target: Some(view),
            affected_issues: visible_count,
        })
    }

    /// Löscht ein Issue-Label endgültig (Gruppen samt Kindern, docs/03 §5.4); entfernt es von
    /// allen Issues (History `removedLabelIds`, keine Benachrichtigungen).
    pub async fn delete_issue_label(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<LabelBulkResult> {
        crate::error::with_retry(|| self.delete_issue_label_once(ctx, reference)).await
    }

    async fn delete_issue_label_once(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<LabelBulkResult> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let label = self
            .find_label(&mut *tx, &snap, LabelKind::Issue, reference, None)
            .await?;
        require_manage(&snap, ctx, &label)?;
        require_destroy(&snap, &label)?;
        let mut ids = vec![label.id];
        if label.is_group {
            ids.extend(
                tx.labels()
                    .children(LabelKind::Issue, label.id)
                    .await?
                    .into_iter()
                    .map(|c| c.id),
            );
        }
        let mut locks: Vec<_> = ids.iter().map(|l| l.0).collect();
        locks.sort();
        for l in locks {
            tx.advisory_lock("issue_label", Some(l)).await?;
        }
        let issues = tx.labels().issues_with_issue_labels(&ids).await?;
        let visible = snap.visible_team_ids();
        let visible_count = issues.iter().filter(|(_, t)| visible.contains(t)).count();
        for (issue_id, _) in &issues {
            let present = tx.issues().label_ids(*issue_id).await?;
            let removed: Vec<LabelId> = ids
                .iter()
                .copied()
                .filter(|l| present.contains(l))
                .collect();
            record_label_history(self, &mut *tx, ctx, *issue_id, vec![], removed, now).await?;
        }
        tx.labels().delete_issue_labels(&ids).await?;
        for id in &ids {
            tx.outbox()
                .push(NewEvent::from_ctx(
                    DomainEvent::IssueLabelRemoved { label_id: *id },
                    ctx,
                ))
                .await?;
        }
        tx.commit().await?;
        Ok(LabelBulkResult {
            target: None,
            affected_issues: visible_count,
        })
    }
}

/// History-Eintrag und Event für ein Issue, dessen Label-Menge sich durch Merge/Delete ändert.
async fn record_label_history(
    app: &App,
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    issue_id: lineal_domain::ids::IssueId,
    added: Vec<LabelId>,
    removed: Vec<LabelId>,
    now: chrono::DateTime<chrono::Utc>,
) -> AppResult<()> {
    let Some(issue) = tx.issues().get(issue_id, false).await? else {
        return Ok(());
    };
    let delta = super::issue::history::Delta {
        added_labels: added,
        removed_labels: removed,
        ..Default::default()
    };
    super::issue::history::record(
        tx,
        app.ids.new_uuid(),
        &issue,
        actor_user_id(ctx),
        crate::ports::repos::issue::HistorySource::User,
        &delta,
        now,
    )
    .await?;
    tx.outbox()
        .push(NewEvent::from_ctx(
            DomainEvent::IssueUpdated {
                issue_id: issue.id,
                team_id: issue.team_id,
                changed: vec!["labelIds".into()],
                mentioned_user_ids: vec![],
                previous: None,
            },
            ctx,
        ))
        .await?;
    Ok(())
}
