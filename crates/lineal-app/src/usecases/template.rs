//! usecases – Templates (docs/06 §2–3): Verwaltung (Workspace/Team, Vererbung an Sub-Teams),
//! Listen mit Auflösungsreihenfolge, `applyTemplate` für Issues (inkl. Sub-Issues in einer
//! Transaktion), Projekte und Dokumente, Default-Templates der Teams.
//!
//! Struktur: [`data`] (Aufbau und Validierung von `templateData`), `apply` (Merge-Regeln und
//! Anlegen der Kinder). Dieses Modul enthält die Use-Cases der Verwaltung.

mod apply;
pub mod data;

pub use apply::{TemplateIssueOutcome, null_field_set, strip_placeholders};
pub(crate) use apply::{
    apply_project_children, create_issue_tree, merge_document, merge_project,
    prepare_issue_template,
};

mod form;

pub use form::{FormAnswer, RenderedForm, render_form};

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot, TeamTree};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::template::{DefaultSlot, NewTemplate, Template};
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::actor_user_id;
use crate::usecases::issue::CreateIssueInput;
use lineal_domain::enums::TemplateType;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Eine Vorlage mit Anzeigewerten.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateView {
    pub template: Template,
    /// Team `(id, key, name)`; `None` = Workspace-Template.
    pub team: Option<(TeamId, String, String)>,
    /// `true`, wenn es ein Form-Template ist (`formFields` in `templateData`).
    pub is_form: bool,
}

/// Eingabe für Anlegen und Ändern von Templates.
#[derive(Debug, Clone, Default)]
pub struct SaveTemplateInput {
    /// Referenz des bestehenden Templates (Update).
    pub id: Option<String>,
    /// Nur beim Anlegen; danach unveränderlich.
    pub template_type: Option<TemplateType>,
    pub name: Option<String>,
    /// Leerer String entfernt Beschreibung/Icon/Farbe.
    pub description: Option<String>,
    /// Nur beim Anlegen (`None` = Workspace-Template); danach unveränderlich.
    pub team: Option<String>,
    pub template_data: Option<Value>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub sort_order: Option<f64>,
    /// Beim Update: Sub-Team, das ein geerbtes Template anpasst – es entsteht eine Kopie mit
    /// `inheritedFrom` (docs/06 §2.3).
    pub as_team: Option<String>,
}

fn not_found(reference: &str) -> AppError {
    AppError::not_found(format!(
        "Could not find template \"{reference}\". Call list_templates to see what is available"
    ))
    .with_details(json!({ "reference": reference, "type": "Template" }))
}

fn invalid(field: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field }))
}

/// Ob die Vorlage im Kontext der Teams anwendbar ist: Workspace-Templates überall, Team-Templates
/// im Team selbst und in allen Sub-Teams (rekursiv, ohne Kopie).
pub fn available_in(tree: &TeamTree, tpl: &Template, teams: &[TeamId]) -> bool {
    match tpl.team_id {
        None => true,
        Some(t) => teams
            .iter()
            .any(|x| *x == t || tree.ancestors(*x).iter().any(|a| a.id == t)),
    }
}

fn visible_to_actor(snap: &PolicySnapshot, tpl: &Template) -> bool {
    tpl.team_id
        .is_none_or(|t| snap.visible_team_ids().contains(&t))
}

fn is_form(tpl: &Template) -> bool {
    tpl.template_data.get("formFields").is_some()
}

/// Lädt eine Vorlage per UUID oder Name (nur für den Akteur sichtbare; archivierte nie).
/// `ty` begrenzt die Art (Typkonflikt per UUID → `invalid_request`).
pub(crate) async fn load_template(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    reference: &str,
    ty: Option<TemplateType>,
) -> AppResult<Template> {
    let r = reference.trim();
    if let Ok(u) = Uuid::parse_str(r) {
        let t = tx
            .templates()
            .get(TemplateId(u))
            .await?
            .filter(|t| t.archived_at.is_none() && visible_to_actor(snap, t))
            .ok_or_else(|| not_found(reference))?;
        if let Some(want) = ty
            && t.template_type != want
        {
            return Err(AppError::invalid_request(format!(
                "the template is a {} template, a {} template is required",
                t.template_type.as_str(),
                want.as_str()
            ))
            .with_details(json!({ "field": "template", "reason": "template_type_mismatch" })));
        }
        return Ok(t);
    }
    let mut found: Vec<Template> = tx
        .templates()
        .find_all_by_name(r)
        .await?
        .into_iter()
        .filter(|t| visible_to_actor(snap, t))
        .filter(|t| ty.is_none_or(|want| t.template_type == want))
        .collect();
    match found.len() {
        0 => Err(not_found(reference)),
        1 => Ok(found.remove(0)),
        _ => Err(AppError::ambiguous_reference(format!(
            "Template name \"{reference}\" matches several templates."
        ))
        .with_details(json!({
            "reference": reference,
            "type": "Template",
            "candidates": found.iter().take(10).map(|t| json!({
                "id": t.id.to_string(), "name": t.name,
                "team": t.team_id.and_then(|id| snap.tree.get(id)).map(|x| x.key.clone()),
            })).collect::<Vec<_>>(),
        }))),
    }
}

fn view(snap: &PolicySnapshot, t: Template) -> TemplateView {
    let team = t
        .team_id
        .and_then(|id| snap.tree.get(id))
        .map(|x| (x.id, x.key.clone(), x.name.clone()));
    let is_form = is_form(&t);
    TemplateView {
        template: t,
        team,
        is_form,
    }
}

fn require_manage(snap: &PolicySnapshot, ctx: &RequestCtx, team: Option<TeamId>) -> AppResult<()> {
    match team {
        None => snap.require_workspace(ctx, Action::ManageWorkspaceTemplates),
        Some(t) => snap
            .require_team(ctx, Action::ManageTeamTemplates, t)
            .map(|_| ()),
    }
}

async fn emit(tx: &mut dyn Tx, ctx: &RequestCtx, event: DomainEvent) -> AppResult<()> {
    tx.outbox().push(NewEvent::from_ctx(event, ctx)).await?;
    Ok(())
}

fn validate_name(name: &str) -> AppResult<String> {
    Ok(lineal_domain::rules::text::validate_single_line_name(
        "name", name, 255,
    )?)
}

fn clean_opt(v: &Option<String>) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn validate_color(c: &str) -> AppResult<()> {
    let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(invalid("color", "color must be #RRGGBB"))
    }
}

impl App {
    /// `list_templates`: mit `team` die Vorlagen des Teams (eigene, geerbte, Workspace), sonst alle
    /// für den Akteur sichtbaren (Workspace plus sichtbare Teams).
    pub async fn list_templates(
        &self,
        ctx: &RequestCtx,
        team: Option<&str>,
        ty: Option<TemplateType>,
    ) -> AppResult<Vec<TemplateView>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = match team {
            Some(r) => {
                Some(resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Content).await?)
            }
            None => None,
        };
        let all: Vec<Template> = tx
            .templates()
            .list_active()
            .await?
            .into_iter()
            .filter(|t| ty.is_none_or(|want| t.template_type == want))
            .filter(|t| visible_to_actor(&snap, t))
            .collect();
        let mut rows: Vec<(usize, Template)> = Vec::new();
        match &team {
            Some(team) => {
                let chain: Vec<TeamId> = std::iter::once(team.id)
                    .chain(snap.tree.ancestors(team.id).iter().map(|t| t.id))
                    .collect();
                // Kopien (`inheritedFrom`) verdecken ihr Original.
                let shadowed: BTreeSet<TemplateId> = all
                    .iter()
                    .filter(|t| t.team_id.is_some_and(|x| chain.contains(&x)))
                    .filter_map(|t| t.inherited_from_id)
                    .collect();
                for t in all {
                    if shadowed.contains(&t.id) {
                        continue;
                    }
                    let rank = match t.team_id {
                        None => Some(chain.len()),
                        Some(x) => chain.iter().position(|c| *c == x),
                    };
                    if let Some(r) = rank {
                        rows.push((r, t));
                    }
                }
            }
            None => {
                for t in all {
                    let rank = usize::from(t.team_id.is_some());
                    rows.push((rank, t));
                }
            }
        }
        rows.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.sort_order.total_cmp(&b.1.sort_order))
                .then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()))
        });
        let out = rows.into_iter().map(|(_, t)| view(&snap, t)).collect();
        tx.commit().await?;
        Ok(out)
    }

    /// `get_template`: per UUID oder Name; nicht sichtbar → `not_found`.
    pub async fn get_template(&self, ctx: &RequestCtx, reference: &str) -> AppResult<TemplateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let t = load_template(&mut *tx, &snap, reference, None).await?;
        tx.commit().await?;
        Ok(view(&snap, t))
    }

    /// `templateCreate` / `templateUpdate` (Typ und Team sind nach dem Anlegen unveränderlich).
    pub async fn save_template(
        &self,
        ctx: &RequestCtx,
        input: SaveTemplateInput,
    ) -> AppResult<TemplateView> {
        crate::error::with_retry(|| self.save_template_once(ctx, input.clone())).await
    }

    async fn save_template_once(
        &self,
        ctx: &RequestCtx,
        input: SaveTemplateInput,
    ) -> AppResult<TemplateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let actor = actor_user_id(ctx);
        if let Some(c) = clean_opt(&input.color) {
            validate_color(&c)?;
        }
        if let Some(i) = clean_opt(&input.icon) {
            crate::usecases::document::validate_icon(&i)?;
        }
        if let Some(s) = input.sort_order
            && !s.is_finite()
        {
            return Err(invalid("sortOrder", "sortOrder must be a finite number"));
        }
        let saved = match &input.id {
            None => {
                let ty = input
                    .template_type
                    .ok_or_else(|| invalid("type", "type is required when creating a template"))?;
                let name = validate_name(input.name.as_deref().unwrap_or(""))?;
                let team = match &input.team {
                    Some(r) => Some(
                        resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Content).await?,
                    ),
                    None => None,
                };
                require_manage(&snap, ctx, team.as_ref().map(|t| t.id))?;
                let data = input.template_data.clone().unwrap_or_else(|| json!({}));
                let data =
                    data::validate_and_normalize(&mut *tx, &snap, ctx, ty, team.as_ref(), &data)
                        .await?;
                self.ensure_unique_name(&mut *tx, team.as_ref().map(|t| t.id), ty, &name, None)
                    .await?;
                let sort_order = match input.sort_order {
                    Some(s) => s,
                    None => {
                        // Eindeutig auch bei paralleler Anlage im selben Scope (Millisekunde).
                        let scope = team.as_ref().map(|t| t.id);
                        tx.advisory_lock("template_sort", scope.map(|t| t.0))
                            .await?;
                        let max = tx
                            .templates()
                            .list_active()
                            .await?
                            .into_iter()
                            .filter(|t| t.team_id == scope)
                            .map(|t| t.sort_order)
                            .fold(f64::MIN, f64::max);
                        (now.timestamp_millis() as f64).max(max + 1.0)
                    }
                };
                let t = tx
                    .templates()
                    .insert(&NewTemplate {
                        id: TemplateId(self.ids.new_uuid()),
                        template_type: ty,
                        name,
                        description: clean_opt(&input.description),
                        icon: clean_opt(&input.icon),
                        color: clean_opt(&input.color),
                        team_id: team.as_ref().map(|t| t.id),
                        template_data: data,
                        sort_order,
                        inherited_from_id: None,
                        creator_id: actor,
                        now,
                    })
                    .await?;
                emit(
                    &mut *tx,
                    ctx,
                    DomainEvent::TemplateCreated { template_id: t.id },
                )
                .await?;
                t
            }
            Some(reference) => {
                if input.team.is_some() || input.template_type.is_some() {
                    return Err(invalid(
                        "type",
                        "type and team cannot be changed after creation",
                    ));
                }
                let found = load_template(&mut *tx, &snap, reference, None).await?;
                // Serialisieren und frischen Stand lesen (Lost Update bei parallelen Änderungen).
                tx.advisory_lock("template", Some(found.id.0)).await?;
                let mut t = load_template(&mut *tx, &snap, &found.id.to_string(), None).await?;
                // Anpassung eines geerbten Templates durch ein Sub-Team: Kopie mit `inheritedFrom`.
                if let Some(as_team) = &input.as_team {
                    let sub =
                        resolve::team::resolve_in(&mut *tx, &snap, as_team, Visibility::Content)
                            .await?;
                    let inherited = t.team_id.is_some_and(|x| {
                        x != sub.id && snap.tree.ancestors(sub.id).iter().any(|a| a.id == x)
                    });
                    if !inherited {
                        return Err(invalid(
                            "asTeam",
                            "only a template inherited from an ancestor team can be customised",
                        ));
                    }
                    require_manage(&snap, ctx, Some(sub.id))?;
                    let name = validate_name(input.name.as_deref().unwrap_or(&t.name))?;
                    self.ensure_unique_name(&mut *tx, Some(sub.id), t.template_type, &name, None)
                        .await?;
                    let data = match &input.template_data {
                        Some(d) => {
                            data::validate_and_normalize(
                                &mut *tx,
                                &snap,
                                ctx,
                                t.template_type,
                                Some(&sub),
                                d,
                            )
                            .await?
                        }
                        None => t.template_data.clone(),
                    };
                    let copy = tx
                        .templates()
                        .insert(&NewTemplate {
                            id: TemplateId(self.ids.new_uuid()),
                            template_type: t.template_type,
                            name,
                            description: match &input.description {
                                Some(_) => clean_opt(&input.description),
                                None => t.description.clone(),
                            },
                            icon: match &input.icon {
                                Some(_) => clean_opt(&input.icon),
                                None => t.icon.clone(),
                            },
                            color: match &input.color {
                                Some(_) => clean_opt(&input.color),
                                None => t.color.clone(),
                            },
                            team_id: Some(sub.id),
                            template_data: data,
                            sort_order: input.sort_order.unwrap_or(t.sort_order),
                            inherited_from_id: Some(t.id),
                            creator_id: actor,
                            now,
                        })
                        .await?;
                    emit(
                        &mut *tx,
                        ctx,
                        DomainEvent::TemplateCreated {
                            template_id: copy.id,
                        },
                    )
                    .await?;
                    let v = view(&snap, copy);
                    tx.commit().await?;
                    return Ok(v);
                }
                require_manage(&snap, ctx, t.team_id)?;
                if let Some(n) = &input.name {
                    let n = validate_name(n)?;
                    if !n.eq_ignore_ascii_case(&t.name) {
                        self.ensure_unique_name(
                            &mut *tx,
                            t.team_id,
                            t.template_type,
                            &n,
                            Some(t.id),
                        )
                        .await?;
                    }
                    t.name = n;
                }
                if input.description.is_some() {
                    t.description = clean_opt(&input.description);
                }
                if input.icon.is_some() {
                    t.icon = clean_opt(&input.icon);
                }
                if input.color.is_some() {
                    t.color = clean_opt(&input.color);
                }
                if let Some(s) = input.sort_order {
                    t.sort_order = s;
                }
                if let Some(d) = &input.template_data {
                    let team = match t.team_id {
                        Some(id) => snap.tree.get(id).cloned(),
                        None => None,
                    };
                    t.template_data = data::validate_and_normalize(
                        &mut *tx,
                        &snap,
                        ctx,
                        t.template_type,
                        team.as_ref(),
                        d,
                    )
                    .await?;
                }
                let t = tx.templates().update(&t, actor, now).await?;
                emit(
                    &mut *tx,
                    ctx,
                    DomainEvent::TemplateUpdated { template_id: t.id },
                )
                .await?;
                t
            }
        };
        let v = view(&snap, saved);
        tx.commit().await?;
        Ok(v)
    }

    async fn ensure_unique_name(
        &self,
        tx: &mut dyn Tx,
        team: Option<TeamId>,
        ty: TemplateType,
        name: &str,
        except: Option<TemplateId>,
    ) -> AppResult<()> {
        if let Some(other) = tx.templates().find_by_name(team, ty, name).await?
            && Some(other.id) != except
        {
            return Err(AppError::conflict(format!(
                "A {} template named \"{name}\" already exists in this scope.",
                ty.as_str()
            ))
            .with_details(json!({ "field": "name" })));
        }
        Ok(())
    }

    /// `templateDelete`: archiviert die Vorlage; Verweise der Team-Defaults werden geleert.
    pub async fn delete_template(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<TemplateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let t = load_template(&mut *tx, &snap, reference, None).await?;
        require_manage(&snap, ctx, t.team_id)?;
        // Wiederkehrende Issues erzeugen ihre Issues aus dem Template: Löschen nur ohne Regeln.
        let rules = tx.triage().count_recurring_for_template(t.id.0).await?;
        if rules > 0 {
            return Err(AppError::conflict(format!(
                "The template is used by {rules} recurring issue rule(s); delete or change them first."
            ))
            .with_details(json!({ "reason": "template_in_use", "recurringIssues": rules })));
        }
        tx.templates().archive(t.id, self.clock.now()).await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::TemplateRemoved { template_id: t.id },
        )
        .await?;
        tx.commit().await?;
        Ok(view(&snap, t))
    }

    /// Die drei Default-Templates eines Teams (für die Team-Settings).
    pub async fn get_team_default_templates(
        &self,
        ctx: &RequestCtx,
        team: &str,
    ) -> AppResult<crate::ports::repos::template::TeamDefaults> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Content).await?;
        let d = tx.templates().team_defaults(team.id).await?;
        tx.commit().await?;
        Ok(d)
    }

    /// Setzt oder löscht ein Default-Template des Teams (docs/06 §2.8).
    pub async fn set_team_default_template(
        &self,
        ctx: &RequestCtx,
        team: &str,
        slot: DefaultSlot,
        template: Option<&str>,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team = resolve::team::resolve_in(&mut *tx, &snap, team, Visibility::Content).await?;
        snap.require_team(ctx, Action::ManageTeamTemplates, team.id)?;
        let id = match template {
            None => None,
            Some(r) => {
                let want = if slot == DefaultSlot::Project {
                    TemplateType::Project
                } else {
                    TemplateType::Issue
                };
                let t = load_template(&mut *tx, &snap, r, Some(want)).await?;
                if !available_in(&snap.tree, &t, &[team.id]) {
                    return Err(invalid(
                        "template",
                        "the template is not available in this team",
                    ));
                }
                if is_form(&t) && slot != DefaultSlot::NonMembers {
                    return Err(invalid(
                        "template",
                        "form templates are only allowed as default for non-members",
                    ));
                }
                Some(t.id)
            }
        };
        tx.templates().set_team_default(team.id, slot, id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Legt ein Issue aus einem Template an (docs/06 §2.5/§2.6): Wurzel-Issue und alle Sub-Issues in
    /// einer Transaktion über [`App::create_issue_in`], Nummern lückenlos in Tiefensuche-Reihenfolge.
    ///
    /// `null_fields` nennt Felder, die der Aufrufer ausdrücklich als `null` übergeben hat
    /// (`state`, `priority`, `estimate`, `assignee`, `dueDate`, `project`, `milestone`, `cycle`):
    /// sie überschreiben den Template-Wert mit „nicht setzen“.
    pub async fn create_issue_from_template(
        &self,
        ctx: &RequestCtx,
        template: &str,
        input: CreateIssueInput,
        null_fields: &[String],
    ) -> AppResult<TemplateIssueOutcome> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (tpl, team, data) =
            prepare_issue_template(&mut *tx, &snap, ctx, template, &input).await?;
        let nulls: BTreeSet<String> = null_fields.iter().cloned().collect();
        let outcome = create_issue_tree(
            self,
            &mut *tx,
            &snap,
            ctx,
            &team,
            Some(&tpl),
            &data,
            input,
            &nulls,
            false,
        )
        .await?;
        tx.commit().await?;
        Ok(outcome)
    }
}
