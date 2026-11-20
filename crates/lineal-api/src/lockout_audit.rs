//! LIN-58 Teil 3: Audit-Senke, die `auth.distributed_lockout` schreibt.
//!
//! Der Brute-Force-Zähler ([`crate::auth_failures`]) ist synchron und hält kurz seinen Mutex; er darf
//! dort keine async-DB-Arbeit tun. Diese Senke kehrt daher sofort zurück und erledigt das
//! Auflösen der Adresse auf Workspaces und das Schreiben der Audit-Einträge in einer losgelösten
//! Task (`tokio::spawn`) – genau wie `record_failed_logins` im App-Layer.

use crate::auth_failures::LockoutAudit;
use lineal_app::App;
use std::sync::Arc;

/// Verbindet den Zähler mit dem App-Layer.
pub struct AppLockoutAudit {
    app: Arc<App>,
}

impl AppLockoutAudit {
    pub fn new(app: Arc<App>) -> Self {
        Self { app }
    }
}

impl LockoutAudit for AppLockoutAudit {
    fn distributed_lockout(
        &self,
        account: &str,
        ip_count: u32,
        failures: u32,
        window_ms: i64,
        allowed_known_ip: bool,
    ) {
        let app = self.app.clone();
        let account = account.to_owned();
        tokio::spawn(async move {
            if let Err(e) = app
                .record_distributed_lockout(
                    &account,
                    ip_count,
                    failures,
                    window_ms,
                    allowed_known_ip,
                )
                .await
            {
                tracing::warn!(error = %e, "distributed lockout audit not written");
            }
        });
    }
}
