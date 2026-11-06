//! GraphQL: persönlicher Kalender-Feed (`calendarFeedEnabled`, `calendarFeedRotate`,
//! `calendarFeedRevoke`). Die URL erscheint nur beim Anlegen, gespeichert wird ihr Hash.

use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, Object};

#[derive(Default)]
pub struct CalendarQuery;

#[Object]
impl CalendarQuery {
    /// Ob die angemeldete Person einen Kalender-Feed hat.
    async fn calendar_feed_enabled(&self, ctx: &Context<'_>) -> async_graphql::Result<bool> {
        app(ctx)?
            .calendar_feed_enabled(request_ctx(ctx)?)
            .await
            .map_err(gql_err)
    }
}

#[derive(Default)]
pub struct CalendarMutation;

#[Object]
impl CalendarMutation {
    /// Legt den Feed an oder ersetzt ihn und liefert die neue URL.
    async fn calendar_feed_rotate(&self, ctx: &Context<'_>) -> async_graphql::Result<String> {
        app(ctx)?
            .rotate_calendar_feed(request_ctx(ctx)?)
            .await
            .map_err(gql_err)
    }

    /// Schaltet den Feed ab.
    async fn calendar_feed_revoke(&self, ctx: &Context<'_>) -> async_graphql::Result<bool> {
        app(ctx)?
            .revoke_calendar_feed(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        Ok(true)
    }
}
