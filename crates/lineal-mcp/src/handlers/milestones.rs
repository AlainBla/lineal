//! Tool-Handler der Gruppe `milestones` (WP07): `list_milestones`, `get_milestone`,
//! `save_milestone` (docs/07 §3.8). `progress` ist 0..100.

use super::projects::{arg_str, milestone_json, required};
use crate::envelope::page;
use lineal_app::usecases::project::milestone::SaveMilestoneInput;
use lineal_app::usecases::project::pure::parse_date;
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_milestones" => {
            let (p, list) = app
                .list_milestones(ctx, required(&args, "project")?)
                .await?;
            Ok(page(
                "milestones",
                list.iter().map(|m| milestone_json(&p, m)).collect(),
                None,
            ))
        }
        "get_milestone" => {
            let (p, m) = app
                .get_milestone(ctx, required(&args, "project")?, required(&args, "query")?)
                .await?;
            let mut j = milestone_json(&p, &m);
            j["issueCount"] = json!(m.issue_count);
            j["completedIssueCount"] = json!(m.completed_issue_count);
            Ok(j)
        }
        "save_milestone" => {
            let target_date = match args.get("targetDate") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(v) => {
                    let s = v.as_str().unwrap_or_default();
                    Some(Some(parse_date("targetDate", s)?))
                }
            };
            let (p, m) = app
                .save_milestone(
                    ctx,
                    SaveMilestoneInput {
                        project: required(&args, "project")?.to_owned(),
                        id: arg_str(&args, "id").map(str::to_owned),
                        name: arg_str(&args, "name").map(str::to_owned),
                        description: arg_str(&args, "description").map(str::to_owned),
                        target_date,
                    },
                )
                .await?;
            Ok(milestone_json(&p, &m))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by milestones"
        ))),
    }
}
