//! Anwendung von Templates (docs/06 §2.5–2.7): Merge-Regeln, Sub-Issues in derselben Transaktion
//! über `App::create_issue_in` (Nummern lückenlos in Tiefensuche-Reihenfolge), Projekt-Templates
//! mit Meilensteinen, Issues und Dokumenten.

use super::data::{
    DocData, IssueData, MilestoneData, ProjectData, SubIssueData, parse, parse_relative_days,
};
use super::{available_in, load_template};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::issue::HistorySource;
use crate::ports::repos::project::NewMilestone;
use crate::ports::repos::team::Team;
use crate::ports::repos::template::Template;
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::actor_user_id;
use crate::usecases::document::{ParentRefs, SaveDocumentInput, clean_markdown};
use crate::usecases::issue::{CreateIssueInput, IssueView};
use crate::usecases::project::SaveProjectInput;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use lineal_domain::enums::{DateResolution, TemplateType};
use lineal_domain::ids::*;
use serde_json::json;
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

/// Ergebnis der Anwendung eines Issue-Templates.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateIssueOutcome {
    pub root: IssueView,
    /// Alle Sub-Issues in Tiefensuche-Reihenfolge.
    pub children: Vec<IssueView>,
    /// Nicht auflösbare Werte, die still übersprungen wurden (docs/06 §2.4).
    pub warnings: Vec<String>,
}

/// Entfernt unberührte Platzhalter `{{placeholder: …}}` (docs/06 §2.7).
pub fn strip_placeholders(text: &str) -> String {
    const OPEN: &str = "{{placeholder:";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        match rest[start..].find("}}") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                // Unvollständiger Platzhalter: als Text belassen.
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    // Zeilen, in denen ein Platzhalter stand, verlieren den Leerraum am Zeilenende.
    out.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        + if out.ends_with('\n') { "\n" } else { "" }
}

fn invalid(field: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field }))
}

/// Ergänzt `details.path` (z. B. `subIssues[1]`) an einem Fehler.
fn with_path(mut e: AppError, path: &str) -> AppError {
    match e.details.as_object_mut() {
        Some(o) => {
            o.entry("path").or_insert_with(|| json!(path));
        }
        None => e.details = json!({ "path": path }),
    }
    e
}

fn local_today(team: &Team, now: DateTime<Utc>) -> NaiveDate {
    team.timezone.parse::<Tz>().map_or_else(
        |_| now.date_naive(),
        |tz| now.with_timezone(&tz).date_naive(),
    )
}

fn add_days(date: NaiveDate, days: i64) -> AppResult<NaiveDate> {
    date.checked_add_signed(Duration::days(days))
        .ok_or_else(|| invalid("dueIn", "relative date is out of range"))
}

struct Env<'a> {
    app: &'a App,
    snap: &'a PolicySnapshot,
    ctx: &'a RequestCtx,
    team: &'a Team,
    now: DateTime<Utc>,
}

/// Status-Verweis eines Templates für das Zielteam (docs/06 §2.3): Status des Zielteams direkt,
/// geerbter Status über `inherited_from_id`, sonst Fallback auf die Kategorie.
async fn state_for_team(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    state_id: &str,
    warnings: &mut Vec<String>,
    field: &str,
) -> AppResult<Option<String>> {
    let Ok(uuid) = Uuid::parse_str(state_id) else {
        warnings.push(field.to_owned());
        return Ok(None);
    };
    let Some(s) = tx.states().get(StateId(uuid)).await? else {
        warnings.push(field.to_owned());
        return Ok(None);
    };
    if s.archived_at.is_some() {
        warnings.push(field.to_owned());
        return Ok(None);
    }
    if s.team_id == env.team.id {
        return Ok(Some(s.id.to_string()));
    }
    if env.team.inherit_workflow_statuses {
        for own in tx.states().list_for_team(env.team.id, false).await? {
            if own.inherited_from_id == Some(s.id) {
                return Ok(Some(own.id.to_string()));
            }
        }
    }
    Ok(Some(s.state_type.as_str().to_owned()))
}

/// Labels: Vereinigung, bei `singleSelect`-Gruppen gewinnt das Input-Label (docs/06 §2.5).
async fn merge_labels(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    template_labels: &[String],
    input_labels: &[String],
    warnings: &mut Vec<String>,
) -> AppResult<Vec<String>> {
    let mut from_template = Vec::new();
    for r in template_labels {
        match resolve::label::resolve_for_team(tx, &env.snap.tree, env.team.id, r).await {
            Ok(l) if l.retired_at.is_none() && !l.is_group => from_template.push(l),
            Ok(_) => warnings.push("labelIds".into()),
            Err(e) if e.kind == ErrorKind::NotFound => warnings.push("labelIds".into()),
            Err(e) => return Err(e),
        }
    }
    let mut from_input = Vec::new();
    for r in input_labels {
        from_input
            .push(resolve::label::resolve_for_team(tx, &env.snap.tree, env.team.id, r).await?);
    }
    let mut single_select_groups: BTreeSet<LabelId> = BTreeSet::new();
    for l in &from_input {
        if let Some(g) = l.parent_id
            && let Some(group) = tx
                .labels()
                .get(crate::ports::repos::label::LabelKind::Issue, g)
                .await?
            && group.effective_group_type() == lineal_domain::enums::LabelGroupType::SingleSelect
        {
            single_select_groups.insert(g);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for l in from_template {
        if l.parent_id
            .is_some_and(|g| single_select_groups.contains(&g))
        {
            continue;
        }
        let id = l.id.to_string();
        if !out.contains(&id) {
            out.push(id);
        }
    }
    for l in from_input {
        let id = l.id.to_string();
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// Aktueller bzw. nächster Cycle des Teams.
async fn relative_cycle(tx: &mut dyn Tx, env: &Env<'_>, which: &str) -> AppResult<Option<String>> {
    let cycles = tx.cycles().list_for_team(env.team.id, false).await?;
    let found = if which == "current" {
        cycles
            .iter()
            .find(|c| c.starts_at <= env.now && env.now < c.ends_at)
    } else {
        cycles
            .iter()
            .filter(|c| c.starts_at > env.now)
            .min_by_key(|c| c.starts_at)
    };
    Ok(found.map(|c| c.id.to_string()))
}

/// Baut die Eingabe des Zielobjekts nach den Merge-Regeln (docs/06 §2.5).
async fn build_issue_input(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    data: &IssueData,
    input: &CreateIssueInput,
    null_fields: &BTreeSet<String>,
    ignore_planning: bool,
    warnings: &mut Vec<String>,
) -> AppResult<CreateIssueInput> {
    let mut out = input.clone();
    let is_null = |f: &str| null_fields.contains(f);

    // 1. Titel: Input gewinnt.
    let title = input
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .or_else(|| data.title.clone().filter(|t| !t.trim().is_empty()));
    out.title = Some(title.ok_or_else(|| {
        invalid(
            "title",
            "title is required: neither the input nor the template provides one",
        )
    })?);

    // 2. Beschreibung: nicht leerer Input ersetzt, sonst Template-Text ohne Platzhalter.
    let description = match input.description.as_deref() {
        Some(d) if !d.trim().is_empty() => Some(d.to_owned()),
        _ => data
            .description
            .as_deref()
            .map(strip_placeholders)
            .filter(|s| !s.trim().is_empty()),
    };
    out.description = description.map(|d| clean_markdown(&d)).transpose()?;

    // 3. Skalare Felder: explizit gesetzter Input (auch null) gewinnt.
    if is_null("state") {
        out.state = None;
    } else if input.state.is_none() {
        if let Some(sid) = &data.state_id {
            out.state = state_for_team(tx, env, sid, warnings, "stateId").await?;
        } else if let Some(t) = &data.state_type {
            out.state = Some(t.clone());
        }
    }
    if is_null("priority") {
        out.priority = None;
    } else if input.priority.is_none() {
        out.priority = data.priority;
    }
    if is_null("estimate") {
        out.estimate = None;
    } else if input.estimate.is_none() {
        out.estimate = data.estimate;
    }
    if is_null("assignee") {
        out.assignee = None;
    } else if input.assignee.is_none()
        && let Some(a) = &data.assignee_id
    {
        if a == "creator" {
            out.assignee = actor_user_id(env.ctx).map(|u| u.to_string());
        } else if let Ok(u) = Uuid::parse_str(a)
            && tx.users().get(UserId(u)).await?.is_some_and(|u| u.active)
        {
            out.assignee = Some(a.clone());
        } else {
            warnings.push("assigneeId".into());
        }
    }
    if is_null("dueDate") {
        out.due_date = None;
    } else if input.due_date.is_none() {
        if let Some(d) = &data.due_date {
            out.due_date = NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
        } else if let Some(i) = &data.due_in {
            let days = parse_relative_days("dueIn", i)?;
            out.due_date = Some(add_days(local_today(env.team, env.now), days)?);
        }
    }
    if !ignore_planning {
        if is_null("project") {
            out.project = None;
        } else if input.project.is_none()
            && let Some(p) = &data.project_id
        {
            let ok = match Uuid::parse_str(p) {
                Ok(u) => tx
                    .issues()
                    .get_project(ProjectId(u))
                    .await?
                    .is_some_and(|p| !p.trashed),
                Err(_) => false,
            };
            if ok {
                out.project = Some(p.clone());
            } else {
                warnings.push("projectId".into());
            }
        }
        if is_null("milestone") {
            out.milestone = None;
        } else if input.milestone.is_none()
            && let Some(m) = &data.project_milestone_id
        {
            let ok = match Uuid::parse_str(m) {
                Ok(u) => tx
                    .projects()
                    .get_milestone(MilestoneId(u))
                    .await?
                    .is_some_and(|m| m.archived_at.is_none()),
                Err(_) => false,
            };
            if ok {
                out.milestone = Some(m.clone());
            } else {
                warnings.push("projectMilestoneId".into());
            }
        }
        if is_null("cycle") {
            out.cycle = None;
        } else if input.cycle.is_none()
            && let Some(c) = &data.cycle
        {
            match relative_cycle(tx, env, c).await? {
                Some(id) => out.cycle = Some(id),
                None => warnings.push("cycle".into()),
            }
        }
    } else {
        out.cycle = None;
    }

    // 4. Labels: Vereinigung, singleSelect-Gruppen: Input gewinnt.
    let mut input_labels: Vec<String> = input.labels.clone().unwrap_or_default();
    input_labels.extend(input.add_labels.iter().cloned());
    let template_labels = data.label_ids.clone().unwrap_or_default();
    if !template_labels.is_empty() || input.labels.is_some() || !input.add_labels.is_empty() {
        let merged = merge_labels(tx, env, &template_labels, &input_labels, warnings).await?;
        out.labels = Some(merged);
        out.add_labels = Vec::new();
    }
    Ok(out)
}

/// Legt Wurzel-Issue und Sub-Issues an (eine Transaktion, Tiefensuche-Reihenfolge).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_issue_tree(
    app: &App,
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    team: &Team,
    template: Option<&Template>,
    data: &IssueData,
    input: CreateIssueInput,
    null_fields: &BTreeSet<String>,
    ignore_planning: bool,
) -> AppResult<TemplateIssueOutcome> {
    let now = app.clock.now();
    let env = Env {
        app,
        snap,
        ctx,
        team,
        now,
    };
    let mut warnings = Vec::new();
    let source = input.source;
    let root_input = build_issue_input(
        tx,
        &env,
        data,
        &input,
        null_fields,
        ignore_planning,
        &mut warnings,
    )
    .await?;
    let root = app.create_issue_in(tx, ctx, root_input).await?;
    if let Some(t) = template {
        tx.templates()
            .mark_object(TemplateType::Issue, root.issue.id.0, t.id)
            .await?;
        tx.templates().mark_applied(t.id, now).await?;
    }
    for (i, s) in data.subscriber_ids.iter().flatten().enumerate() {
        match Uuid::parse_str(s) {
            Ok(u) if tx.users().get(UserId(u)).await?.is_some() => {
                tx.issues()
                    .subscribe(app.ids.new_uuid(), root.issue.id, UserId(u), "manual", now)
                    .await?;
            }
            _ => warnings.push(format!("subscriberIds[{i}]")),
        }
    }
    let mut children = Vec::new();
    if let Some(subs) = &data.sub_issues {
        create_subs(
            &env,
            tx,
            &root,
            subs,
            "subIssues",
            source,
            &mut children,
            &mut warnings,
        )
        .await?;
    }
    Ok(TemplateIssueOutcome {
        root,
        children,
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn create_subs<'a>(
    env: &'a Env<'a>,
    tx: &'a mut dyn Tx,
    parent: &'a IssueView,
    subs: &'a [SubIssueData],
    path: &'a str,
    source: HistorySource,
    out: &'a mut Vec<IssueView>,
    warnings: &'a mut Vec<String>,
) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
    Box::pin(async move {
        for (i, s) in subs.iter().enumerate() {
            let p = format!("{path}[{i}]");
            let mut d = IssueData {
                title: Some(s.title.clone()),
                description: s.description.clone(),
                state_id: s.state_id.clone(),
                state_type: s.state_type.clone(),
                priority: s.priority,
                estimate: s.estimate,
                label_ids: s.label_ids.clone(),
                assignee_id: s.assignee_id.clone(),
                due_in: s.due_in.clone(),
                ..Default::default()
            };
            // Weiteres Template ohne eigene Sub-Issues liefert Defaults (docs/06 §2.4).
            if let Some(tid) = &s.template_id {
                let tpl = load_template(tx, env.snap, tid, Some(TemplateType::Issue))
                    .await
                    .map_err(|e| with_path(e, &p))?;
                if !available_in(&env.snap.tree, &tpl, &[env.team.id]) {
                    return Err(with_path(
                        AppError::not_found("Could not find referenced Template."),
                        &p,
                    ));
                }
                let base: IssueData = parse(&tpl.template_data).map_err(|e| with_path(e, &p))?;
                if base.sub_issues.as_ref().is_some_and(|x| !x.is_empty()) {
                    return Err(with_path(
                        invalid(
                            "templateId",
                            "a template referenced from a sub-issue must not contain sub-issues",
                        ),
                        &p,
                    ));
                }
                d.description = d.description.or(base.description);
                d.state_id = d.state_id.or(base.state_id);
                d.state_type = d.state_type.or(base.state_type);
                d.priority = d.priority.or(base.priority);
                d.estimate = d.estimate.or(base.estimate);
                d.label_ids = d.label_ids.or(base.label_ids);
                d.assignee_id = d.assignee_id.or(base.assignee_id);
                d.due_in = d.due_in.or(base.due_in);
            }
            let input = CreateIssueInput {
                team: env.team.id.to_string(),
                parent: Some(parent.issue.id.to_string()),
                // Projekt, Meilenstein und Cycle erbt das Sub-Issue vom Wurzel-Issue.
                project: parent.issue.project_id.map(|x| x.to_string()),
                milestone: parent.issue.project_milestone_id.map(|x| x.to_string()),
                cycle: parent.issue.cycle_id.map(|x| x.to_string()),
                source,
                ..Default::default()
            };
            let built = build_issue_input(tx, env, &d, &input, &BTreeSet::new(), true, warnings)
                .await
                .map_err(|e| with_path(e, &p))?;
            // `ignore_planning` hat den Cycle geleert; Cycle des Wurzel-Issues wieder setzen.
            let built = CreateIssueInput {
                cycle: input.cycle.clone(),
                ..built
            };
            let view = env
                .app
                .create_issue_in(tx, env.ctx, built)
                .await
                .map_err(|e| with_path(e, &p))?;
            out.push(view.clone());
            if let Some(children) = &s.sub_issues {
                create_subs(
                    env,
                    tx,
                    &view,
                    children,
                    &format!("{p}.subIssues"),
                    source,
                    out,
                    warnings,
                )
                .await?;
            }
        }
        Ok(())
    })
}

/// Ergebnis der Dokument-Vorbelegung.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedDocument {
    pub title: Option<String>,
    pub content: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub template_id: TemplateId,
    pub warnings: Vec<String>,
}

/// Dokument-Template anwenden (docs/06 §2.5): Titel/Icon/Farbe gewinnt der Input; nicht leerer
/// Inhalt ersetzt, sonst Template-Inhalt ohne Platzhalter.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn merge_document(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    now: DateTime<Utc>,
    teams: &[TeamId],
    template_ref: &str,
    title: Option<String>,
    content: Option<String>,
    icon: Option<String>,
    color: Option<String>,
) -> AppResult<MergedDocument> {
    let tpl = load_template(tx, snap, template_ref, Some(TemplateType::Document)).await?;
    if !available_in(&snap.tree, &tpl, teams) {
        return Err(AppError::not_found("Could not find referenced Template."));
    }
    let data: DocData = parse(&tpl.template_data)?;
    let content = match content.as_deref() {
        Some(c) if !c.trim().is_empty() => Some(c.to_owned()),
        _ => data.content.as_deref().map(strip_placeholders),
    };
    tx.templates().mark_applied(tpl.id, now).await?;
    Ok(MergedDocument {
        title: title.or(data.title),
        content,
        icon: icon.or(data.icon),
        color: color.or(data.color),
        template_id: tpl.id,
        warnings: vec![],
    })
}

/// Nach dem Anlegen eines Projekts aus dem Template: Meilensteine, Issues und Dokumente.
#[derive(Debug, Clone)]
pub(crate) struct ProjectApply {
    pub template: Template,
    pub data: ProjectData,
    /// Basisdatum für relative Ziel-/Meilensteindaten.
    pub base_date: NaiveDate,
    pub warnings: Vec<String>,
}

fn opt_str(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Führt Input und Projekt-Template zusammen (Mengenfelder: Vereinigung, Skalare: Input gewinnt).
pub(crate) async fn merge_project(
    app: &App,
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    template_ref: &str,
    mut input: SaveProjectInput,
) -> AppResult<(SaveProjectInput, ProjectApply)> {
    let tpl = load_template(tx, snap, template_ref, Some(TemplateType::Project)).await?;
    let data: ProjectData = parse(&tpl.template_data)?;
    let now = app.clock.now();
    let mut warnings = Vec::new();

    // Kontextteams (Eingabe) für die Verfügbarkeit des Templates.
    let mut ctx_teams: Vec<Team> = Vec::new();
    let refs: Vec<String> = input
        .set_teams
        .clone()
        .unwrap_or_default()
        .into_iter()
        .chain(input.add_teams.iter().cloned())
        .chain(input.lead_team.iter().cloned())
        .collect();
    for r in &refs {
        let t = resolve::team::resolve_in(tx, snap, r, Visibility::Content).await?;
        if !ctx_teams.iter().any(|x| x.id == t.id) {
            ctx_teams.push(t);
        }
    }
    let ids: Vec<TeamId> = ctx_teams.iter().map(|t| t.id).collect();
    if !ids.is_empty() && !available_in(&snap.tree, &tpl, &ids) {
        return Err(AppError::not_found("Could not find referenced Template."));
    }
    if let Some(t) = tpl.team_id {
        // Ein Team-Template enthält sein eigenes Team implizit.
        let tid = t.to_string();
        if input.set_teams.is_some() {
            if let Some(s) = input.set_teams.as_mut() {
                s.push(tid);
            }
        } else {
            input.add_teams.push(tid);
        }
    }
    for t in data.team_ids.iter().flatten() {
        match input.set_teams.as_mut() {
            Some(s) => s.push(t.clone()),
            None => input.add_teams.push(t.clone()),
        }
    }

    if opt_str(&input.name).is_none() {
        input.name = data.name.clone();
    }
    if input.summary.is_none() {
        input.summary = data.summary.clone();
    }
    input.description = match opt_str(&input.description) {
        Some(d) => Some(d.to_owned()),
        None => data
            .description
            .as_deref()
            .map(strip_placeholders)
            .filter(|s| !s.trim().is_empty()),
    };
    if input.state.is_none() {
        input.state = data.status_id.clone().or_else(|| data.status_type.clone());
    }
    if input.priority.is_none() {
        input.priority = data.priority.and_then(|p| i32::try_from(p).ok());
    }
    if input.lead.is_none()
        && let Some(l) = &data.lead_id
    {
        if l == "creator" {
            input.lead = actor_user_id(ctx).map(|u| Some(u.to_string()));
        } else if let Ok(u) = Uuid::parse_str(l)
            && tx.users().get(UserId(u)).await?.is_some_and(|u| u.active)
        {
            input.lead = Some(Some(l.clone()));
        } else {
            warnings.push("leadId".into());
        }
    }
    // Labels: Vereinigung (nicht mehr vorhandene oder stillgelegte werden übersprungen).
    let mut labels: Vec<String> = Vec::new();
    for l in data.label_ids.iter().flatten() {
        let ok = match Uuid::parse_str(l) {
            Ok(u) => tx
                .projects()
                .get_label(u)
                .await?
                .is_some_and(|x| !x.retired && !x.is_group),
            Err(_) => false,
        };
        if ok {
            labels.push(l.clone());
        } else {
            warnings.push("labelIds".into());
        }
    }
    if input.labels.is_some() || !labels.is_empty() {
        for l in input.labels.take().unwrap_or_default() {
            if !labels.contains(&l) {
                labels.push(l);
            }
        }
        input.labels = Some(labels);
    }
    for i in data.initiative_ids.iter().flatten() {
        match input.set_initiatives.as_mut() {
            Some(s) => s.push(i.clone()),
            None => input.add_initiatives.push(i.clone()),
        }
    }
    if input.icon.is_none() {
        input.icon = data.icon.clone();
    }
    if input.color.is_none() {
        input.color = data.color.clone();
    }
    // Daten: relative Angaben ab heute (Zeitzone des ersten Teams).
    let today = ctx_teams
        .first()
        .map_or_else(|| now.date_naive(), |t| local_today(t, now));
    if input.start_date.is_none()
        && let Some(s) = &data.start_in
    {
        input.start_date = Some(Some(add_days(today, parse_relative_days("startIn", s)?)?));
    }
    let base = match input.start_date {
        Some(Some(d)) => d,
        _ => today,
    };
    if input.target_date.is_none()
        && let Some(t) = &data.target_in
    {
        input.target_date = Some(Some(add_days(base, parse_relative_days("targetIn", t)?)?));
    }
    if input.target_date_resolution.is_none()
        && let Some(r) = &data.target_date_resolution
    {
        input.target_date_resolution = r.parse::<DateResolution>().ok().map(Some);
    }
    input.template = None;
    Ok((
        input,
        ProjectApply {
            template: tpl,
            data,
            base_date: base,
            warnings,
        },
    ))
}

/// Legt Meilensteine, Issues und Dokumente des Projekt-Templates an (gleiche Transaktion).
pub(crate) async fn apply_project_children(
    app: &App,
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    project: ProjectId,
    ap: ProjectApply,
) -> AppResult<Vec<String>> {
    let now = app.clock.now();
    let mut warnings = ap.warnings.clone();
    tx.templates()
        .mark_object(TemplateType::Project, project.0, ap.template.id)
        .await?;
    tx.templates().mark_applied(ap.template.id, now).await?;
    let project_teams = tx.projects().list_teams(project).await?;
    let lead_team = tx
        .projects()
        .get(project)
        .await?
        .map(|p| p.lead_team_id)
        .ok_or_else(|| AppError::internal("project missing after creation"))?;

    // Mitglieder: nicht mehr vorhandene User werden übersprungen.
    for (i, m) in ap.data.member_ids.iter().flatten().enumerate() {
        let user = match Uuid::parse_str(m) {
            Ok(u) => tx.users().get(UserId(u)).await?.filter(|u| u.active),
            Err(_) => None,
        };
        match user {
            Some(u) => {
                let memberships = tx.teams().list_memberships_of_user(u.id).await?;
                let view = crate::policy::ActorView::new(
                    crate::context::Actor::User {
                        id: u.id,
                        role: u.role,
                    },
                    &memberships,
                );
                let visible = crate::policy::compute_visible_team_ids(&view, &snap.tree);
                if project_teams.iter().any(|t| visible.contains(t)) {
                    tx.projects().add_member(project, u.id, now).await?;
                } else {
                    warnings.push(format!("memberIds[{i}]"));
                }
            }
            None => warnings.push(format!("memberIds[{i}]")),
        }
    }

    // Meilensteine.
    let mut milestone_ids: Vec<(String, MilestoneId)> = Vec::new();
    for (i, m) in ap.data.milestones.iter().flatten().enumerate() {
        let MilestoneData {
            name,
            description,
            target_in,
        } = m;
        let target = match target_in {
            Some(t) => Some(add_days(
                ap.base_date,
                parse_relative_days(&format!("milestones[{i}].targetIn"), t)?,
            )?),
            None => None,
        };
        let id = MilestoneId(app.ids.new_uuid());
        tx.projects()
            .insert_milestone(&NewMilestone {
                id,
                project_id: project,
                name: name.trim().to_owned(),
                description: description.clone(),
                target_date: target,
                sort_order: (i + 1) as f64,
                now,
            })
            .await
            .map_err(|e| with_path(e, &format!("milestones[{i}]")))?;
        milestone_ids.push((name.to_lowercase(), id));
    }

    // Issues: pro Team in aufsteigender Team-ID (stabil) anlegen, um Sperrenreihenfolgen
    // über Teams hinweg zu vereinheitlichen (docs/06 §2.6).
    let entries: Vec<_> = ap.data.issues.iter().flatten().enumerate().collect();
    let mut keyed: Vec<(Uuid, usize, &super::data::ProjectIssueData)> = Vec::new();
    for (i, e) in entries {
        let team_id = match &e.team_id {
            Some(t) => Uuid::parse_str(t).map_err(|_| {
                with_path(
                    invalid("teamId", "teamId must be a UUID"),
                    &format!("issues[{i}]"),
                )
            })?,
            None => lead_team.0,
        };
        keyed.push((team_id, i, e));
    }
    keyed.sort_by_key(|(t, i, _)| (*t, *i));
    for (team_id, i, e) in keyed {
        let path = format!("issues[{i}]");
        let team_id = TeamId(team_id);
        if !project_teams.contains(&team_id) {
            return Err(with_path(
                invalid("teamId", "the team is not part of the project"),
                &path,
            ));
        }
        let team = snap.tree.get(team_id).cloned().ok_or_else(|| {
            with_path(
                AppError::not_found("Could not find referenced Team."),
                &path,
            )
        })?;
        snap.require_team(ctx, Action::CreateIssue, team_id)
            .map_err(|e| with_path(e, &path))?;
        let (data, template) = match &e.template_id {
            Some(tid) => {
                let tpl = load_template(tx, snap, tid, Some(TemplateType::Issue))
                    .await
                    .map_err(|e| with_path(e, &path))?;
                if !available_in(&snap.tree, &tpl, &[team_id]) {
                    return Err(with_path(
                        AppError::not_found("Could not find referenced Template."),
                        &path,
                    ));
                }
                let d: IssueData = parse(&tpl.template_data).map_err(|e| with_path(e, &path))?;
                (d, Some(tpl))
            }
            None => (e.issue.clone().unwrap_or_default(), None),
        };
        let milestone = match &e.milestone {
            Some(m) => Some(
                milestone_ids
                    .iter()
                    .find(|(n, _)| *n == m.to_lowercase())
                    .map(|(_, id)| id.to_string())
                    .ok_or_else(|| with_path(invalid("milestone", "unknown milestone"), &path))?,
            ),
            None => None,
        };
        let input = CreateIssueInput {
            team: team_id.to_string(),
            project: Some(project.to_string()),
            milestone,
            source: HistorySource::default(),
            ..Default::default()
        };
        let outcome = create_issue_tree(
            app,
            tx,
            snap,
            ctx,
            &team,
            template.as_ref(),
            &data,
            input,
            &BTreeSet::new(),
            true,
        )
        .await
        .map_err(|e| with_path(e, &path))?;
        warnings.extend(outcome.warnings);
    }

    // Dokumente.
    for (i, e) in ap.data.documents.iter().flatten().enumerate() {
        let path = format!("documents[{i}]");
        let doc = e.document.clone().unwrap_or_default();
        let input = SaveDocumentInput {
            title: match &e.template_id {
                Some(_) => doc.title.clone(),
                None => Some(doc.title.clone().unwrap_or_default()),
            },
            content: doc.content.clone(),
            icon: doc.icon.clone(),
            color: doc.color.clone(),
            parent: ParentRefs {
                project: Some(project.to_string()),
                ..Default::default()
            },
            template: e.template_id.clone(),
            ..Default::default()
        };
        let view = app
            .save_document_in(tx, ctx, input)
            .await
            .map_err(|e| with_path(e, &path))?;
        warnings.extend(view.warnings);
    }
    Ok(warnings)
}

/// Eingabe für [`App::create_issue_from_template`] aus JSON-Feldnamen (`null`-Werte im Input).
pub fn null_field_set(fields: &[&str]) -> BTreeSet<String> {
    fields.iter().map(|s| (*s).to_owned()).collect()
}

/// Gemeinsamer Teil: Team auflösen, Rechte prüfen, Template laden.
pub(crate) async fn prepare_issue_template(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    template_ref: &str,
    input: &CreateIssueInput,
) -> AppResult<(Template, Team, IssueData)> {
    let team = resolve::team::resolve_in(tx, snap, &input.team, Visibility::Content).await?;
    snap.require_team(ctx, Action::CreateIssue, team.id)?;
    let tpl = load_template(tx, snap, template_ref, Some(TemplateType::Issue)).await?;
    if !available_in(&snap.tree, &tpl, &[team.id]) {
        return Err(AppError::not_found("Could not find referenced Template.")
            .with_details(json!({ "reference": template_ref, "type": "Template" })));
    }
    let data: IssueData = parse(&tpl.template_data)?;
    if input.parent.is_some() && data.sub_issues.as_ref().is_some_and(|s| !s.is_empty()) {
        return Err(AppError::invalid_request(
            "a template with sub-issues cannot be applied to a sub-issue",
        )
        .with_details(json!({ "field": "parentId", "reason": "template_has_sub_issues" })));
    }
    Ok((tpl, team, data))
}
