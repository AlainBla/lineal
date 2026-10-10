//! Tool-Handler der Gruppe `workspace` (WP11): `get_workspace_overview` (docs/14 §4), eine
//! dokumentierte Lineal-Erweiterung über den Linear-MCP hinaus. Die Zahlen stammen aus
//! `App::workspace_overview` – demselben Use-Case wie GraphQL `workspaceOverview`.

use lineal_app::usecases::overview::{OverviewInput, OverviewPeriod};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "get_workspace_overview" => {
            let teams = match args.get("teams") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a
                    .iter()
                    .map(|v| {
                        v.as_str().map(str::to_owned).ok_or_else(|| {
                            AppError::invalid_request("`teams` must contain strings")
                                .with_details(json!({ "path": "/teams" }))
                        })
                    })
                    .collect::<AppResult<Vec<_>>>()?,
                Some(_) => {
                    return Err(AppError::invalid_request("`teams` must be an array")
                        .with_details(json!({ "path": "/teams" })));
                }
            };
            let period = match args.get("period").and_then(Value::as_str) {
                None => OverviewPeriod::Days7,
                Some(p) => OverviewPeriod::parse(p).ok_or_else(|| {
                    AppError::invalid_request("`period` must be 7d or 30d")
                        .with_details(json!({ "path": "/period" }))
                })?,
            };
            let overview = app
                .workspace_overview(
                    ctx,
                    OverviewInput {
                        teams,
                        include_sub_teams: args
                            .get("includeSubTeams")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        period,
                    },
                )
                .await?;
            Ok(overview.to_json())
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by workspace"
        ))),
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
