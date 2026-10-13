//! Tool-Handler der Gruppe `projects` (WP07): `list_projects`, `get_project`, `save_project`
//! (docs/07 §3.7). Enthält die gemeinsamen JSON-Bausteine der Planungsgruppen (Projekt-/Initiative-Refs,
//! URLs, `progress` als 0..100).

use super::teams_users::{envelope, page_request, parse_time_filter, team_ref, ts};
use crate::fields::{parse_fields, select_fields, truncate_description};
use lineal_app::ports::repos::project::{ExternalLink, ProjectLabelRow};
use lineal_app::ports::repos::user::User;
use lineal_app::usecases::project::milestone::ProjectRef;
use lineal_app::usecases::project::pure::parse_date;
use lineal_app::usecases::project::{
    ListProjectsInput, MilestoneView, ProjectView, SaveProjectInput, ViewOpts,
};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use lineal_domain::enums::{DateResolution, Priority};
use lineal_domain::rules::patch::PatchOp;
use serde_json::{Map, Value, json};

/// `progress` im Core 0..1, in MCP 0..100 mit einer Nachkommastelle.
pub(crate) fn pct(p: f64) -> f64 {
    (p * 1000.0).round() / 10.0
}

pub(crate) fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

pub(crate) fn url_of(ws_url: &str, kind: &str, name: &str, slug_id: &str) -> String {
    let s = slugify(name);
    if s.is_empty() {
        format!("{ws_url}/{kind}/{slug_id}")
    } else {
        format!("{ws_url}/{kind}/{s}-{slug_id}")
    }
}

pub(crate) fn user_ref(u: &User) -> Value {
    json!({ "id": u.id.to_string(), "name": if u.name.is_empty() { u.email.clone() } else { u.name.clone() } })
}

pub(crate) fn priority_json(p: i32) -> Value {
    let v = u8::try_from(p)
        .ok()
        .and_then(|v| Priority::try_from(v).ok())
        .unwrap_or(Priority::NoPriority);
    json!({ "value": v.value(), "name": v.name() })
}

pub(crate) fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub(crate) fn arg_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(crate) fn arg_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn required<'a>(args: &'a Value, key: &str) -> AppResult<&'a str> {
    arg_str(args, key)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            AppError::invalid_request(format!("`{key}` is required"))
                .with_details(json!({ "path": format!("/{key}") }))
        })
}

pub(crate) fn parse_resolution(field: &str, s: &str) -> AppResult<DateResolution> {
    s.parse::<DateResolution>().map_err(|_| {
        AppError::invalid_request(format!("`{field}` is not a valid resolution"))
            .with_details(json!({ "field": field }))
    })
}

pub(crate) fn parse_patch(args: &Value) -> AppResult<Option<Vec<PatchOp>>> {
    match args.get("patch") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| {
            AppError::invalid_request(format!("invalid `patch`: {e}"))
                .with_details(json!({ "path": "/patch" }))
        }),
    }
}

fn label_ref(l: &ProjectLabelRow) -> Value {
    json!({ "id": l.id.to_string(), "name": l.name })
}

fn link_json(l: &ExternalLink) -> Value {
    json!({ "id": l.id.to_string(), "title": l.label, "url": l.url })
}

/// `MilestoneRef`-nahes Objekt (`list_milestones`-Form) eines Meilensteins.
pub(crate) fn milestone_json(p: &ProjectRef, m: &MilestoneView) -> Value {
    json!({
        "id": m.milestone.id.to_string(), "name": m.milestone.name, "description": m.milestone.description,
        "targetDate": m.milestone.target_date.map(|d| d.to_string()), "progress": pct(m.progress),
        "sortOrder": m.milestone.sort_order, "status": m.status.as_str(),
        "project": json!({ "id": p.id.to_string(), "identifier": p.identifier, "name": p.name }),
        "createdAt": ts(m.milestone.created_at), "updatedAt": ts(m.milestone.updated_at),
    })
}

/// Vollständiges Projektobjekt; `fields` und Kürzung wenden die Aufrufer an.
pub(crate) fn project_json(
    ws_url: &str,
    v: &ProjectView,
    with_members: bool,
    with_milestones: bool,
) -> Map<String, Value> {
    let p = &v.project;
    let pref = ProjectRef {
        id: p.id,
        identifier: v.identifier.clone(),
        name: p.name.clone(),
    };
    let mut m = Map::new();
    m.insert("id".into(), json!(v.identifier));
    m.insert("uuid".into(), json!(p.id.to_string()));
    m.insert("name".into(), json!(p.name));
    m.insert("summary".into(), json!(p.description));
    m.insert("description".into(), json!(p.content));
    m.insert("icon".into(), json!(p.icon));
    m.insert("color".into(), json!(p.color));
    m.insert(
        "url".into(),
        json!(url_of(ws_url, "project", &p.name, &p.slug_id)),
    );
    m.insert("trashed".into(), json!(p.trashed));
    m.insert("createdAt".into(), json!(ts(p.created_at)));
    m.insert("updatedAt".into(), json!(ts(p.updated_at)));
    m.insert("startedAt".into(), json!(p.started_at.map(ts)));
    m.insert("completedAt".into(), json!(p.completed_at.map(ts)));
    m.insert("canceledAt".into(), json!(p.canceled_at.map(ts)));
    m.insert(
        "startDate".into(),
        json!(p.start_date.map(|d| d.to_string())),
    );
    m.insert(
        "startDateResolution".into(),
        json!(p.start_date_resolution.map(|r| r.as_str())),
    );
    m.insert(
        "targetDate".into(),
        json!(p.target_date.map(|d| d.to_string())),
    );
    m.insert(
        "targetDateResolution".into(),
        json!(p.target_date_resolution.map(|r| r.as_str())),
    );
    m.insert("priority".into(), priority_json(p.priority));
    m.insert("sortOrder".into(), json!(p.sort_order));
    m.insert(
        "status".into(),
        json!({ "id": v.status.id.to_string(), "name": v.status.name, "type": v.status.status_type.as_str() }),
    );
    m.insert("health".into(), json!(p.health.map(|h| h.as_str())));
    m.insert("progress".into(), json!(pct(v.scope.progress)));
    m.insert(
        "labels".into(),
        Value::Array(v.labels.iter().map(label_ref).collect()),
    );
    m.insert(
        "initiatives".into(),
        Value::Array(
            v.initiatives
                .iter()
                .map(|i| json!({ "id": i.id.to_string(), "identifier": i.identifier(), "name": i.name }))
                .collect(),
        ),
    );
    m.insert("lead".into(), v.lead.as_ref().map_or(Value::Null, user_ref));
    m.insert(
        "leadTeam".into(),
        v.lead_team.as_ref().map_or(Value::Null, team_ref),
    );
    m.insert(
        "teams".into(),
        Value::Array(v.teams.iter().map(team_ref).collect()),
    );
    if with_members {
        m.insert(
            "members".into(),
            Value::Array(
                v.members
                    .iter()
                    .map(|u| {
                        let mut j = user_ref(u);
                        j["email"] = json!(u.email);
                        j
                    })
                    .collect(),
            ),
        );
    }
    if with_milestones {
        m.insert(
            "milestones".into(),
            Value::Array(
                v.milestones
                    .iter()
                    .map(|x| milestone_json(&pref, x))
                    .collect(),
            ),
        );
    }
    m
}

async fn ws_url(app: &App, ctx: &RequestCtx) -> AppResult<String> {
    let org = app.get_workspace(ctx).await?;
    Ok(app.workspace_url(&org))
}

fn save_input(args: &Value) -> AppResult<SaveProjectInput> {
    let date = |key: &str| -> AppResult<Option<Option<chrono::NaiveDate>>> {
        arg_str(args, key)
            .map(|s| parse_date(key, s).map(Some))
            .transpose()
    };
    let res = |key: &str| -> AppResult<Option<Option<DateResolution>>> {
        arg_str(args, key)
            .map(|s| parse_resolution(key, s).map(Some))
            .transpose()
    };
    let links = args
        .get("links")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    Some((
                        l.get("url")?.as_str()?.to_owned(),
                        l.get("title")?.as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(SaveProjectInput {
        id: arg_str(args, "id").map(str::to_owned),
        name: arg_str(args, "name").map(str::to_owned),
        summary: arg_str(args, "summary").map(str::to_owned),
        description: arg_str(args, "description").map(str::to_owned),
        patch: parse_patch(args)?,
        icon: arg_str(args, "icon").map(str::to_owned),
        color: arg_str(args, "color").map(str::to_owned),
        priority: args
            .get("priority")
            .and_then(Value::as_i64)
            .map(|p| p as i32),
        state: arg_str(args, "state").map(str::to_owned),
        lead: match args.get("lead") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(v) => v.as_str().map(|s| Some(s.to_owned())),
        },
        lead_team: arg_str(args, "leadTeam").map(str::to_owned),
        set_teams: args.get("setTeams").map(|_| arg_list(args, "setTeams")),
        add_teams: arg_list(args, "addTeams"),
        remove_teams: arg_list(args, "removeTeams"),
        add_members: Vec::new(),
        remove_members: Vec::new(),
        labels: args.get("labels").map(|_| arg_list(args, "labels")),
        set_initiatives: args
            .get("setInitiatives")
            .map(|_| arg_list(args, "setInitiatives")),
        add_initiatives: arg_list(args, "addInitiatives"),
        remove_initiatives: arg_list(args, "removeInitiatives"),
        links,
        start_date: date("startDate")?,
        start_date_resolution: res("startDateResolution")?,
        target_date: date("targetDate")?,
        target_date_resolution: res("targetDateResolution")?,
        template: arg_str(args, "template").map(str::to_owned),
        // MCP: ein Team mit Issues im Projekt zu entfernen ist ein Fehler (docs/07 §3.7).
        detach_issues_of_removed_teams: false,
    })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_projects" => {
            let now = app.clock.now();
            let time = |key: &str| {
                arg_str(&args, key)
                    .map(|s| parse_time_filter(key, s, now))
                    .transpose()
            };
            let fields = parse_fields(&args);
            let include_members =
                arg_bool(&args, "includeMembers") || fields.iter().any(|f| f == "members");
            let include_milestones =
                arg_bool(&args, "includeMilestones") || fields.iter().any(|f| f == "milestones");
            let input = ListProjectsInput {
                custom_view: arg_str(&args, "customView").map(str::to_owned),
                team: arg_str(&args, "team").map(str::to_owned),
                include_sub_teams: arg_bool(&args, "includeSubTeams"),
                initiative: arg_str(&args, "initiative").map(str::to_owned),
                member: arg_str(&args, "member").map(str::to_owned),
                state: arg_str(&args, "state").map(str::to_owned),
                label: arg_str(&args, "label").map(str::to_owned),
                query: arg_str(&args, "query").map(str::to_owned),
                include_archived: arg_bool(&args, "includeArchived"),
                trashed: false,
                created_after: time("createdAt")?,
                updated_after: time("updatedAt")?,
                include_members,
                include_milestones,
                page: page_request(tool, &args)?,
            };
            let (page, views) = app.list_projects(ctx, input).await?;
            let ws = ws_url(app, ctx).await?;
            let items = views
                .iter()
                .map(|v| {
                    let mut full = project_json(&ws, v, include_members, include_milestones);
                    if let Some(Value::String(d)) = full.get("description").cloned() {
                        full.insert(
                            "description".into(),
                            json!(truncate_description(&d, "get_project")),
                        );
                    }
                    select_fields(
                        &full,
                        &fields,
                        &["labels", "initiatives", "teams", "members", "milestones"],
                    )
                })
                .collect();
            Ok(envelope("projects", tool, &args, &page, items))
        }
        "get_project" => {
            let query = required(&args, "query")?;
            let opts = ViewOpts {
                members: arg_bool(&args, "includeMembers"),
                milestones: arg_bool(&args, "includeMilestones"),
            };
            let v = app.get_project(ctx, query, opts).await?;
            let ws = ws_url(app, ctx).await?;
            let mut j = project_json(&ws, &v, opts.members, opts.milestones);
            if arg_bool(&args, "includeResources") {
                let r = app.get_project_resources(ctx, query).await?;
                j.insert(
                    "resources".into(),
                    json!({
                        "documents": r.documents.iter().map(|(id, title, slug)| json!({
                            "id": id.to_string(), "title": title, "url": url_of(&ws, "document", title, slug),
                        })).collect::<Vec<_>>(),
                        "links": r.links.iter().map(link_json).collect::<Vec<_>>(),
                        "attachments": [],
                    }),
                );
                j.insert("resourceCount".into(), json!(r.resource_count));
            }
            Ok(Value::Object(j))
        }
        "save_project" => {
            let v = app.save_project(ctx, save_input(&args)?).await?;
            let ws = ws_url(app, ctx).await?;
            let mut j = project_json(&ws, &v, false, false);
            if let Some(n) = v.open_issue_count {
                j.insert("openIssueCount".into(), json!(n));
            }
            Ok(Value::Object(j))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by projects"
        ))),
    }
}

#[cfg(test)]
#[path = "planning_tests.rs"]
mod planning_tests;
