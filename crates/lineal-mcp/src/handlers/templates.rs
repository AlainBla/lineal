//! Tool-Handler der Gruppe `templates` (WP10): `list_templates`, `get_template` (docs/07 §3.16).
//!
//! Referenzen kommen wie gespeichert als IDs (docs/06 §2.4). Das Anwenden eines Templates läuft über
//! `save_issue`/`save_project` mit `template`; die Verwaltung (anlegen/ändern/löschen) gibt es nur
//! in der öffentlichen API (docs/06 §4).

use super::comments::{arg_str, required_str};
use super::teams_users::ts;
use crate::envelope::{opt_text, page, team_ref};
use lineal_app::usecases::template::TemplateView;
use lineal_app::{App, AppError, AppResult, RequestCtx};
use lineal_domain::enums::TemplateType;
use serde_json::{Map, Value, json};

fn team_json(v: &TemplateView) -> Value {
    v.team
        .as_ref()
        .map_or(Value::Null, |(id, key, name)| team_ref(id, key, name))
}

/// Eintrag der Listenantwort.
fn list_item(v: &TemplateView) -> Value {
    let t = &v.template;
    json!({
        "id": t.id.to_string(),
        "name": t.name,
        "type": t.template_type.as_str(),
        "description": opt_text(t.description.as_deref()),
        "team": team_json(v),
        "isForm": v.is_form,
        "updatedAt": ts(t.updated_at),
    })
}

/// Sub-Issue-Eintrag mit Defaults (`labelIds: []`, `assigneeId: null`).
fn sub_issue_json(v: &Value) -> Value {
    let mut m: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    m.entry("labelIds").or_insert_with(|| json!([]));
    m.entry("assigneeId").or_insert(Value::Null);
    if let Some(Value::Array(children)) = m.get("subIssues").cloned() {
        m.insert(
            "subIssues".into(),
            Value::Array(children.iter().map(sub_issue_json).collect()),
        );
    }
    Value::Object(m)
}

/// Antwort von `get_template`.
fn detail(v: &TemplateView) -> Value {
    let t = &v.template;
    let data = t.template_data.as_object().cloned().unwrap_or_default();
    let mut content = Map::new();
    let mut sub_issues: Vec<Value> = Vec::new();
    let mut form_fields = Value::Array(vec![]);
    match t.template_type {
        TemplateType::Issue => {
            for key in [
                "title",
                "description",
                "stateId",
                "stateType",
                "priority",
                "estimate",
                "labelIds",
                "assigneeId",
                "projectId",
                "projectMilestoneId",
                "cycle",
                "dueIn",
                "dueDate",
                "subscriberIds",
            ] {
                if let Some(x) = data.get(key) {
                    content.insert(key.into(), x.clone());
                }
            }
            // Default-Schlüssel der Antwort (docs/07 §3.16), auch wenn leer.
            for key in [
                "stateId",
                "assigneeId",
                "projectId",
                "estimate",
                "title",
                "description",
            ] {
                content.entry(key).or_insert(Value::Null);
            }
            content.entry("labelIds").or_insert_with(|| json!([]));
            if let Some(Value::Array(subs)) = data.get("subIssues") {
                sub_issues = subs.iter().map(sub_issue_json).collect();
            }
            if let Some(f) = data.get("formFields") {
                form_fields = f.clone();
            }
        }
        TemplateType::Project | TemplateType::Document => {
            content = data;
        }
    }
    json!({
        "id": t.id.to_string(),
        "name": t.name,
        "type": t.template_type.as_str(),
        "description": opt_text(t.description.as_deref()),
        "team": team_json(v),
        "content": content,
        "subIssues": sub_issues,
        "formFields": form_fields,
        "isForm": v.is_form,
    })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_templates" => {
            let ty = match arg_str(&args, "type")? {
                None => None,
                Some(s) => Some(s.parse::<TemplateType>().map_err(|_| {
                    AppError::invalid_request("`type` must be issue, project or document")
                        .with_details(json!({ "path": "/type" }))
                })?),
            };
            let team = arg_str(&args, "team")?.filter(|t| !t.trim().is_empty());
            let items = app.list_templates(ctx, team, ty).await?;
            Ok(page(
                "templates",
                items.iter().map(list_item).collect(),
                None,
            ))
        }
        "get_template" => {
            let v = app.get_template(ctx, required_str(&args, "id")?).await?;
            Ok(detail(&v))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` does not belong to the templates group"
        ))),
    }
}

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
