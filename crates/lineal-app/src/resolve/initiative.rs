//! resolve – Initiativen: UUID → Kennung (`I-4`) → Slug/`slugId` → exakter Name.
//!
//! Eine Initiative mit privatem (für den Akteur unzugänglichem) Lead-Team ist unsichtbar
//! (`not_found`, docs/04 §4.6). Gäste dürfen Initiativen nicht sehen (`forbidden`); das prüft der
//! Use-Case vor der Auflösung.

use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::initiative::Initiative;
use lineal_domain::ids::InitiativeId;
use serde_json::json;
use uuid::Uuid;

fn not_found(reference: &str) -> AppError {
    AppError::not_found(format!("Could not find initiative \"{reference}\""))
        .with_details(json!({ "reference": reference, "type": "Initiative" }))
}

/// Ob die Initiative für den Akteur sichtbar ist (kein Lead-Team oder zugängliches Lead-Team).
pub fn is_visible(snap: &PolicySnapshot, i: &Initiative) -> bool {
    match i.lead_team_id {
        None => true,
        Some(t) => snap.visible_team_ids().contains(&t),
    }
}

fn parse_identifier(r: &str) -> Option<i32> {
    r.strip_prefix("I-")
        .or_else(|| r.strip_prefix("i-"))?
        .parse()
        .ok()
}

fn is_slug_id(s: &str) -> bool {
    s.len() == 12
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Löst eine Initiative-Referenz mit einem bereits geladenen Snapshot auf.
pub async fn resolve_in(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    reference: &str,
) -> AppResult<Initiative> {
    let r = reference.trim();
    let mut found: Option<Initiative> = None;
    if let Ok(uuid) = Uuid::parse_str(r) {
        found = tx
            .initiatives()
            .get(InitiativeId(uuid))
            .await?
            .filter(|i| !i.trashed);
    } else if let Some(n) = parse_identifier(r) {
        found = tx.initiatives().find_by_number(n).await?;
    } else {
        let slug = r.rsplit('-').next().unwrap_or(r);
        if is_slug_id(r) {
            found = tx.initiatives().find_by_slug_id(r).await?;
        } else if r.contains('-') && is_slug_id(slug) {
            found = tx.initiatives().find_by_slug_id(slug).await?;
        }
    }
    if let Some(i) = found {
        return if is_visible(snap, &i) {
            Ok(i)
        } else {
            Err(not_found(reference))
        };
    }
    let mut candidates: Vec<Initiative> = tx
        .initiatives()
        .find_by_name(r)
        .await?
        .into_iter()
        .filter(|i| is_visible(snap, i))
        .collect();
    match candidates.len() {
        0 => Err(not_found(reference)),
        1 => Ok(candidates.remove(0)),
        _ => Err(AppError::ambiguous_reference(format!(
            "Initiative name \"{reference}\" matches several initiatives."
        ))
        .with_details(json!({
            "reference": reference, "type": "Initiative",
            "candidates": candidates.iter().take(10).map(|i| json!({
                "id": i.id.to_string(), "name": i.name, "key": i.identifier(),
            })).collect::<Vec<_>>(),
        }))),
    }
}

/// Löst eine Initiative-Referenz auf (lädt den Snapshot selbst).
pub async fn resolve(
    tx: &mut dyn Tx,
    ctx: &crate::context::RequestCtx,
    reference: &str,
) -> AppResult<Initiative> {
    let snap = PolicySnapshot::load(tx, ctx).await?;
    resolve_in(tx, &snap, reference).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_shape() {
        assert_eq!(parse_identifier("I-4"), Some(4));
        assert_eq!(parse_identifier("i-12"), Some(12));
        assert_eq!(parse_identifier("P-ENG-4"), None);
    }
}
