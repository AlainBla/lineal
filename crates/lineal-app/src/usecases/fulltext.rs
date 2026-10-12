//! usecases – Volltextkonfiguration des Workspace (WP34, docs/05 §5): `simple` (Default, keine
//! Wortstämme), `german` oder `english`. Die Suchvektoren pflegen Datenbank-Trigger; ein Wechsel
//! baut sie für den ganzen Workspace in derselben Transaktion neu auf.

use super::audit::AuditEvent;
use super::team::emit;
use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use lineal_domain::events::DomainEvent;
use serde_json::json;

/// Erlaubte Konfigurationen.
pub const FULLTEXT_CONFIGS: [&str; 3] = ["simple", "german", "english"];

impl App {
    /// Liest die Volltextkonfiguration (Admin/Owner).
    pub async fn get_fulltext_config(&self, ctx: &RequestCtx) -> AppResult<String> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::UpdateWorkspaceSettings)?;
        tx.orgs().fulltext_config().await
    }

    /// Setzt die Volltextkonfiguration (Admin/Owner) und protokolliert im Audit-Log.
    pub async fn set_fulltext_config(&self, ctx: &RequestCtx, config: &str) -> AppResult<String> {
        if matches!(ctx.actor, Actor::System) {
            return Err(AppError::forbidden(
                "The full-text configuration cannot be changed by system contexts.",
            ));
        }
        if !FULLTEXT_CONFIGS.contains(&config) {
            return Err(AppError::invalid_request(
                "fulltextConfig must be simple, german or english",
            )
            .with_details(json!({ "field": "fulltextConfig" })));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::UpdateWorkspaceSettings)?;
        let before = tx.orgs().fulltext_config().await?;
        if before != config {
            tx.orgs()
                .set_fulltext_config(config, self.clock.now())
                .await?;
            emit(&mut *tx, ctx, DomainEvent::OrganizationUpdated {}).await?;
            self.record_audit(
                &mut *tx,
                ctx,
                AuditEvent::new(
                    "workspace.fulltext_config_updated",
                    "workspace",
                    Some(ctx.org_id.0),
                    json!({ "changes": { "fulltextConfig": { "from": before, "to": config } } }),
                ),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(config.to_owned())
    }
}
