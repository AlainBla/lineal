//! Tool-Handler der Gruppe `initiatives` (WP07): `list_initiatives`, `get_initiative`,
//! `save_initiative` (docs/07 §3.10). Status wird als `StatusRef` ausgegeben.

use super::projects::{
    arg_bool, arg_list, arg_str, parse_patch, priority_json, required, url_of, user_ref,
};
use super::teams_users::{envelope, page_request, parse_time_filter, team_ref, ts};
use crate::fields::{parse_fields, select_fields, truncate_description};
use lineal_app::usecases::initiative::{
    InitiativeView, ListInitiativesInput, SaveInitiativeInput, initiative_status_ref,
};
use lineal_app::usecases::project::pure::parse_date;
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Map, Value, json};

pub(crate) fn initiative_ref(i: &lineal_app::ports::repos::initiative::Initiative) -> Value {
    json!({ "id": i.id.to_string(), "identifier": i.identifier(), "name": i.name })
}

fn initiative_json(ws: &str, v: &InitiativeView, projects: bool, subs: bool) -> Map<String, Value> {
    let i = &v.initiative;
    let (sid, sname, stype) = initiative_status_ref(i.status);
    let mut m = Map::new();
    m.insert("id".into(), json!(v.identifier));
    m.insert("uuid".into(), json!(i.id.to_string()));
    m.insert("name".into(), json!(i.name));
    m.insert("summary".into(), json!(i.description));
    m.insert("description".into(), json!(i.content));
    m.insert(
        "url".into(),
        json!(url_of(ws, "initiative", &i.name, &i.slug_id)),
    );
    m.insert("color".into(), json!(i.color));
    m.insert("icon".into(), json!(i.icon));
    m.insert(
        "status".into(),
        json!({ "id": sid, "name": sname, "type": stype }),
    );
    m.insert("priority".into(), priority_json(i.priority));
    m.insert(
        "targetDate".into(),
        json!(i.target_date.map(|d| d.to_string())),
    );
    m.insert(
        "targetDateResolution".into(),
        json!(i.target_date_resolution.map(|r| r.as_str())),
    );
    m.insert("health".into(), json!(i.health.map(|h| h.as_str())));
    m.insert("createdAt".into(), json!(ts(i.created_at)));
    m.insert("updatedAt".into(), json!(ts(i.updated_at)));
    m.insert(
        "owner".into(),
        v.owner.as_ref().map_or(Value::Null, user_ref),
    );
    m.insert(
        "creator".into(),
        v.creator.as_ref().map_or(Value::Null, user_ref),
    );
    m.insert(
        "leadTeam".into(),
        v.lead_team.as_ref().map_or(Value::Null, team_ref),
    );
    m.insert(
        "parentInitiatives".into(),
        Value::Array(v.parents.iter().map(initiative_ref).collect()),
    );
    m.insert(
        "labels".into(),
        Value::Array(
            v.labels
                .iter()
                .map(|l| json!({ "id": l.id.to_string(), "name": l.name }))
                .collect(),
        ),
    );
    if projects {
        m.insert(
            "projects".into(),
            Value::Array(
                v.projects
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.project.id.to_string(), "identifier": p.identifier, "name": p.project.name,
                            "status": { "id": p.status.id.to_string(), "name": p.status.name, "type": p.status.status_type.as_str() },
                        })
                    })
                    .collect(),
            ),
        );
    }
    if subs {
        m.insert(
            "subInitiatives".into(),
            Value::Array(v.sub_initiatives.iter().map(initiative_ref).collect()),
        );
    }
    m
}

fn save_input(args: &Value) -> AppResult<SaveInitiativeInput> {
    let nullable = |key: &str| match args.get(key) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => v.as_str().map(|s| Some(s.to_owned())),
    };
    Ok(SaveInitiativeInput {
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
        status: arg_str(args, "status").map(str::to_owned),
        owner: nullable("owner"),
        lead_team: nullable("leadTeam"),
        parent_initiatives: arg_list(args, "parentInitiatives"),
        labels: args.get("labels").map(|_| arg_list(args, "labels")),
        target_date: arg_str(args, "targetDate")
            .map(|s| parse_date("targetDate", s).map(Some))
            .transpose()?,
        target_date_resolution: None,
    })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    let ws = |org: &lineal_app::ports::repos::org::Org| app.workspace_url(org);
    match tool {
        "list_initiatives" => {
            if arg_str(&args, "customView").is_some() {
                return Err(AppError::invalid_request(
                    "`customView` filters are not available yet.",
                )
                .with_details(
                    json!({ "path": "/customView", "reason": "custom_view_unsupported" }),
                ));
            }
            let now = app.clock.now();
            let time = |key: &str| {
                arg_str(&args, key)
                    .map(|s| parse_time_filter(key, s, now))
                    .transpose()
            };
            let fields = parse_fields(&args);
            let projects =
                arg_bool(&args, "includeProjects") || fields.iter().any(|f| f == "projects");
            let subs = arg_bool(&args, "includeSubInitiatives")
                || fields.iter().any(|f| f == "subInitiatives");
            let input = ListInitiativesInput {
                status: arg_str(&args, "status").map(str::to_owned),
                owner: arg_str(&args, "owner").map(str::to_owned),
                lead_team: arg_str(&args, "leadTeam").map(str::to_owned),
                parent_initiative: arg_str(&args, "parentInitiative").map(str::to_owned),
                label: arg_str(&args, "label").map(str::to_owned),
                query: arg_str(&args, "query").map(str::to_owned),
                include_archived: arg_bool(&args, "includeArchived"),
                created_after: time("createdAt")?,
                updated_after: time("updatedAt")?,
                page: page_request(tool, &args)?,
            };
            let (page, views) = app.list_initiatives(ctx, input).await?;
            let org = app.get_workspace(ctx).await?;
            let url = ws(&org);
            let items = views
                .iter()
                .map(|v| {
                    let mut full = initiative_json(&url, v, projects, subs);
                    if let Some(Value::String(d)) = full.get("description").cloned() {
                        full.insert(
                            "description".into(),
                            json!(truncate_description(&d, "get_initiative")),
                        );
                    }
                    select_fields(
                        &full,
                        &fields,
                        &["parentInitiatives", "labels", "projects", "subInitiatives"],
                    )
                })
                .collect();
            Ok(envelope("initiatives", tool, &args, &page, items))
        }
        "get_initiative" => {
            let v = app.get_initiative(ctx, required(&args, "query")?).await?;
            let org = app.get_workspace(ctx).await?;
            Ok(Value::Object(initiative_json(
                &ws(&org),
                &v,
                arg_bool(&args, "includeProjects"),
                arg_bool(&args, "includeSubInitiatives"),
            )))
        }
        "save_initiative" => {
            let v = app.save_initiative(ctx, save_input(&args)?).await?;
            let org = app.get_workspace(ctx).await?;
            Ok(Value::Object(initiative_json(&ws(&org), &v, false, false)))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by initiatives"
        ))),
    }
}
