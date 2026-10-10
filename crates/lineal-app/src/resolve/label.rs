//! resolve – Labels: UUID → exakter Name (case-insensitiv) → `Gruppe/Label` (docs/07 §2.1).
//!
//! Kandidaten werden vor der Mehrdeutigkeitsprüfung auf den Geltungsbereich begrenzt
//! (Workspace-Labels, Labels des Teams und seiner Vorfahren bzw. aller sichtbaren Teams).

use crate::error::{AppError, AppResult};
use crate::policy::TeamTree;
use crate::ports::Tx;
use crate::ports::repos::label::{Label, LabelKind};
use lineal_domain::ids::{LabelId, TeamId};
use serde_json::json;
use std::collections::BTreeSet;
use uuid::Uuid;

/// Fehler für nicht auflösbare Labels.
pub fn label_not_found(kind: LabelKind, reference: &str) -> AppError {
    AppError::not_found(format!("Could not find referenced {} label.", kind.noun()))
        .with_details(json!({ "reference": reference, "type": "IssueLabel" }))
}

/// Alle Labels, auf die die Referenz passt (ohne Bereichsfilter).
pub async fn candidates(
    tx: &mut dyn Tx,
    kind: LabelKind,
    reference: &str,
) -> AppResult<Vec<Label>> {
    let reference = reference.trim();
    if let Ok(uuid) = Uuid::parse_str(reference) {
        return Ok(tx
            .labels()
            .get(kind, LabelId(uuid))
            .await?
            .into_iter()
            .collect());
    }
    let mut found = tx.labels().find_by_name(kind, reference).await?;
    if found.is_empty()
        && let Some((group, name)) = reference.split_once('/')
    {
        let mut out = Vec::new();
        for l in tx.labels().find_by_name(kind, name.trim()).await? {
            if let Some(parent) = l.parent_id
                && let Some(g) = tx.labels().get(kind, parent).await?
                && g.name.eq_ignore_ascii_case(group.trim())
            {
                out.push(l);
            }
        }
        found = out;
    }
    Ok(found)
}

fn ambiguous(reference: &str, found: &[Label]) -> AppError {
    AppError::ambiguous_reference(format!("Label \"{reference}\" matches several labels."))
        .with_details(json!({
            "reference": reference,
            "type": "IssueLabel",
            "candidates": found.iter().take(10).map(|l| json!({
                "id": l.id.to_string(), "name": l.name,
                "team": l.team_id.map(|t| t.to_string()),
            })).collect::<Vec<_>>(),
        }))
}

/// Genau ein Treffer, sonst `not_found` bzw. `ambiguous_reference`.
pub fn exactly_one(kind: LabelKind, reference: &str, mut found: Vec<Label>) -> AppResult<Label> {
    match found.len() {
        0 => Err(label_not_found(kind, reference)),
        1 => Ok(found.remove(0)),
        _ => Err(ambiguous(reference, &found)),
    }
}

/// Teams, deren Team-Labels in `team` verfügbar sind: das Team selbst und alle Vorfahren.
pub fn available_scopes(tree: &TeamTree, team: TeamId) -> BTreeSet<TeamId> {
    std::iter::once(team)
        .chain(tree.ancestors(team).iter().map(|t| t.id))
        .collect()
}

/// Löst ein Issue-Label für ein Issue des Teams `team` auf (Workspace-Label oder Label des Teams
/// bzw. eines Vorfahren, nicht archiviert).
pub async fn resolve_for_team(
    tx: &mut dyn Tx,
    tree: &TeamTree,
    team: TeamId,
    reference: &str,
) -> AppResult<Label> {
    let scopes = available_scopes(tree, team);
    let found: Vec<Label> = candidates(tx, LabelKind::Issue, reference)
        .await?
        .into_iter()
        .filter(|l| l.archived_at.is_none() && l.team_id.is_none_or(|t| scopes.contains(&t)))
        .collect();
    exactly_one(LabelKind::Issue, reference, found)
}

/// Kandidaten innerhalb der sichtbaren Teams (Workspace-Labels immer); `team_hint` begrenzt auf
/// Labels dieses Teams (Disambiguierung per `teamId`).
pub async fn resolve_visible(
    tx: &mut dyn Tx,
    kind: LabelKind,
    visible: &BTreeSet<TeamId>,
    reference: &str,
    team_hint: Option<TeamId>,
) -> AppResult<Vec<Label>> {
    Ok(candidates(tx, kind, reference)
        .await?
        .into_iter()
        .filter(|l| l.team_id.is_none_or(|t| visible.contains(&t)))
        .filter(|l| team_hint.is_none_or(|h| l.team_id == Some(h)))
        .collect())
}
