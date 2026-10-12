//! resolve – Issues: UUID → Kennung (`ENG-123`, inkl. früherer Kennungen/Aliase) (docs/00 §2).
//!
//! Sichtbarkeit: ein Issue gilt nur dann als vorhanden, wenn sein Team für den Akteur sichtbar ist
//! oder das Issue mit ihm geteilt wurde; sonst `not_found` (nie `forbidden`).

use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::issue::Issue;
use lineal_domain::identifier::{ParsedIdentifier, parse_identifier};
use lineal_domain::ids::{IssueId, TeamId, UserId};
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
    share_grants_access(tx, snap, ctx, issue.id).await
}

/// Für wen Freigaben gelten: nur Menschen (keine Apps), nur solange der Workspace Freigaben
/// erlaubt und nie für team-beschränkte API-Keys (ein Key mit `team_scope` darf über eine
/// Freigabe nicht aus seinen Teams heraus).
pub fn share_recipient(snap: &PolicySnapshot, ctx: &RequestCtx) -> Option<UserId> {
    match ctx.actor {
        crate::context::Actor::User { id, .. }
            if ctx.team_scope.is_none() && snap.org.limits.issue_sharing_enabled =>
        {
            Some(id)
        }
        _ => None,
    }
}

/// Wie [`share_recipient`], zusätzlich muss das Issue mit dem Akteur geteilt sein.
pub async fn share_grants_access(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    issue: IssueId,
) -> AppResult<bool> {
    match share_recipient(snap, ctx) {
        Some(user) => Ok(tx.issues().get_share(issue, user).await?.is_some()),
        None => Ok(false),
    }
}

/// Zentrale Prüfung für Unterobjekte eines Issues (Kommentare, Reaktionen, Anhänge): `action` auf
/// dem Team des Issues. Ist das Team unsichtbar, genügt eine Freigabe an den Akteur für Lesen,
/// Kommentieren und Reagieren (`ViewTeam`, `CommentIssue`, `ReactIssue`) – nie für mehr und nie
/// für andere Issues des Teams. Der Token-Scope (`read`/`write`) gilt weiter.
pub async fn authorize_issue(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    issue: IssueId,
    team: TeamId,
    action: Action,
) -> AppResult<()> {
    let err = match snap.require_team(ctx, action, team) {
        Ok(_) => return Ok(()),
        Err(e) => e,
    };
    let shareable = matches!(
        action,
        Action::ViewTeam | Action::CommentIssue | Action::ReactIssue
    );
    if err.kind != crate::error::ErrorKind::NotFound || !shareable {
        return Err(err);
    }
    if !share_grants_access(tx, snap, ctx, issue).await? {
        return Err(err);
    }
    if !ctx.scopes.read || (action.is_write() && !ctx.scopes.write) {
        return Err(
            AppError::forbidden("Token scope does not allow this operation.")
                .with_details(json!({ "reason": "insufficient_scope" })),
        );
    }
    Ok(())
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
