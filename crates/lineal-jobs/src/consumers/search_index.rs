//! Outbox-Konsument `search_index` (WP11). Bewusst ein No-op: Die Volltextspalten (`search_vector`
//! auf issue, project, initiative, document, comment) sind generierte Spalten (Migration 0015/1100) und
//! werden von PostgreSQL selbst gepflegt; die Berechtigung wird zur Abfragezeit angehängt, ein
//! Reindex bei Sichtbarkeitswechsel ist daher nicht nötig (docs/10 §5.3).

use crate::EventConsumer;
use async_trait::async_trait;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::{App, AppResult};

pub struct SearchIndexConsumer;

#[async_trait]
impl EventConsumer for SearchIndexConsumer {
    fn name(&self) -> &'static str {
        "search_index"
    }

    async fn handle(&self, _app: &App, _ev: &StoredEvent) -> AppResult<()> {
        Ok(())
    }
}
