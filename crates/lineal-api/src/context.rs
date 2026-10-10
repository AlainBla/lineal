//! Zugriff auf Context-Daten in GraphQL-Resolvern (WP03 erweitert).

use async_graphql::Context;
use lineal_app::{App, RequestCtx};
use std::sync::Arc;

/// Die `App` aus den Schema-Daten.
pub fn app<'a>(ctx: &Context<'a>) -> async_graphql::Result<&'a Arc<App>> {
    ctx.data::<Arc<App>>()
}

/// Der `RequestCtx` des Aufrufers (von der Auth-Middleware pro Request gesetzt).
pub fn request_ctx<'a>(ctx: &Context<'a>) -> async_graphql::Result<&'a RequestCtx> {
    ctx.data::<RequestCtx>()
}
