//! Tool-Handler der Gruppe `issues` (WP06): `list_issues`, `get_issue`, `save_issue` (docs/07 §3.2).
//! Issues werden flach ausgegeben (Namen plus `*Id`-Felder), alle Default-Schlüssel sind immer da.

use super::teams_users::{parse_time_filter, ts as ts_millis};
use crate::envelope::{
    decode_cursor, encode_cursor, page, parse_limit, priority, query_fingerprint,
};
use crate::fields::{parse_fields, select_fields, truncate_description};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use lineal_app::ports::repos::issue::HistorySource;
use lineal_app::ports::repos::user::{OrderBy, PageCursor};
use lineal_app::usecases::attachment::LinkAttachmentInput;
use lineal_app::usecases::issue::{
    CreateIssueInput, DescriptionEdit, IssueDetail, IssueView, ListIssuesInput, RelationChanges,
    RelationsView, UpdateIssueInput,
};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use lineal_domain::enums::SlaType;
use lineal_domain::rules::patch::PatchOp;
use serde_json::{Map, Value, json};

/// Schlüssel der Default-Antwort in Listen und bei `get_issue`.
const DEFAULT_KEYS: [&str; 29] = [
    "id",
    "uuid",
    "title",
    "description",
    "priority",
    "url",
    "gitBranchName",
    "createdAt",
    "updatedAt",
    "archivedAt",
    "completedAt",
    "startedAt",
    "canceledAt",
    "dueDate",
    "slaStartedAt",
    "slaMediumRiskAt",
    "slaHighRiskAt",
    "slaBreachesAt",
    "status",
    "statusType",
    "labels",
    "createdBy",
    "createdById",
    "assignee",
    "assigneeId",
    "project",
    "projectId",
    "parentId",
    "team",
];
const DEFAULT_KEYS_TAIL: [&str; 2] = ["teamId", "cycleId"];
/// Listenfelder (leer = `[]` statt `null`).
const LIST_FIELDS: [&str; 5] = [
    "labels",
    "attachments",
    "documents",
    "stateHistory",
    "releases",
];

fn bad(path: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "path": format!("/{path}") }))
}

fn ts(t: Option<DateTime<Utc>>) -> Value {
    t.map_or(Value::Null, |t| Value::String(ts_millis(t)))
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 50 {
            break;
        }
    }
    out.trim_matches('-').to_owned()
}

/// Vollständige Feldmenge eines Issues; der Aufrufer wählt aus.
fn issue_map(
    v: &IssueView,
    base_url: &str,
    handle: &str,
    truncate: bool,
) -> AppResult<Map<String, Value>> {
    let i = &v.issue;
    let e = &v.extras;
    let slug = slug(&i.title);
    let uuid = |u: Option<uuid::Uuid>| u.map_or(Value::Null, |u| Value::String(u.to_string()));
    let description = i.description.as_deref().map(|d| {
        if truncate {
            truncate_description(d, "get_issue")
        } else {
            d.to_owned()
        }
    });
    let mut m = Map::new();
    m.insert("id".into(), json!(i.identifier));
    m.insert("uuid".into(), json!(i.id.to_string()));
    m.insert("title".into(), json!(i.title));
    m.insert("description".into(), json!(description));
    m.insert("priority".into(), priority(i.priority.value())?);
    m.insert(
        "estimate".into(),
        i.estimate.map_or(Value::Null, |x| json!(x)),
    );
    m.insert(
        "url".into(),
        json!(format!("{base_url}/issue/{}/{slug}", i.identifier)),
    );
    m.insert(
        "gitBranchName".into(),
        json!(format!("{handle}/{}-{slug}", i.identifier.to_lowercase())),
    );
    m.insert("createdAt".into(), ts(Some(i.created_at)));
    m.insert("updatedAt".into(), ts(Some(i.updated_at)));
    m.insert("archivedAt".into(), ts(i.archived_at));
    m.insert("completedAt".into(), ts(i.completed_at));
    m.insert("startedAt".into(), ts(i.started_at));
    m.insert("canceledAt".into(), ts(i.canceled_at));
    m.insert("startedTriageAt".into(), ts(i.started_triage_at));
    m.insert("triagedAt".into(), ts(i.triaged_at));
    m.insert(
        "dueDate".into(),
        i.due_date.map_or(Value::Null, |d| json!(d.to_string())),
    );
    m.insert("slaStartedAt".into(), ts(i.sla_started_at));
    m.insert("slaMediumRiskAt".into(), ts(i.sla_medium_risk_at));
    m.insert("slaHighRiskAt".into(), ts(i.sla_high_risk_at));
    m.insert("slaBreachesAt".into(), ts(i.sla_breaches_at));
    m.insert(
        "slaType".into(),
        i.sla_type.map_or(Value::Null, |t| json!(t.as_str())),
    );
    m.insert("status".into(), json!(e.state_name));
    m.insert("statusType".into(), json!(i.state_type.as_str()));
    m.insert("labels".into(), json!(e.label_names));
    m.insert("triageIntel".into(), Value::Null);
    m.insert("createdBy".into(), json!(e.creator_name));
    m.insert("createdById".into(), uuid(i.creator_id.map(|u| u.0)));
    m.insert("assignee".into(), json!(e.assignee_name));
    m.insert("assigneeId".into(), uuid(i.assignee_id.map(|u| u.0)));
    m.insert("delegate".into(), json!(e.delegate_name));
    m.insert("delegateId".into(), uuid(i.delegate_id.map(|u| u.0)));
    m.insert("project".into(), json!(e.project_name));
    m.insert("projectId".into(), uuid(i.project_id.map(|u| u.0)));
    m.insert(
        "projectMilestone".into(),
        match (&e.milestone_name, i.project_milestone_id) {
            (Some(n), Some(id)) => json!({ "id": id.to_string(), "name": n }),
            _ => Value::Null,
        },
    );
    m.insert("parentId".into(), json!(e.parent_identifier));
    m.insert("team".into(), json!(e.team_name));
    m.insert("teamId".into(), json!(i.team_id.to_string()));
    m.insert("cycleId".into(), uuid(i.cycle_id.map(|u| u.0)));
    Ok(m)
}

fn related_json(r: &lineal_app::usecases::issue::RelatedIssue) -> Value {
    json!({ "id": r.identifier, "title": r.title })
}

fn relations_json(r: &RelationsView) -> Value {
    json!({
        "blocks": r.blocks.iter().map(related_json).collect::<Vec<_>>(),
        "blockedBy": r.blocked_by.iter().map(related_json).collect::<Vec<_>>(),
        "relatedTo": r.related_to.iter().map(related_json).collect::<Vec<_>>(),
        "duplicateOf": r.duplicate_of.as_ref().map(related_json),
        "duplicates": r.duplicates.iter().map(related_json).collect::<Vec<_>>(),
    })
}

/// Antwort in der Form von `get_issue`.
fn detail_json(
    d: &IssueDetail,
    base_url: &str,
    handle: &str,
    args: &Value,
    include_relations: bool,
) -> AppResult<Value> {
    let mut full = issue_map(&d.view, base_url, handle, false)?;
    full.insert("attachments".into(), json!([]));
    full.insert("documents".into(), json!([]));
    full.insert(
        "stateHistory".into(),
        json!(
            d.state_history
                .iter()
                .map(|s| json!({
                    "state": { "id": s.state_id.to_string(), "name": s.name, "type": s.state_type.as_str() },
                    "startedAt": ts(Some(s.started_at)),
                    "endedAt": ts(s.ended_at),
                }))
                .collect::<Vec<_>>()
        ),
    );
    full.insert("relations".into(), relations_json(&d.relations));
    full.insert("customerNeeds".into(), json!([]));
    full.insert("releases".into(), json!([]));
    let requested = parse_fields(args);
    if requested.is_empty() {
        let mut keys: Vec<&str> = DEFAULT_KEYS.to_vec();
        keys.extend(DEFAULT_KEYS_TAIL);
        keys.extend(["attachments", "documents", "stateHistory"]);
        if include_relations {
            keys.push("relations");
        }
        let mut out = Map::new();
        for k in keys {
            out.insert(k.into(), full.get(k).cloned().unwrap_or(Value::Null));
        }
        return Ok(Value::Object(out));
    }
    Ok(select_fields(&full, &requested, &LIST_FIELDS))
}

async fn caller_handle(app: &App, ctx: &RequestCtx) -> String {
    match app.get_user(ctx, "me").await {
        Ok(u) => u.user.display_name,
        Err(_) => "lineal".into(),
    }
}

async fn base_url(app: &App, ctx: &RequestCtx) -> AppResult<String> {
    let org = app.get_workspace(ctx).await?;
    Ok(app.workspace_url(&org))
}

fn arg_str<'a>(args: &'a Value, key: &str) -> AppResult<Option<&'a str>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(bad(key, format!("`{key}` must be a string"))),
    }
}

/// Tri-State für Referenzfelder: fehlt → `None`, `null` → `Some(None)`.
fn tri_str(args: &Value, key: &str) -> AppResult<Option<Option<String>>> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(bad(key, format!("`{key}` must be a string or null"))),
    }
}

fn str_list(args: &Value, key: &str) -> AppResult<Vec<String>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad(key, format!("`{key}` must contain strings")))
            })
            .collect(),
        Some(_) => Err(bad(key, format!("`{key}` must be an array"))),
    }
}

fn opt_list(args: &Value, key: &str) -> AppResult<Option<Vec<String>>> {
    if args.get(key).is_none_or(Value::is_null) {
        Ok(None)
    } else {
        str_list(args, key).map(Some)
    }
}

fn parse_date(key: &str, s: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .or_else(|_| DateTime::parse_from_rfc3339(s).map(|d| d.date_naive()))
        .map_err(|_| bad(key, format!("`{key}` must be an ISO date (YYYY-MM-DD)")))
}

fn relation_changes(args: &Value) -> AppResult<RelationChanges> {
    Ok(RelationChanges {
        blocks: str_list(args, "blocks")?,
        blocked_by: str_list(args, "blockedBy")?,
        related_to: str_list(args, "relatedTo")?,
        remove_blocks: str_list(args, "removeBlocks")?,
        remove_blocked_by: str_list(args, "removeBlockedBy")?,
        remove_related_to: str_list(args, "removeRelatedTo")?,
        duplicate_of: tri_str(args, "duplicateOf")?,
    })
}

fn priority_arg(args: &Value) -> AppResult<Option<i64>> {
    match args.get("priority") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|n| n.fract() == 0.0)
            .map(|n| Some(n as i64))
            .ok_or_else(|| bad("priority", "`priority` must be an integer between 0 and 4")),
    }
}

fn sla_type_arg(v: &Value) -> AppResult<Option<SlaType>> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => s
            .parse::<SlaType>()
            .map(Some)
            .map_err(|_| bad("slaType", "`slaType` must be `all` or `onlyBusinessDays`")),
        _ => Err(bad("slaType", "`slaType` must be a string or null")),
    }
}

fn sla_at_arg(v: &Value) -> AppResult<Option<DateTime<Utc>>> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .map(|d| Some(d.with_timezone(&Utc)))
            .map_err(|_| {
                bad(
                    "slaBreachesAt",
                    "`slaBreachesAt` must be an ISO-8601 timestamp",
                )
            }),
        _ => Err(bad(
            "slaBreachesAt",
            "`slaBreachesAt` must be a string or null",
        )),
    }
}

/// `links` (`[{url, title}]`): geprüft, bevor etwas angelegt wird (append-only, docs/07 §3.2).
fn links_arg(args: &Value) -> AppResult<Vec<(String, Option<String>)>> {
    let Some(raw) = args.get("links").filter(|v| !v.is_null()) else {
        return Ok(vec![]);
    };
    let items = raw
        .as_array()
        .ok_or_else(|| bad("links", "`links` must be an array"))?;
    let mut out = Vec::new();
    for (i, l) in items.iter().enumerate() {
        let url = l
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| bad(&format!("links/{i}/url"), "each link needs a `url`"))?;
        lineal_app::usecases::attachment::normalize_url(url)?;
        let title = l.get("title").and_then(Value::as_str).map(str::to_owned);
        out.push((url.to_owned(), title));
    }
    Ok(out)
}

/// Felder, die ausdrücklich als `null` übergeben wurden (überschreiben Template-Werte, docs/06 §2.5).
fn explicit_nulls(args: &Value) -> Vec<String> {
    [
        "state",
        "priority",
        "estimate",
        "assignee",
        "dueDate",
        "project",
        "milestone",
        "cycle",
    ]
    .into_iter()
    .filter(|k| matches!(args.get(*k), Some(Value::Null)))
    .map(str::to_owned)
    .collect()
}

fn create_input(args: &Value) -> AppResult<CreateIssueInput> {
    if args.get("patch").is_some() {
        return Err(bad("patch", "`patch` is only valid when updating an issue"));
    }
    if !str_list(args, "removeLabels")?.is_empty() {
        return Err(bad(
            "removeLabels",
            "`removeLabels` is only valid when updating",
        ));
    }
    let team = arg_str(args, "team")?
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| bad("team", "`team` is required when creating an issue"))?;
    let estimate = match args.get("estimate") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_f64()
                .ok_or_else(|| bad("estimate", "`estimate` must be a number"))?,
        ),
    };
    Ok(CreateIssueInput {
        team: team.to_owned(),
        title: arg_str(args, "title")?.map(str::to_owned),
        description: arg_str(args, "description")?.map(str::to_owned),
        state: arg_str(args, "state")?.map(str::to_owned),
        priority: priority_arg(args)?,
        assignee: arg_str(args, "assignee")?.map(str::to_owned),
        delegate: arg_str(args, "delegate")?.map(str::to_owned),
        estimate,
        labels: opt_list(args, "labels")?,
        add_labels: str_list(args, "addLabels")?,
        due_date: arg_str(args, "dueDate")?
            .map(|s| parse_date("dueDate", s))
            .transpose()?,
        cycle: arg_str(args, "cycle")?.map(str::to_owned),
        project: arg_str(args, "project")?.map(str::to_owned),
        milestone: arg_str(args, "milestone")?.map(str::to_owned),
        parent: arg_str(args, "parentId")?.map(str::to_owned),
        sla_breaches_at: args
            .get("slaBreachesAt")
            .map(sla_at_arg)
            .transpose()?
            .flatten(),
        sla_type: args.get("slaType").map(sla_type_arg).transpose()?.flatten(),
        relations: relation_changes(args)?,
        source: HistorySource::Mcp,
        ..Default::default()
    })
}

fn update_input(id: &str, args: &Value) -> AppResult<UpdateIssueInput> {
    if args.get("template").is_some_and(|v| !v.is_null()) {
        return Err(bad(
            "template",
            "`template` is only valid when creating an issue",
        ));
    }
    let description = match (args.get("description"), args.get("patch")) {
        (Some(_), Some(_)) => {
            return Err(bad(
                "patch",
                "`patch` cannot be combined with `description`",
            ));
        }
        (Some(Value::String(d)), None) => Some(DescriptionEdit::Set(d.clone())),
        (Some(Value::Null), None) => {
            return Err(bad("description", "`description` must be a string"));
        }
        (Some(_), None) => return Err(bad("description", "`description` must be a string")),
        (None, Some(p)) => {
            let ops: Vec<PatchOp> = serde_json::from_value(p.clone())
                .map_err(|e| bad("patch", format!("invalid patch: {e}")))?;
            Some(DescriptionEdit::Patch(ops))
        }
        (None, None) => None,
    };
    let estimate = match args.get("estimate") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => Some(Some(v.as_f64().ok_or_else(|| {
            bad("estimate", "`estimate` must be a number or null")
        })?)),
    };
    let due_date = match args.get("dueDate") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(parse_date("dueDate", s)?)),
        Some(_) => return Err(bad("dueDate", "`dueDate` must be a string or null")),
    };
    Ok(UpdateIssueInput {
        id: id.to_owned(),
        team: arg_str(args, "team")?.map(str::to_owned),
        title: arg_str(args, "title")?.map(str::to_owned),
        description,
        state: arg_str(args, "state")?.map(str::to_owned),
        priority: priority_arg(args)?,
        assignee: tri_str(args, "assignee")?,
        delegate: tri_str(args, "delegate")?,
        estimate,
        labels: opt_list(args, "labels")?,
        add_labels: str_list(args, "addLabels")?,
        remove_labels: str_list(args, "removeLabels")?,
        due_date,
        cycle: tri_str(args, "cycle")?,
        project: tri_str(args, "project")?,
        milestone: tri_str(args, "milestone")?,
        parent: tri_str(args, "parentId")?,
        sort_order: None,
        sla_breaches_at: args.get("slaBreachesAt").map(sla_at_arg).transpose()?,
        sla_type: args.get("slaType").map(sla_type_arg).transpose()?,
        relations: relation_changes(args)?,
        move_sub_issues: false,
        source: HistorySource::Mcp,
    })
}

fn list_input(args: &Value, now: DateTime<Utc>) -> AppResult<(ListIssuesInput, String)> {
    // `query` und `customView` schließen sich aus (docs/05 §4.3, §5.5); die App meldet
    // `invalid_request` auch dann, wenn ein Adapter beide setzt.
    let limit = parse_limit(args, 50, 250)?;
    let fingerprint = query_fingerprint(args);
    let order_by = match arg_str(args, "orderBy")? {
        Some("createdAt") => OrderBy::CreatedAt,
        _ => OrderBy::UpdatedAt,
    };
    let cursor = match arg_str(args, "cursor")? {
        Some(c) => {
            let pos = decode_cursor(c, &fingerprint)?;
            let bad_cursor = || AppError::invalid_request("Invalid cursor");
            let key = pos
                .key
                .as_str()
                .and_then(|k| DateTime::parse_from_rfc3339(k).ok())
                .ok_or_else(bad_cursor)?
                .with_timezone(&Utc);
            let id = pos.id.parse().map_err(|_| bad_cursor())?;
            Some(PageCursor { key, id })
        }
        None => None,
    };
    let time = |key: &str| -> AppResult<Option<DateTime<Utc>>> {
        arg_str(args, key)?
            .map(|s| parse_time_filter(key, s, now))
            .transpose()
    };
    let assignee = match args.get("assignee") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(s.clone())),
        Some(_) => return Err(bad("assignee", "`assignee` must be a string or null")),
    };
    Ok((
        ListIssuesInput {
            custom_view: arg_str(args, "customView")?.map(str::to_owned),
            query: arg_str(args, "query")?.map(str::to_owned),
            filter: None,
            team: arg_str(args, "team")?.map(str::to_owned),
            include_sub_teams: args
                .get("includeSubTeams")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            assignee,
            creator: arg_str(args, "creator")?.map(str::to_owned),
            delegate: arg_str(args, "delegate")?.map(str::to_owned),
            state: arg_str(args, "state")?.map(str::to_owned),
            label: arg_str(args, "label")?.map(str::to_owned),
            project: arg_str(args, "project")?.map(str::to_owned),
            cycle: arg_str(args, "cycle")?.map(str::to_owned),
            parent: arg_str(args, "parentId")?.map(str::to_owned),
            priority: priority_arg(args)?,
            created_after: time("createdAt")?,
            updated_after: time("updatedAt")?,
            triaged_after: time("triagedAt")?,
            include_archived: args
                .get("includeArchived")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            order_by,
            limit: limit as i64,
            cursor,
        },
        fingerprint,
    ))
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_issues" => {
            let (input, fingerprint) = list_input(&args, app.clock.now())?;
            let order_by = input.order_by;
            let res = app.list_issues(ctx, input).await?;
            let base = base_url(app, ctx).await?;
            let handle = caller_handle(app, ctx).await;
            let requested = parse_fields(&args);
            let mut items = Vec::new();
            let mut rows: Vec<(IssueView, String)> = Vec::new();
            for v in &res.items {
                let key = match order_by {
                    OrderBy::CreatedAt => v.issue.created_at,
                    OrderBy::UpdatedAt => v.issue.updated_at,
                };
                rows.push((v.clone(), key.to_rfc3339_opts(SecondsFormat::Micros, true)));
            }
            for (v, _) in &rows {
                let full = issue_map(v, &base, &handle, true)?;
                if requested.is_empty() {
                    let mut out = Map::new();
                    let mut keys: Vec<&str> = DEFAULT_KEYS.to_vec();
                    keys.extend(DEFAULT_KEYS_TAIL);
                    for k in keys {
                        out.insert(k.into(), full.get(k).cloned().unwrap_or(Value::Null));
                    }
                    items.push(Value::Object(out));
                } else {
                    items.push(select_fields(&full, &requested, &LIST_FIELDS));
                }
            }
            let next = res.next.map(|c| {
                encode_cursor(
                    &json!(c.key.to_rfc3339_opts(SecondsFormat::Micros, true)),
                    c.id,
                    &fingerprint,
                )
            });
            Ok(page("issues", items, next))
        }
        "get_issue" => {
            let id = arg_str(&args, "id")?
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| bad("id", "`id` is required"))?;
            let detail = app.get_issue(ctx, id).await?;
            let base = base_url(app, ctx).await?;
            let handle = caller_handle(app, ctx).await;
            let include_relations = args
                .get("includeRelations")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            detail_json(&detail, &base, &handle, &args, include_relations)
        }
        "save_issue" => {
            let relation_params = [
                "blocks",
                "blockedBy",
                "relatedTo",
                "removeBlocks",
                "removeBlockedBy",
                "removeRelatedTo",
                "duplicateOf",
            ]
            .iter()
            .any(|k| args.get(*k).is_some());
            let links = links_arg(&args)?;
            let template = arg_str(&args, "template")?.filter(|t| !t.trim().is_empty());
            let mut sub_issues: Option<(Vec<IssueView>, Vec<String>)> = None;
            let view = match arg_str(&args, "id")? {
                Some(id) => {
                    let input = update_input(id, &args)?;
                    app.update_issue(ctx, input).await?
                }
                None => match template {
                    Some(t) => {
                        let outcome = app
                            .create_issue_from_template(
                                ctx,
                                t,
                                create_input(&args)?,
                                &explicit_nulls(&args),
                            )
                            .await?;
                        sub_issues = Some((outcome.children, outcome.warnings));
                        outcome.root
                    }
                    None => app.create_issue(ctx, create_input(&args)?).await?,
                },
            };
            // `links` (append-only): je Link eine eigene Transaktion nach dem Speichern.
            for (url, title) in links {
                app.link_attachment(
                    ctx,
                    LinkAttachmentInput {
                        issue: view.issue.id.to_string(),
                        url,
                        title,
                        ..Default::default()
                    },
                )
                .await?;
            }
            let base = base_url(app, ctx).await?;
            let handle = caller_handle(app, ctx).await;
            let relations = if relation_params {
                app.issue_relations(ctx, &view.issue.id.to_string()).await?
            } else {
                RelationsView::default()
            };
            let detail = IssueDetail {
                view,
                relations,
                state_history: vec![],
            };
            let mut out = detail_json(&detail, &base, &handle, &json!({}), relation_params)?;
            if let Some(o) = out.as_object_mut() {
                o.remove("attachments");
                o.remove("documents");
                o.remove("stateHistory");
                if !detail.view.previous_identifiers.is_empty() {
                    o.insert(
                        "previousIdentifiers".into(),
                        json!(detail.view.previous_identifiers),
                    );
                }
                if let Some((children, warnings)) = &sub_issues {
                    o.insert(
                        "subIssues".into(),
                        Value::Array(
                            children
                                .iter()
                                .map(
                                    |c| json!({ "id": c.issue.identifier, "title": c.issue.title }),
                                )
                                .collect(),
                        ),
                    );
                    o.insert("warnings".into(), json!(warnings));
                }
            }
            Ok(out)
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by issues"
        ))),
    }
}

#[cfg(test)]
#[path = "issues_tests.rs"]
mod tests;
