//! resolve – Workflow-Status innerhalb eines Teams: UUID → Name (case-insensitiv) → Kategoriename
//! (erster Status dieser Kategorie nach `position`), docs/00 §2.

use crate::error::{AppError, AppResult};
use crate::ports::Tx;
use crate::ports::repos::state::WorkflowState;
use crate::ports::repos::team::Team;
use lineal_domain::enums::StateType;
use lineal_domain::ids::StateId;
use serde_json::json;
use uuid::Uuid;

fn not_found(reference: &str) -> AppError {
    AppError::not_found(format!("Could not find status \"{reference}\""))
        .with_details(json!({ "reference": reference, "type": "WorkflowState" }))
}

/// Löst eine Status-Referenz im Kontext von `team` auf. Archivierte Status werden nur über
/// ihre UUID gefunden. Ein Status eines anderen Teams ist `not_found`.
pub async fn resolve(tx: &mut dyn Tx, team: &Team, reference: &str) -> AppResult<WorkflowState> {
    let reference = reference.trim();
    // 1. UUID
    if let Ok(uuid) = Uuid::parse_str(reference) {
        return match tx.states().get(StateId(uuid)).await? {
            Some(s) if s.team_id == team.id => Ok(s),
            _ => Err(not_found(reference)),
        };
    }
    // 5. exakter Name im Team
    if let Some(s) = tx.states().find_by_name(team.id, reference).await? {
        return Ok(s);
    }
    // 7. Kategoriename -> erster Status dieser Kategorie nach position
    if let Ok(kind) = reference.parse::<StateType>() {
        let states = tx.states().list_for_team(team.id, false).await?;
        if let Some(s) = states.into_iter().find(|s| s.state_type == kind) {
            return Ok(s);
        }
    }
    Err(not_found(reference))
}

/// Lädt einen Status per ID (ohne Team-Kontext); die Sichtbarkeit des Teams prüft der Aufrufer.
pub async fn by_id(tx: &mut dyn Tx, id: StateId) -> AppResult<WorkflowState> {
    tx.states()
        .get(id)
        .await?
        .ok_or_else(|| not_found(&id.to_string()))
}
