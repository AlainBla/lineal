//! Tool-Handler der Gruppe `statuses` (WP02): `list_issue_statuses`, `get_issue_status`
//! (docs/07 §3.3).

use super::teams_users::team_ref;
use lineal_app::usecases::state::StateView;
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};

fn state_json(v: &StateView) -> Value {
    let s = &v.state;
    json!({
        "id": s.id.to_string(), "name": s.name, "type": s.state_type.as_str(), "color": s.color,
        "position": s.position, "description": s.description, "isDefault": v.is_default,
    })
}

fn opt<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_issue_statuses" => {
            let team = opt(&args, "team")
                .filter(|t| !t.trim().is_empty())
                .ok_or_else(|| {
                    AppError::invalid_request("`team` is required")
                        .with_details(json!({ "path": "/team" }))
                })?;
            let states = app.list_workflow_states(ctx, team).await?;
            Ok(json!({
                "statuses": states.iter().map(state_json).collect::<Vec<_>>(),
                "hasNextPage": false,
            }))
        }
        "get_issue_status" => {
            let v = app
                .get_workflow_state(
                    ctx,
                    opt(&args, "id"),
                    opt(&args, "name"),
                    opt(&args, "team"),
                )
                .await?;
            let mut j = state_json(&v);
            if let Some(o) = j.as_object_mut() {
                o.insert("team".into(), team_ref(&v.team));
            }
            Ok(j)
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by statuses"
        ))),
    }
}
