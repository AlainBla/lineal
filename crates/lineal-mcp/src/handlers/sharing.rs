//! Tool-Handler der Gruppe `sharing` (WP06): `share_issue`, `unshare_issue` (docs/07 §3.17).

use super::teams_users::ts;
use lineal_app::usecases::issue::ShareResult;
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};

fn bad(path: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "path": format!("/{path}") }))
}

fn required<'a>(args: &'a Value, key: &str) -> AppResult<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad(key, format!("`{key}` is required")))
}

fn result_json(r: &ShareResult) -> Value {
    let mut j = json!({
        "issue": r.issue.identifier,
        "user": { "id": r.user_id.to_string(), "name": r.user_name },
        "shared": r.shared,
    });
    if let Some(o) = j.as_object_mut() {
        if let Some(at) = r.shared_at {
            o.insert("sharedAt".into(), json!(ts(at)));
        }
        if let Some(m) = &r.message {
            o.insert("message".into(), json!(m));
        }
    }
    j
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    let issue = required(&args, "issue")?;
    let user = required(&args, "user")?;
    // Strenge Variante: nur E-Mail oder UUID (docs/07 §3.17).
    if !(user.contains('@') || uuid::Uuid::parse_str(user.trim()).is_ok()) {
        return Err(bad(
            "user",
            "`user` must be an email address or a user UUID (names and \"me\" are not accepted)",
        ));
    }
    match tool {
        "share_issue" => Ok(result_json(&app.share_issue(ctx, issue, user).await?)),
        "unshare_issue" => Ok(result_json(&app.unshare_issue(ctx, issue, user).await?)),
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by sharing"
        ))),
    }
}
