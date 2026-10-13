//! resolve – Cycles: UUID (global), im Team-Kontext zusätzlich Nummer (`"12"`) und Name.
//!
//! Der Cycle muss zu einem für den Akteur zugänglichen Team gehören (sonst `not_found`).

use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::cycle::Cycle;
use crate::ports::repos::team::Team;
use lineal_domain::ids::CycleId;
use serde_json::json;
use uuid::Uuid;

fn not_found(reference: &str) -> AppError {
    AppError::not_found(format!("Could not find cycle \"{reference}\""))
        .with_details(json!({ "reference": reference, "type": "Cycle" }))
}

/// Lädt einen Cycle per ID und prüft die Sichtbarkeit seines Teams.
pub async fn by_id(tx: &mut dyn Tx, snap: &PolicySnapshot, id: CycleId) -> AppResult<Cycle> {
    let c = tx
        .cycles()
        .get(id)
        .await?
        .filter(|c| snap.visible_team_ids().contains(&c.team_id))
        .ok_or_else(|| not_found(&id.to_string()))?;
    Ok(c)
}

/// Löst eine Cycle-Referenz auf. Ohne `team` ist nur die UUID eindeutig.
pub async fn resolve_in(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    team: Option<&Team>,
    reference: &str,
) -> AppResult<Cycle> {
    let r = reference.trim();
    if let Ok(uuid) = Uuid::parse_str(r) {
        let c = by_id(tx, snap, CycleId(uuid)).await?;
        if team.is_some_and(|t| t.id != c.team_id) {
            return Err(not_found(reference));
        }
        return Ok(c);
    }
    let Some(team) = team else {
        return Err(AppError::ambiguous_reference(format!(
            "Cycle reference \"{reference}\" needs a team context (cycle numbers are team-local)."
        ))
        .with_details(json!({ "reference": reference, "type": "Cycle" })));
    };
    let cycles = tx.cycles().list_for_team(team.id, false).await?;
    if let Ok(n) = r.parse::<i32>()
        && let Some(c) = cycles.iter().find(|c| c.number == n)
    {
        return Ok(c.clone());
    }
    let lower = r.to_lowercase();
    let mut named: Vec<&Cycle> = cycles
        .iter()
        .filter(|c| c.name.as_ref().is_some_and(|n| n.to_lowercase() == lower))
        .collect();
    match named.len() {
        // Angezeigter Standardname unbenannter Cycles: „Cycle 2“ (B8.34).
        0 => lower
            .strip_prefix("cycle ")
            .and_then(|rest| rest.trim().parse::<i32>().ok())
            .and_then(|n| cycles.iter().find(|c| c.number == n && c.name.is_none()))
            .cloned()
            .ok_or_else(|| not_found(reference)),
        1 => Ok(named.remove(0).clone()),
        _ => Err(AppError::ambiguous_reference(format!(
            "Cycle name \"{reference}\" matches several cycles."
        ))
        .with_details(json!({
            "reference": reference, "type": "Cycle",
            "candidates": named.iter().take(10).map(|c| json!({
                "id": c.id.to_string(), "name": c.name, "key": c.number.to_string(),
            })).collect::<Vec<_>>(),
        }))),
    }
}
