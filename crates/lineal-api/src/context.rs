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

/// Wie oft `Issue.children`/`relations`/`history` pro Request höchstens aufgelöst werden dürfen.
/// Sie laden beim Zugriff nach (eigene Transaktion, Policy-Snapshot, bis 1000 History-Zeilen) und
/// sind für die Detailansicht gedacht, nicht für Listen.
pub const DETAIL_FIELD_LIMIT: u32 = 10;

/// Request-Budget für die Nachladefelder (wird je Request frisch in die Context-Daten gelegt).
#[derive(Debug, Default)]
pub struct DetailBudget(std::sync::atomic::AtomicU32);

impl DetailBudget {
    /// Verbraucht einen Aufruf; `false`, wenn das Budget erschöpft ist (atomar).
    pub fn take(&self) -> bool {
        self.0
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |n| (n < DETAIL_FIELD_LIMIT).then_some(n + 1),
            )
            .is_ok()
    }
}

/// Bucht einen Aufruf eines Nachladefelds. Fehlt das Budget in den Context-Daten (Ausführung
/// am Router vorbei), wird abgelehnt statt unbegrenzt erlaubt.
pub fn charge_detail_field(ctx: &Context<'_>) -> async_graphql::Result<()> {
    if ctx
        .data_opt::<DetailBudget>()
        .is_some_and(DetailBudget::take)
    {
        Ok(())
    } else {
        Err(crate::graphql::org::gql_err(
            lineal_app::AppError::invalid_request(
                "children, relations and history are only available in the detail view",
            ),
        ))
    }
}

/// Herkunft einer Änderung im Verlauf: Cookie-Session = Web-App (`user`), API-Key und
/// OAuth-Token = `api` (docs/01, B3.18).
pub fn history_source(ctx: &RequestCtx) -> lineal_app::ports::repos::issue::HistorySource {
    use lineal_app::ports::repos::issue::HistorySource;
    if ctx.via_session {
        HistorySource::User
    } else {
        HistorySource::Api
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_app::ports::repos::issue::HistorySource;

    #[test]
    fn web_sessions_are_user_everything_else_is_api() {
        let mut ctx = RequestCtx::system(lineal_domain::ids::OrgId::new_v7(), "r");
        ctx.via_session = true;
        assert_eq!(history_source(&ctx), HistorySource::User);
        ctx.via_session = false;
        assert_eq!(history_source(&ctx), HistorySource::Api);
    }
}
