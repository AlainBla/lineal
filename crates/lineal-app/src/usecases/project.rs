//! usecases – Projekte (docs/04 §2): mehrere Teams, Lead-Team, `P-KEY-n`, Projektstatus mit
//! Vererbung, Lead/Mitglieder, Daten mit Auflösung, Labels, Links, Fortschritt, Papierkorb/Archiv.
//!
//! Meilensteine liegen in [`milestone`], Status-Updates in [`update`], Sichten in [`view`] und
//! reine Regeln in [`pure`].

pub mod milestone;
pub mod pure;
pub mod update;
pub mod view;

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::project::{
    NewProject, Project, ProjectFilter, ProjectLabelRow, ProjectPatch, ProjectStatus,
};
use crate::ports::repos::team::Team;
use crate::ports::repos::user::{Page, PageRequest, User};
use crate::resolve;
use crate::resolve::project::ResolvedProject;
use crate::resolve::team::Visibility;
use chrono::{DateTime, NaiveDate, Utc};
use lineal_domain::enums::{DateResolution, ProjectStatusType, UserRole};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{InitiativeId, IssueId, ProjectId, TeamId, UserId};
use lineal_domain::rules::patch::{self, PatchOp};
use lineal_domain::rules::state::{ProjectTimestamps, apply_project_transition};
use serde_json::json;
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

pub use view::{MilestoneView, ProjectScope, ProjectView, ViewOpts, build_views};

/// Eingabe von `save_project` (Create ohne `id`, Update mit `id`).
#[derive(Debug, Clone, Default)]
pub struct SaveProjectInput {
    /// Referenz des bestehenden Projekts (Update).
    pub id: Option<String>,
    pub name: Option<String>,
    /// Kurzbeschreibung (Spalte `description`, MCP `summary`), höchstens 255 Zeichen; leer = leeren.
    pub summary: Option<String>,
    /// Markdown-Übersicht (Spalte `content`, MCP `description`).
    pub description: Option<String>,
    /// Patch-Operationen auf `description` (nur Update).
    pub patch: Option<Vec<PatchOp>>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub priority: Option<i32>,
    /// Projektstatus (UUID, Name oder Kategorie), aufgelöst gegen den Satz des Lead-Teams.
    pub state: Option<String>,
    /// User-Referenz; `Some(None)` entfernt den Lead.
    pub lead: Option<Option<String>>,
    pub lead_team: Option<String>,
    pub set_teams: Option<Vec<String>>,
    pub add_teams: Vec<String>,
    pub remove_teams: Vec<String>,
    /// Ersetzt die Label-Menge.
    pub labels: Option<Vec<String>>,
    pub set_initiatives: Option<Vec<String>>,
    pub add_initiatives: Vec<String>,
    pub remove_initiatives: Vec<String>,
    /// Externe Links `(url, title)`; nur hinzufügen.
    pub links: Vec<(String, String)>,
    pub start_date: Option<Option<NaiveDate>>,
    pub start_date_resolution: Option<Option<DateResolution>>,
    pub target_date: Option<Option<NaiveDate>>,
    pub target_date_resolution: Option<Option<DateResolution>>,
    /// Projekt-Template (Anwendung gehört WP10).
    pub template: Option<String>,
    /// Beim Entfernen eines Teams die Issues des Teams vom Projekt lösen (statt `invalid_request`).
    pub detach_issues_of_removed_teams: bool,
}

/// Filter von `list_projects` (Referenzen als Strings, werden aufgelöst).
#[derive(Debug, Clone, Default)]
pub struct ListProjectsInput {
    pub team: Option<String>,
    pub include_sub_teams: bool,
    pub initiative: Option<String>,
    pub member: Option<String>,
    pub state: Option<String>,
    pub label: Option<String>,
    pub query: Option<String>,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub include_members: bool,
    pub include_milestones: bool,
    pub page: PageRequest,
}

/// Ressourcen eines Projekts (`includeResources`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectResources {
    pub documents: Vec<(Uuid, String, String)>,
    pub links: Vec<crate::ports::repos::project::ExternalLink>,
    pub resource_count: i64,
}

pub(crate) async fn emit(tx: &mut dyn Tx, ctx: &RequestCtx, event: DomainEvent) -> AppResult<()> {
    tx.outbox().push(NewEvent::from_ctx(event, ctx)).await?;
    Ok(())
}

/// Schreibrecht für ein bestehendes Projekt: `ManageProject` für alle zugänglichen, nicht
/// stillgelegten Teams; sind alle zugänglichen Teams stillgelegt, ist das Projekt read-only.
pub(crate) fn require_manage(
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    action: Action,
    project_teams: &[TeamId],
) -> AppResult<()> {
    if project_teams.is_empty() {
        return Err(crate::policy::team_not_found());
    }
    // Jede Änderung braucht das Recht für ALLE Teams des Projekts (`not_found` vor `forbidden`).
    // Stillgelegte Teams blockieren nur, wenn alle Teams stillgelegt sind (Projekt read-only).
    let active: Vec<TeamId> = project_teams
        .iter()
        .copied()
        .filter(|t| snap.tree.get(*t).is_some_and(|t| t.retired_at.is_none()))
        .collect();
    let to_check: &[TeamId] = if active.is_empty() {
        project_teams
    } else {
        &active
    };
    // Unzugängliche Teams (auch stillgelegte) liefern zuerst `not_found`.
    let visible = snap.visible_team_ids();
    if project_teams.iter().any(|t| !visible.contains(t)) {
        return Err(crate::policy::team_not_found());
    }
    snap.require_teams(ctx, action, to_check)?;
    Ok(())
}

fn writable(p: &Project) -> AppResult<()> {
    if p.archived_at.is_some() || p.trashed {
        return Err(
            AppError::invalid_request("The project is archived or deleted and read-only.")
                .with_details(json!({ "reason": "project_archived", "project": p.id.to_string() })),
        );
    }
    Ok(())
}

/// Entfernt aktive Inhalte aus einfachem Text (Name, Kurzbeschreibung, Titel).
pub(crate) fn clean_text(s: &str) -> String {
    lineal_domain::rules::markdown::sanitize(s.trim())
        .trim()
        .to_owned()
}

/// Icon: Name oder Emoji-Code (`Rocket`, `:eagle:`) nach Allowlist.
pub(crate) fn validate_icon(i: &str) -> AppResult<Option<String>> {
    if i.is_empty() {
        return Ok(None);
    }
    let ok = i.len() <= 64
        && i.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-'));
    if ok {
        Ok(Some(i.to_owned()))
    } else {
        Err(
            AppError::invalid_request("icon must be an icon name or emoji code")
                .with_details(json!({ "field": "icon" })),
        )
    }
}

/// Externer Link: nur http/https und sicheres Schema.
pub(crate) fn validate_link(url: &str, title: &str) -> AppResult<(String, String)> {
    let lower = url.trim().to_lowercase();
    let ok = (lower.starts_with("http://") || lower.starts_with("https://"))
        && lineal_domain::rules::markdown::is_safe_url(url.trim());
    if !ok {
        return Err(AppError::invalid_request("links must use http or https")
            .with_details(json!({ "field": "links", "url": url })));
    }
    let t = clean_text(title);
    Ok((
        url.trim().to_owned(),
        if t.is_empty() {
            url.trim().to_owned()
        } else {
            t
        },
    ))
}

fn validate_name(name: &str) -> AppResult<String> {
    let n = clean_text(name);
    let n = n.as_str();
    if n.is_empty() || n.chars().count() > 255 {
        return Err(AppError::invalid_request("name must be 1-255 characters")
            .with_details(json!({ "field": "name" })));
    }
    Ok(n.to_owned())
}

fn validate_summary(s: &str) -> AppResult<Option<String>> {
    let s = clean_text(s);
    if s.chars().count() > 255 {
        return Err(
            AppError::invalid_request("summary must be at most 255 characters")
                .with_details(json!({ "field": "summary" })),
        );
    }
    Ok((!s.is_empty()).then_some(s))
}

fn validate_color(c: &str) -> AppResult<()> {
    let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(AppError::invalid_request("color must be #RRGGBB")
            .with_details(json!({ "field": "color" })))
    }
}

fn validate_priority(p: i32) -> AppResult<i32> {
    if (0..=4).contains(&p) {
        Ok(p)
    } else {
        Err(
            AppError::invalid_request("priority must be between 0 and 4")
                .with_details(json!({ "field": "priority" })),
        )
    }
}

fn dedupe<T: Copy + Eq + std::hash::Hash>(v: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|x| seen.insert(*x)).collect()
}

/// Löst Projekt-Labels auf (UUID, Name oder `Gruppe/Label`). Verfügbar: Workspace-Labels plus
/// Team-Labels der Projekt-Teams und ihrer Vorfahren; höchstens ein Label je Gruppe.
pub(crate) async fn resolve_labels(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    refs: &[String],
    project_teams: &[TeamId],
    existing: &[ProjectLabelRow],
) -> AppResult<Vec<ProjectLabelRow>> {
    let mut available: BTreeSet<TeamId> = project_teams.iter().copied().collect();
    for t in project_teams {
        available.extend(snap.tree.ancestors(*t).iter().map(|a| a.id));
    }
    let is_available = |l: &ProjectLabelRow| l.team_id.is_none_or(|t| available.contains(&t));
    let unknown = |r: &str| {
        AppError::not_found(format!("Could not find project label \"{r}\""))
            .with_details(json!({ "reference": r, "type": "ProjectLabel" }))
    };
    let mut out: Vec<ProjectLabelRow> = Vec::new();
    for r in refs {
        let r = r.trim();
        let label = if let Ok(id) = Uuid::parse_str(r) {
            tx.projects().get_label(id).await?.filter(&is_available)
        } else {
            let (group, name) = match r.split_once('/') {
                Some((g, n)) => (Some(g.trim()), n.trim()),
                None => (None, r),
            };
            let mut found = None;
            for l in tx.projects().find_labels_by_name(name).await? {
                if !is_available(&l) {
                    continue;
                }
                if l.retired && !existing.iter().any(|e| e.id == l.id) {
                    continue;
                }
                if let Some(g) = group {
                    let parent = match l.parent_id {
                        Some(p) => tx.projects().get_label(p).await?,
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
            .with_details(json!({ "field": "labels", "label": label.name })));
        }
        if !out.iter().any(|o| o.id == label.id) {
            out.push(label);
        }
    }
    let mut groups = HashSet::new();
    for l in &out {
        if let Some(g) = l.parent_id
            && !groups.insert(g)
        {
            return Err(AppError::invalid_request(
                "A project can carry at most one label from a label group.",
            )
            .with_details(json!({ "field": "labels", "group": g.to_string() })));
        }
    }
    Ok(out)
}

/// Lead/Mitglieder brauchen Zugriff auf mindestens ein Projekt-Team (docs/04 §2.2).
async fn check_member_allowed(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    user: &User,
    teams: &[TeamId],
) -> AppResult<()> {
    let memberships = tx.teams().list_memberships_of_user(user.id).await?;
    let view = crate::policy::ActorView::new(
        crate::context::Actor::User {
            id: user.id,
            role: user.role,
        },
        &memberships,
    );
    let visible = crate::policy::compute_visible_team_ids(&view, &snap.tree);
    if teams.iter().any(|t| visible.contains(t)) {
        Ok(())
    } else {
        Err(
            AppError::invalid_request("The user needs access to at least one team of the project.")
                .with_details(json!({ "field": "lead", "user": user.id.to_string() })),
        )
    }
}

fn status_type_changes(
    old: ProjectStatusType,
    new: ProjectStatusType,
    ts: ProjectTimestamps,
    now: DateTime<Utc>,
) -> ProjectTimestamps {
    apply_project_transition(old, new, now, ts)
}

/// Löst ein Team (nur zugängliche) zu seiner ID auf; unzugänglich = `not_found`.
async fn resolve_team_id(tx: &mut dyn Tx, snap: &PolicySnapshot, r: &str) -> AppResult<TeamId> {
    Ok(resolve::team::resolve_in(tx, snap, r, Visibility::Content)
        .await?
        .id)
}

impl App {
    /// `list_projects`: sichtbare Projekte (mindestens ein zugängliches Team) nach `docs/04 §7.2`.
    pub async fn list_projects(
        &self,
        ctx: &RequestCtx,
        input: ListProjectsInput,
    ) -> AppResult<(Page<Project>, Vec<ProjectView>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let mut filter = ProjectFilter {
            visible_teams: snap.visible_team_ids().into_iter().collect(),
            include_archived: input.include_archived,
            created_after: input.created_after,
            updated_after: input.updated_after,
            query: input.query.clone(),
            ..Default::default()
        };
        if let Some(t) = &input.team {
            let team = resolve::team::resolve_in(&mut *tx, &snap, t, Visibility::Content).await?;
            let mut ids = vec![team.id];
            if input.include_sub_teams {
                ids.extend(snap.tree.descendants(team.id).iter().map(|d| d.id));
            }
            filter.teams = Some(ids);
        }
        if let Some(i) = &input.initiative {
            if snap.actor.role == UserRole::Guest {
                return Err(AppError::forbidden("Guests cannot access initiatives."));
            }
            let init = resolve::initiative::resolve_in(&mut *tx, &snap, i).await?;
            let mut ids = vec![init.id];
            if init.include_sub_initiative_projects {
                let edges = tx.initiatives().list_edges().await?;
                ids.extend(initiative::graph::descendants(&edges, init.id));
            }
            filter.initiatives = Some(ids);
        }
        if let Some(m) = &input.member {
            filter.member = Some(resolve::user::resolve_in(&mut *tx, &snap, ctx, m).await?.id);
        }
        if let Some(s) = &input.state {
            let all = tx.projects().list_statuses().await?;
            let r = s.trim();
            let ids: Vec<Uuid> = if let Ok(id) = Uuid::parse_str(r) {
                all.iter().filter(|x| x.id == id).map(|x| x.id).collect()
            } else {
                let named: Vec<Uuid> = all
                    .iter()
                    .filter(|x| x.name.eq_ignore_ascii_case(r))
                    .map(|x| x.id)
                    .collect();
                if named.is_empty() {
                    match r.parse::<ProjectStatusType>() {
                        Ok(k) => all
                            .iter()
                            .filter(|x| x.status_type == k)
                            .map(|x| x.id)
                            .collect(),
                        Err(_) => Vec::new(),
                    }
                } else {
                    named
                }
            };
            if ids.is_empty() {
                return Err(
                    AppError::not_found(format!("Could not find project status \"{r}\""))
                        .with_details(json!({ "reference": r, "type": "ProjectStatus" })),
                );
            }
            filter.status_ids = Some(ids);
        }
        if let Some(l) = &input.label {
            let r = l.trim();
            let ids: Vec<Uuid> = if let Ok(id) = Uuid::parse_str(r) {
                tx.projects()
                    .get_label(id)
                    .await?
                    .map(|x| vec![x.id])
                    .unwrap_or_default()
            } else {
                tx.projects()
                    .find_labels_by_name(r)
                    .await?
                    .into_iter()
                    .map(|x| x.id)
                    .collect()
            };
            if ids.is_empty() {
                return Err(
                    AppError::not_found(format!("Could not find project label \"{r}\""))
                        .with_details(json!({ "reference": r, "type": "ProjectLabel" })),
                );
            }
            filter.label_ids = Some(ids);
        }
        let page = tx.projects().list(&filter, &input.page).await?;
        let views = build_views(
            &mut *tx,
            &snap,
            page.items.clone(),
            ViewOpts {
                members: input.include_members,
                milestones: input.include_milestones,
            },
            now,
        )
        .await?;
        Ok((page, views))
    }

    /// `get_project`: ein Projekt nach Referenz (UUID, Kennung, Slug, Name).
    pub async fn get_project(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        opts: ViewOpts,
    ) -> AppResult<ProjectView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, reference).await?;
        let mut views =
            build_views(&mut *tx, &snap, vec![r.project], opts, self.clock.now()).await?;
        views
            .pop()
            .ok_or_else(|| AppError::internal("project view missing"))
    }

    /// Ressourcen des Projekts (Dokumente, Links, `resourceCount`).
    pub async fn get_project_resources(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<ProjectResources> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, reference).await?;
        let documents = tx.projects().list_documents(r.project.id).await?;
        let links = tx.projects().list_links(r.project.id).await?;
        Ok(ProjectResources {
            resource_count: (documents.len() + links.len()) as i64,
            documents,
            links,
        })
    }

    /// `save_project`: legt ein Projekt an (ohne `id`) oder ändert es (mit `id`).
    pub async fn save_project(
        &self,
        ctx: &RequestCtx,
        input: SaveProjectInput,
    ) -> AppResult<ProjectView> {
        if input.set_teams.is_some()
            && (!input.add_teams.is_empty() || !input.remove_teams.is_empty())
        {
            return Err(AppError::invalid_request(
                "`setTeams` cannot be combined with `addTeams`/`removeTeams`",
            ));
        }
        if input.set_initiatives.is_some()
            && (!input.add_initiatives.is_empty() || !input.remove_initiatives.is_empty())
        {
            return Err(AppError::invalid_request(
                "`setInitiatives` cannot be combined with `addInitiatives`/`removeInitiatives`",
            ));
        }
        // WP10: Templates gelten nur beim Anlegen (docs/06 §2.5).
        if input.template.is_some() && input.id.is_some() {
            return Err(AppError::invalid_request(
                "`template` is only valid when creating a project.",
            )
            .with_details(json!({ "field": "template", "reason": "template_on_update" })));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        // WP10: Template-Merge (Mengenfelder: Vereinigung, Skalare: Input gewinnt); Meilensteine,
        // Issues und Dokumente entstehen nach dem Projekt in derselben Transaktion.
        let mut input = input;
        let mut applied = None;
        if let Some(t) = input.template.clone() {
            let (merged, ap) =
                super::template::merge_project(self, &mut *tx, &snap, ctx, &t, input).await?;
            input = merged;
            applied = Some(ap);
        }
        if let Some(c) = &input.color {
            validate_color(c)?;
        }
        if let Some(p) = input.priority {
            validate_priority(p)?;
        }
        let summary = input.summary.as_deref().map(validate_summary).transpose()?;
        let (project_id, open_issue_count) = match input.id.clone() {
            None => {
                let id = self
                    .create_project_in(&mut *tx, &snap, ctx, &input, summary, now)
                    .await?;
                if let Some(ap) = applied {
                    super::template::apply_project_children(self, &mut *tx, &snap, ctx, id, ap)
                        .await?;
                }
                (id, None)
            }
            Some(reference) => {
                self.update_project_in(&mut *tx, &snap, ctx, &reference, &input, summary, now)
                    .await?
            }
        };
        let project = tx
            .projects()
            .get(project_id)
            .await?
            .ok_or_else(|| AppError::internal("project missing after save"))?;
        // Die Sicht nach dem Speichern: der Akteur sieht das Projekt über seine Teams.
        let mut views = build_views(
            &mut *tx,
            &snap,
            vec![project],
            ViewOpts {
                members: true,
                milestones: true,
            },
            now,
        )
        .await?;
        let mut view = views
            .pop()
            .ok_or_else(|| AppError::internal("project view missing"))?;
        view.open_issue_count = open_issue_count;
        tx.commit().await?;
        Ok(view)
    }

    async fn create_project_in(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        input: &SaveProjectInput,
        summary: Option<Option<String>>,
        now: DateTime<Utc>,
    ) -> AppResult<ProjectId> {
        if input.patch.is_some() {
            return Err(AppError::invalid_request(
                "`patch` cannot be used when creating a project",
            )
            .with_details(json!({ "field": "patch" })));
        }
        let name = validate_name(input.name.as_deref().unwrap_or(""))?;
        let mut team_refs: Vec<String> = input.set_teams.clone().unwrap_or_default();
        team_refs.extend(input.add_teams.iter().cloned());
        if team_refs.is_empty() {
            return Err(AppError::invalid_request(
                "At least one team is required (`addTeams` or `setTeams`).",
            )
            .with_details(json!({ "field": "addTeams" })));
        }
        let mut teams: Vec<Team> = Vec::new();
        for r in &team_refs {
            let t = resolve::team::resolve_in(tx, snap, r, Visibility::Content).await?;
            if !teams.iter().any(|x| x.id == t.id) {
                teams.push(t);
            }
        }
        let lead_team = match &input.lead_team {
            Some(r) => {
                let t = resolve::team::resolve_in(tx, snap, r, Visibility::Content).await?;
                if !teams.iter().any(|x| x.id == t.id) {
                    teams.push(t.clone());
                }
                t
            }
            None => teams[0].clone(),
        };
        // Recht: Zugriff auf alle gewählten Teams (docs/02 §6.4); nicht zugänglich = not_found.
        let ids: Vec<TeamId> = teams.iter().map(|t| t.id).collect();
        snap.require_teams(ctx, Action::ManageProject, &ids)?;
        let statuses = tx.projects().list_statuses().await?;
        let set = pure::effective_status_set(&statuses, &snap.tree, Some(lead_team.id));
        let status = match &input.state {
            Some(s) => pure::resolve_status_ref(&set, s).cloned().ok_or_else(|| {
                AppError::not_found(format!("Could not find project status \"{s}\""))
                    .with_details(json!({ "reference": s, "type": "ProjectStatus" }))
            })?,
            None => set
                .iter()
                .find(|s| s.status_type == ProjectStatusType::Backlog)
                .cloned()
                .ok_or_else(|| AppError::internal("no backlog project status configured"))?,
        };
        let (start, start_res, target, target_res) = pure::resolve_dates(
            input.start_date.flatten(),
            input.start_date_resolution.flatten(),
            input.target_date.flatten(),
            input.target_date_resolution.flatten(),
        )?;
        let lead = match &input.lead {
            Some(Some(r)) => {
                let u = resolve::user::resolve_in(tx, snap, ctx, r).await?;
                check_member_allowed(tx, snap, &u, &ids).await?;
                Some(u)
            }
            _ => None,
        };
        let labels = match &input.labels {
            Some(l) => resolve_labels(tx, snap, l, &ids, &[]).await?,
            None => Vec::new(),
        };
        let init_refs: Vec<String> = input
            .set_initiatives
            .clone()
            .unwrap_or_default()
            .into_iter()
            .chain(input.add_initiatives.iter().cloned())
            .collect();
        let mut initiatives = Vec::new();
        if !init_refs.is_empty() {
            snap.require_workspace(ctx, Action::ManageInitiatives)?;
            for r in &init_refs {
                let i = resolve::initiative::resolve_in(tx, snap, r).await?;
                if !initiatives
                    .iter()
                    .any(|x: &crate::ports::repos::initiative::Initiative| x.id == i.id)
                {
                    initiatives.push(i);
                }
            }
        }
        let number = tx.teams().next_project_number(lead_team.id).await?;
        let id = ProjectId(self.ids.new_uuid());
        let ts = status_type_changes(
            ProjectStatusType::Backlog,
            status.status_type,
            ProjectTimestamps::default(),
            now,
        );
        let creator = resolve::user::actor_user_id(ctx);
        let content = input
            .description
            .as_deref()
            .map(lineal_domain::rules::markdown::sanitize)
            .filter(|c| !c.is_empty());
        tx.projects()
            .insert(&NewProject {
                id,
                name,
                lead_team_id: lead_team.id,
                number,
                description: summary.flatten(),
                content,
                color: input.color.clone(),
                icon: input
                    .icon
                    .as_deref()
                    .map(validate_icon)
                    .transpose()?
                    .flatten(),
                status_id: status.id,
                priority: input.priority.unwrap_or(0),
                sort_order: now.timestamp_millis() as f64,
                lead_id: lead.as_ref().map(|u| u.id),
                creator_id: creator,
                start_date: start,
                start_date_resolution: start_res,
                target_date: target,
                target_date_resolution: target_res,
                started_at: ts.started_at,
                completed_at: ts.completed_at,
                canceled_at: ts.canceled_at,
                now,
            })
            .await?;
        for (i, t) in teams.iter().enumerate() {
            tx.projects().add_team(id, t.id, i as f64, now).await?;
        }
        if let Some(l) = &lead {
            tx.projects().add_member(id, l.id, now).await?;
            tx.projects().subscribe(id, l.id, now).await?;
        }
        if !labels.is_empty() {
            tx.projects().set_labels(id, &labels, now).await?;
        }
        for i in &initiatives {
            tx.initiatives().link_project(i.id, id, now).await?;
            emit(
                tx,
                ctx,
                DomainEvent::InitiativeUpdated {
                    initiative_id: i.id,
                },
            )
            .await?;
        }
        for (url, title) in &input.links {
            let (url, title) = validate_link(url, title)?;
            tx.projects()
                .add_link(id, &url, &title, creator, now)
                .await?;
        }
        emit(tx, ctx, DomainEvent::ProjectCreated { project_id: id }).await?;
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_project_in(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        reference: &str,
        input: &SaveProjectInput,
        summary: Option<Option<String>>,
        now: DateTime<Utc>,
    ) -> AppResult<(ProjectId, Option<i64>)> {
        let ResolvedProject {
            project,
            teams: current_teams,
        } = resolve::project::resolve_in(tx, snap, reference).await?;
        require_manage(snap, ctx, Action::ManageProject, &current_teams)?;
        writable(&project)?;
        let mut patch = ProjectPatch::default();
        let mut changed = false;
        let old_key = snap
            .tree
            .get(project.lead_team_id)
            .map(|t| t.key.clone())
            .unwrap_or_default();

        // --- Teams -------------------------------------------------------------------------
        let mut final_teams: Vec<TeamId> = current_teams.clone();
        let mut added: Vec<TeamId> = Vec::new();
        let mut removed: Vec<TeamId> = Vec::new();
        let teams_touched = input.set_teams.is_some()
            || !input.add_teams.is_empty()
            || !input.remove_teams.is_empty()
            || input.lead_team.is_some();
        let mut new_lead = project.lead_team_id;
        if teams_touched {
            if let Some(set) = &input.set_teams {
                let mut target = Vec::new();
                for r in set {
                    target.push(resolve_team_id(tx, snap, r).await?);
                }
                let target = dedupe(target);
                for t in &target {
                    if !final_teams.contains(t) {
                        added.push(*t);
                    }
                }
                for t in &final_teams {
                    if !target.contains(t) {
                        removed.push(*t);
                    }
                }
                final_teams = target;
            } else {
                for r in &input.add_teams {
                    let t = resolve_team_id(tx, snap, r).await?;
                    if !final_teams.contains(&t) {
                        final_teams.push(t);
                        added.push(t);
                    }
                }
                for r in &input.remove_teams {
                    let t = resolve_team_id(tx, snap, r).await?;
                    if let Some(pos) = final_teams.iter().position(|x| *x == t) {
                        final_teams.remove(pos);
                        removed.push(t);
                    }
                }
            }
            if let Some(r) = &input.lead_team {
                new_lead = resolve_team_id(tx, snap, r).await?;
                if !final_teams.contains(&new_lead) {
                    final_teams.push(new_lead);
                    added.push(new_lead);
                    removed.retain(|t| *t != new_lead);
                }
            }
            if final_teams.is_empty() {
                return Err(AppError::invalid_request(
                    "The last team cannot be removed from a project.",
                )
                .with_details(json!({ "reason": "last_team" })));
            }
            if !final_teams.contains(&new_lead) {
                return Err(AppError::invalid_request(
                    "The lead team cannot be removed unless a new `leadTeam` is set.",
                )
                .with_details(json!({ "reason": "lead_team_removed" })));
            }
            if !added.is_empty() {
                snap.require_teams(ctx, Action::ManageProject, &added)?;
            }
            for t in &removed {
                let issues = tx.projects().issues_of_team(project.id, *t, 10).await?;
                if !issues.is_empty() && !input.detach_issues_of_removed_teams {
                    return Err(AppError::invalid_request(
                        "The team still has issues in the project.",
                    )
                    .with_details(json!({
                        "reason": "team_has_issues",
                        "team": t.to_string(),
                        "issues": issues.iter().map(|(_, ident)| ident).collect::<Vec<_>>(),
                    })));
                }
            }
        }

        // --- Skalare Felder -------------------------------------------------------------------
        if let Some(n) = &input.name {
            patch.name = Some(validate_name(n)?);
            changed = true;
        }
        if let Some(s) = summary {
            patch.description = Some(s);
            changed = true;
        }
        if input.description.is_some() && input.patch.is_some() {
            return Err(AppError::invalid_request(
                "`description` and `patch` cannot be combined",
            ));
        }
        if let Some(d) = &input.description {
            let clean = lineal_domain::rules::markdown::sanitize(d);
            patch.content = Some((!clean.is_empty()).then_some(clean));
            changed = true;
        }
        if let Some(ops) = &input.patch {
            let base = project.content.clone().unwrap_or_default();
            let next = patch::apply(&base, ops).map_err(pure::patch_error)?;
            let clean = lineal_domain::rules::markdown::sanitize(&next);
            patch.content = Some((!clean.is_empty()).then_some(clean));
            changed = true;
        }
        if let Some(c) = &input.color {
            patch.color = Some(c.clone());
            changed = true;
        }
        if let Some(i) = &input.icon {
            patch.icon = Some(validate_icon(i)?);
            changed = true;
        }
        if let Some(p) = input.priority {
            patch.priority = Some(p);
            changed = true;
        }

        // --- Lead-Team-Wechsel: neue Nummer, Alias, Status neu auflösen --------------------------
        let statuses = tx.projects().list_statuses().await?;
        let current_status = statuses
            .iter()
            .find(|s| s.id == project.status_id)
            .cloned()
            .ok_or_else(|| AppError::internal("project status missing"))?;
        let mut target_status: ProjectStatus = current_status.clone();
        if new_lead != project.lead_team_id {
            let number = tx.teams().next_project_number(new_lead).await?;
            tx.projects()
                .add_alias(
                    project.id,
                    &format!("P-{}-{}", old_key, project.number),
                    &old_key,
                )
                .await?;
            patch.lead_team_id = Some(new_lead);
            patch.number = Some(number);
            changed = true;
            let new_set = pure::effective_status_set(&statuses, &snap.tree, Some(new_lead));
            if let Some(s) = pure::remap_status(&current_status, &new_set) {
                target_status = s.clone();
            }
        }
        let lead_for_status = new_lead;
        if let Some(s) = &input.state {
            let set = pure::effective_status_set(&statuses, &snap.tree, Some(lead_for_status));
            target_status = pure::resolve_status_ref(&set, s).cloned().ok_or_else(|| {
                AppError::not_found(format!("Could not find project status \"{s}\""))
                    .with_details(json!({ "reference": s, "type": "ProjectStatus" }))
            })?;
        }
        let mut open_issue_count = None;
        if target_status.id != project.status_id {
            patch.status_id = Some(target_status.id);
            changed = true;
            if target_status.status_type != current_status.status_type {
                let ts = status_type_changes(
                    current_status.status_type,
                    target_status.status_type,
                    ProjectTimestamps {
                        started_at: project.started_at,
                        completed_at: project.completed_at,
                        canceled_at: project.canceled_at,
                    },
                    now,
                );
                if ts.started_at != project.started_at {
                    patch.started_at = Some(ts.started_at);
                }
                if ts.completed_at != project.completed_at {
                    patch.completed_at = Some(ts.completed_at);
                }
                if ts.canceled_at != project.canceled_at {
                    patch.canceled_at = Some(ts.canceled_at);
                }
                if matches!(
                    target_status.status_type,
                    ProjectStatusType::Completed | ProjectStatusType::Canceled
                ) {
                    open_issue_count = Some(tx.projects().open_issue_count(project.id).await?);
                }
            }
        }

        // --- Daten mit Auflösung ------------------------------------------------------------------
        if input.start_date.is_some()
            || input.target_date.is_some()
            || input.start_date_resolution.is_some()
            || input.target_date_resolution.is_some()
        {
            let start = input.start_date.unwrap_or(project.start_date);
            let start_res = input
                .start_date_resolution
                .unwrap_or(project.start_date_resolution);
            let target = input.target_date.unwrap_or(project.target_date);
            let target_res = input
                .target_date_resolution
                .unwrap_or(project.target_date_resolution);
            // Wird ein Datum geleert, wird auch die Auflösung geleert.
            let start_res = if start.is_none() { None } else { start_res };
            let target_res = if target.is_none() { None } else { target_res };
            let (s, sr, t, tr) = pure::resolve_dates(start, start_res, target, target_res)?;
            patch.start_date = Some(s);
            patch.start_date_resolution = Some(sr);
            patch.target_date = Some(t);
            patch.target_date_resolution = Some(tr);
            changed = true;
        }

        // --- Lead -----------------------------------------------------------------------------------
        let mut new_lead_user: Option<UserId> = None;
        if let Some(l) = &input.lead {
            match l {
                None => patch.lead_id = Some(None),
                Some(r) => {
                    let u = resolve::user::resolve_in(tx, snap, ctx, r).await?;
                    check_member_allowed(tx, snap, &u, &final_teams).await?;
                    patch.lead_id = Some(Some(u.id));
                    new_lead_user = Some(u.id);
                }
            }
            changed = true;
        }

        // --- Labels --------------------------------------------------------------------------------
        let existing_labels = tx.projects().labels_of(project.id).await?;
        let mut label_rows: Option<Vec<ProjectLabelRow>> = None;
        if let Some(refs) = &input.labels {
            label_rows =
                Some(resolve_labels(tx, snap, refs, &final_teams, &existing_labels).await?);
        } else if teams_touched && !removed.is_empty() {
            // Labels, die nur über entfernte Teams verfügbar waren, werden entfernt.
            let mut available: BTreeSet<TeamId> = final_teams.iter().copied().collect();
            for t in &final_teams {
                available.extend(snap.tree.ancestors(*t).iter().map(|a| a.id));
            }
            let keep: Vec<ProjectLabelRow> = existing_labels
                .iter()
                .filter(|l| l.team_id.is_none_or(|t| available.contains(&t)))
                .cloned()
                .collect();
            if keep.len() != existing_labels.len() {
                label_rows = Some(keep);
            }
        }

        // --- Anwenden ---------------------------------------------------------------------------------
        if changed {
            tx.projects().update(project.id, &patch, now).await?;
        }
        for t in &added {
            tx.projects()
                .add_team(project.id, *t, final_teams.len() as f64, now)
                .await?;
        }
        for t in &removed {
            let detached = tx
                .projects()
                .detach_team_issues(project.id, *t, resolve::user::actor_user_id(ctx), now)
                .await?;
            for issue in detached {
                emit_issue_detached(tx, ctx, issue, *t).await?;
            }
        }
        for t in &removed {
            tx.projects().remove_team(project.id, *t).await?;
        }
        if let Some(rows) = &label_rows {
            tx.projects().set_labels(project.id, rows, now).await?;
            changed = true;
        }
        if let Some(l) = new_lead_user {
            tx.projects().add_member(project.id, l, now).await?;
            tx.projects().subscribe(project.id, l, now).await?;
        }
        if !added.is_empty() || !removed.is_empty() {
            changed = true;
        }
        for (url, title) in &input.links {
            let (url, title) = validate_link(url, title)?;
            tx.projects()
                .add_link(
                    project.id,
                    &url,
                    &title,
                    resolve::user::actor_user_id(ctx),
                    now,
                )
                .await?;
            changed = true;
        }
        changed |= self
            .apply_initiative_links(tx, snap, ctx, project.id, input, now)
            .await?;
        if changed {
            if patch == ProjectPatch::default() {
                // Nur Beziehungen geändert: `updated_at`/`version` trotzdem erhöhen.
                tx.projects()
                    .update(project.id, &ProjectPatch::default(), now)
                    .await?;
            }
            emit(
                tx,
                ctx,
                DomainEvent::ProjectUpdated {
                    project_id: project.id,
                },
            )
            .await?;
        }
        Ok((project.id, open_issue_count))
    }

    /// `setInitiatives`/`addInitiatives`/`removeInitiatives`; liefert, ob sich etwas geändert hat.
    async fn apply_initiative_links(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        project: ProjectId,
        input: &SaveProjectInput,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        if input.set_initiatives.is_none()
            && input.add_initiatives.is_empty()
            && input.remove_initiatives.is_empty()
        {
            return Ok(false);
        }
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        let mut changed = false;
        let current: Vec<InitiativeId> =
            tx.initiatives().initiative_ids_of_project(project).await?;
        let (to_add, to_remove): (Vec<InitiativeId>, Vec<InitiativeId>) =
            if let Some(set) = &input.set_initiatives {
                let mut target = Vec::new();
                for r in set {
                    target.push(resolve::initiative::resolve_in(tx, snap, r).await?.id);
                }
                let target = dedupe(target);
                (
                    target
                        .iter()
                        .copied()
                        .filter(|t| !current.contains(t))
                        .collect(),
                    current
                        .iter()
                        .copied()
                        .filter(|c| !target.contains(c))
                        .collect(),
                )
            } else {
                let mut add = Vec::new();
                for r in &input.add_initiatives {
                    add.push(resolve::initiative::resolve_in(tx, snap, r).await?.id);
                }
                let mut rem = Vec::new();
                for r in &input.remove_initiatives {
                    rem.push(resolve::initiative::resolve_in(tx, snap, r).await?.id);
                }
                (dedupe(add), dedupe(rem))
            };
        for i in to_add {
            if tx.initiatives().link_project(i, project, now).await? {
                emit(tx, ctx, DomainEvent::InitiativeUpdated { initiative_id: i }).await?;
                changed = true;
            }
        }
        for i in to_remove {
            if tx.initiatives().unlink_project(i, project).await? {
                emit(tx, ctx, DomainEvent::InitiativeUpdated { initiative_id: i }).await?;
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Archiviert bzw. stellt ein Projekt wieder her (nur Admins und Lead, docs/04 §2.12).
    pub async fn set_project_archived(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        archived: bool,
    ) -> AppResult<ProjectView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = resolve::project::resolve_in(&mut *tx, &snap, reference).await?;
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        let is_lead =
            r.project.lead_id.is_some() && r.project.lead_id == resolve::user::actor_user_id(ctx);
        if !(snap.actor.is_admin() || snap.actor.is_system() || is_lead) {
            return Err(AppError::forbidden(
                "Only admins and the project lead can archive a project.",
            )
            .with_details(json!({ "reason": "role" })));
        }
        let now = self.clock.now();
        tx.projects()
            .update(
                r.project.id,
                &ProjectPatch {
                    archived_at: Some(archived.then_some(now)),
                    ..Default::default()
                },
                now,
            )
            .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::ProjectUpdated {
                project_id: r.project.id,
            },
        )
        .await?;
        let project = tx
            .projects()
            .get(r.project.id)
            .await?
            .ok_or_else(|| AppError::internal("project missing"))?;
        let mut views =
            build_views(&mut *tx, &snap, vec![project], ViewOpts::default(), now).await?;
        tx.commit().await?;
        views
            .pop()
            .ok_or_else(|| AppError::internal("project view missing"))
    }

    /// Löscht ein Projekt (Papierkorb, 30 Tage) bzw. stellt es wieder her.
    pub async fn set_project_trashed(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        trashed: bool,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let r = if trashed {
            resolve::project::resolve_in(&mut *tx, &snap, reference).await?
        } else {
            // Wiederherstellen: das Projekt liegt im Papierkorb; nur per UUID auffindbar.
            let id = Uuid::parse_str(reference.trim())
                .map_err(|_| AppError::not_found("Trashed projects can only be restored by id."))?;
            let p = tx
                .projects()
                .get(ProjectId(id))
                .await?
                .filter(|p| p.trashed)
                .ok_or_else(|| AppError::not_found("Could not find the trashed project."))?;
            let teams = tx.projects().list_teams(p.id).await?;
            if !teams.iter().any(|t| snap.visible_team_ids().contains(t)) {
                return Err(AppError::not_found("Could not find the trashed project."));
            }
            ResolvedProject { project: p, teams }
        };
        require_manage(&snap, ctx, Action::ManageProject, &r.teams)?;
        let now = self.clock.now();
        tx.projects()
            .update(
                r.project.id,
                &ProjectPatch {
                    trashed: Some(trashed),
                    ..Default::default()
                },
                now,
            )
            .await?;
        let ev = if trashed {
            DomainEvent::ProjectRemoved {
                project_id: r.project.id,
            }
        } else {
            DomainEvent::ProjectUpdated {
                project_id: r.project.id,
            }
        };
        emit(&mut *tx, ctx, ev).await?;
        tx.commit().await
    }
}

/// `IssueUpdated`-Event, wenn ein Issue vom Projekt gelöst wurde.
pub(crate) async fn emit_issue_detached(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    issue: IssueId,
    team: TeamId,
) -> AppResult<()> {
    emit(
        tx,
        ctx,
        DomainEvent::IssueUpdated {
            issue_id: issue,
            team_id: team,
            changed: vec!["projectId".into(), "projectMilestoneId".into()],
            mentioned_user_ids: vec![],
        },
    )
    .await
}

use super::initiative;
