//! resolve – User: UUID → `"me"` → E-Mail → exakter Name/Anzeigename.
//!
//! Gäste sehen nur User, mit denen sie ein Team teilen (docs/02 §4.4); sonst `not_found`.

use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::user::User;
use lineal_domain::enums::UserRole;
use lineal_domain::ids::UserId;
use serde_json::json;
use uuid::Uuid;

fn not_found(reference: &str) -> AppError {
    AppError::not_found(format!("Could not find user \"{reference}\""))
        .with_details(json!({ "reference": reference, "type": "User" }))
}

/// Die ID des Aufrufers (`"me"`); `None` für Systemkontexte.
pub fn actor_user_id(ctx: &RequestCtx) -> Option<UserId> {
    match ctx.actor {
        Actor::User { id, .. } | Actor::App { id } => Some(id),
        Actor::System => None,
    }
}

/// Ob der Akteur den User sehen darf (Gäste: nur gemeinsame Teams oder sich selbst).
pub async fn can_see_user(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    user: &User,
) -> AppResult<bool> {
    if snap.actor.role != UserRole::Guest {
        return Ok(true);
    }
    if actor_user_id(ctx) == Some(user.id) {
        return Ok(true);
    }
    let theirs = tx.teams().list_memberships_of_user(user.id).await?;
    Ok(theirs.iter().any(|m| snap.actor.is_member(m.team_id)))
}

/// Löst eine User-Referenz mit einem bereits geladenen Snapshot auf.
pub async fn resolve_in(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    reference: &str,
) -> AppResult<User> {
    let reference = reference.trim();
    // 6. Literal "me"
    if reference.eq_ignore_ascii_case("me") {
        let id = actor_user_id(ctx).ok_or_else(|| not_found(reference))?;
        return tx.users().get(id).await?.ok_or_else(|| not_found(reference));
    }
    let mut found: Option<User> = None;
    // 1. UUID
    if let Ok(uuid) = Uuid::parse_str(reference) {
        found = tx.users().get(UserId(uuid)).await?;
    }
    // 6. E-Mail
    if found.is_none() && reference.contains('@') {
        found = tx.users().find_by_email(reference).await?;
    }
    if let Some(u) = found {
        if can_see_user(tx, snap, ctx, &u).await? {
            return Ok(u);
        }
        return Err(not_found(reference));
    }
    // 5. exakter Name oder Anzeigename (Kandidaten vor der Mehrdeutigkeitsprüfung filtern)
    let mut candidates = Vec::new();
    for u in tx.users().find_by_name(reference).await? {
        if can_see_user(tx, snap, ctx, &u).await? {
            candidates.push(u);
        }
    }
    match candidates.len() {
        0 => Err(not_found(reference)),
        1 => Ok(candidates.remove(0)),
        _ => Err(AppError::ambiguous_reference(format!(
            "User name \"{reference}\" matches several users."
        ))
        .with_details(json!({
            "reference": reference,
            "type": "User",
            "candidates": candidates.iter().take(10).map(|u| json!({
                "id": u.id.to_string(), "name": u.name
            })).collect::<Vec<_>>(),
        }))),
    }
}

/// Löst eine User-Referenz auf (lädt den Snapshot selbst).
pub async fn resolve(tx: &mut dyn Tx, ctx: &RequestCtx, reference: &str) -> AppResult<User> {
    let snap = PolicySnapshot::load(tx, ctx).await?;
    resolve_in(tx, &snap, ctx, reference).await
}
