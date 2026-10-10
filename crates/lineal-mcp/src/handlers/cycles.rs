//! Tool-Handler der Gruppe `cycles` (WP07): `list_cycles` (docs/07 §3.9). Antwort: `isCurrent`
//! statt `isActive`, `progress` 0..100, aufsteigend nach Nummer, immer `hasNextPage: false`.

use super::projects::{pct, required};
use super::teams_users::{team_ref, ts};
use crate::envelope::page;
use lineal_app::usecases::cycle::{CycleKind, CycleView};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};

fn cycle_json(v: &CycleView) -> Value {
    let c = &v.cycle;
    json!({
        "id": c.id.to_string(), "number": c.number, "name": c.name, "team": team_ref(&v.team),
        "startsAt": ts(c.starts_at), "endsAt": ts(c.ends_at), "completedAt": c.completed_at.map(ts),
        "isCurrent": v.is_active, "isPrevious": v.is_previous, "isNext": v.is_next,
        "isPast": v.is_past, "isFuture": v.is_future,
        "progress": pct(v.progress),
        "issueCountHistory": c.issue_count_history,
        "completedIssueCountHistory": c.completed_issue_count_history,
        "scopeHistory": c.scope_history,
        "completedScopeHistory": c.completed_scope_history,
        "inProgressScopeHistory": c.in_progress_scope_history,
    })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_cycles" => {
            let kind = match args.get("type").and_then(Value::as_str) {
                None => None,
                Some("current") => Some(CycleKind::Current),
                Some("previous") => Some(CycleKind::Previous),
                Some("next") => Some(CycleKind::Next),
                Some(_) => {
                    return Err(AppError::invalid_request(
                        "`type` must be current, previous or next",
                    ));
                }
            };
            let (_, views) = app
                .list_cycles(ctx, required(&args, "teamId")?, kind)
                .await?;
            Ok(page("cycles", views.iter().map(cycle_json).collect(), None))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by cycles"
        ))),
    }
}
