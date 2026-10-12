//! resolve – Issues: UUID → Kennung (`ENG-123`, inkl. früherer Kennungen/Aliase) (docs/00 §2).
//!
//! Sichtbarkeit: ein Issue gilt nur dann als vorhanden, wenn sein Team für den Akteur sichtbar ist
//! oder das Issue mit ihm geteilt wurde; sonst `not_found` (nie `forbidden`).

use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::issue::Issue;
use crate::resolve::user::actor_user_id;
use lineal_domain::identifier::{ParsedIdentifier, parse_identifier};
use lineal_domain::ids::IssueId;
use serde_json::json;
use uuid::Uuid;

/// Fehler für nicht auflösbare oder nicht sichtbare Issues.
pub fn issue_not_found(reference: &str) -> AppError {
    AppError::not_found("Could not find referenced Issue.")
        .with_details(json!({ "reference": reference, "type": "Issue" }))
}

/// Löst eine Issue-Referenz auf; liefert auch Issues im Papierkorb (Aufrufer prüft `trashed`).
pub async fn resolve_in(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    reference: &str,
) -> AppResult<Issue> {
    let reference = reference.trim();
    let id = lookup(tx, reference).await?;
    let issue = tx
        .issues()
        .get(id, false)
        .await?
        .ok_or_else(|| issue_not_found(reference))?;
    if !can_see(tx, snap, ctx, &issue).await? {
        return Err(issue_not_found(reference));
    }
    Ok(issue)
}

/// Ob der Akteur das Issue sehen darf (Team sichtbar oder Issue geteilt).
pub async fn can_see(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    issue: &Issue,
) -> AppResult<bool> {
    if snap.visible_team_ids().contains(&issue.team_id) {
        return Ok(true);
    }
    match actor_user_id(ctx) {
        Some(u) => Ok(tx.issues().get_share(issue.id, u).await?.is_some()),
        None => Ok(false),
    }
}

pub(crate) async fn lookup(tx: &mut dyn Tx, reference: &str) -> AppResult<IssueId> {
    if let Ok(uuid) = Uuid::parse_str(reference) {
        return Ok(IssueId(uuid));
    }
    match parse_identifier(reference) {
        Some(ParsedIdentifier::Issue(ident)) => {
            if let Some(team) = tx.teams().find_id_by_key(ident.team_key.as_str()).await?
                && let Some(id) = tx.issues().find_by_number(team, ident.number).await?
            {
                return Ok(id);
            }
            tx.issues()
                .find_by_alias(&ident.to_string())
                .await?
                .ok_or_else(|| issue_not_found(reference))
        }
        Some(ParsedIdentifier::Project(_)) => Err(AppError::invalid_request(format!(
            "\"{reference}\" is a project identifier, not an issue ID. Use get_project instead"
        ))
        .with_details(json!({ "reference": reference, "type": "Issue" }))),
        Some(ParsedIdentifier::Initiative { .. }) => Err(AppError::invalid_request(format!(
            "\"{reference}\" is an initiative identifier, not an issue ID. Use get_initiative instead"
        ))
        .with_details(json!({ "reference": reference, "type": "Issue" }))),
        None => Err(issue_not_found(reference)),
    }
}
