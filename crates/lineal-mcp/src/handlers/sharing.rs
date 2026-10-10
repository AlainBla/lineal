//! Tool-Handler der Gruppe `sharing` (WP06). Im Scaffold: `not implemented`.

use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::Value;

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(_app: &App, _ctx: &RequestCtx, _tool: &str, _args: Value) -> AppResult<Value> {
    Err(AppError::internal("not implemented"))
}
