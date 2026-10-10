//! resolve – Teams: UUID → Key (aktiv oder historisch, case-insensitiv) → exakter Name.
//!
//! Kandidaten werden **vor** der Mehrdeutigkeitsprüfung auf zugängliche Teams gefiltert
//! (docs/02 §6.6); nicht zugängliche Teams sind `not_found` mit `details.reference`/`details.type`.

use crate::error::{AppError, AppResult};
use crate::policy::{PolicySnapshot, team_not_found};
use crate::ports::Tx;
use crate::ports::repos::team::Team;
use lineal_domain::ids::TeamId;
use serde_json::json;
use std::collections::BTreeSet;
use uuid::Uuid;

/// Worauf sich die Sichtbarkeit der Kandidaten bezieht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Teams, deren Inhalte der Akteur sehen darf (`visible_team_ids`).
    Content,
    /// Zusätzlich alle Teams als Metadaten, wenn der Akteur `admin`/`owner` ist (Verwaltung).
    Management,
}

fn not_found(reference: &str) -> AppError {
    team_not_found().with_details(json!({ "reference": reference, "type": "Team" }))
}

fn candidate_ids(snap: &PolicySnapshot, visibility: Visibility) -> BTreeSet<TeamId> {
    let mut ids = snap.visible_team_ids();
    if visibility == Visibility::Management && (snap.actor.is_admin() || snap.actor.is_system()) {
        ids.extend(snap.tree.all().filter(|t| !t.trashed).map(|t| t.id));
    }
    ids
}

/// Löst eine Team-Referenz mit einem bereits geladenen Snapshot auf.
pub async fn resolve_in(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    reference: &str,
    visibility: Visibility,
) -> AppResult<Team> {
    let reference = reference.trim();
    let candidates = candidate_ids(snap, visibility);
    let visible = |id: TeamId| candidates.contains(&id);

    // 1. UUID
    if let Ok(uuid) = Uuid::parse_str(reference) {
        let id = TeamId(uuid);
        return match snap.tree.get(id) {
            Some(t) if visible(id) => Ok(t.clone()),
            _ => Err(not_found(reference)),
        };
    }
    // 4. Key (aktiv zuerst, dann Historie), case-insensitiv
    let key = reference.to_uppercase();
    if let Some(id) = tx.teams().find_id_by_key(&key).await?
        && visible(id)
        && let Some(t) = snap.tree.get(id)
    {
        return Ok(t.clone());
    }
    // 5. exakter Name, case-insensitiv; Mehrdeutigkeit nur unter den sichtbaren Kandidaten
    let lower = reference.to_lowercase();
    let matches: Vec<&Team> = snap
        .tree
        .all()
        .filter(|t| visible(t.id) && t.name.to_lowercase() == lower)
        .collect();
    match matches.as_slice() {
        [] => Err(not_found(reference)),
        [one] => Ok((*one).clone()),
        many => Err(AppError::ambiguous_reference(format!(
            "Team name \"{reference}\" matches several teams."
        ))
        .with_details(json!({
            "reference": reference,
            "type": "Team",
            "candidates": many.iter().take(10).map(|t| json!({
                "id": t.id.to_string(), "name": t.name, "key": t.key
            })).collect::<Vec<_>>(),
        }))),
    }
}

/// Löst eine Team-Referenz auf (lädt den Snapshot selbst).
pub async fn resolve(
    tx: &mut dyn Tx,
    ctx: &crate::context::RequestCtx,
    reference: &str,
    visibility: Visibility,
) -> AppResult<Team> {
    let snap = PolicySnapshot::load(tx, ctx).await?;
    resolve_in(tx, &snap, reference, visibility).await
}
