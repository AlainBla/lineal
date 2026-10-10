//! resolve – Projekte: UUID → Kennung (`P-ENG-7`, auch frühere) → Slug/`slugId` → exakter Name.
//!
//! Ein Projekt ist sichtbar, wenn mindestens eines seiner Teams zugänglich ist (docs/02 §5.3);
//! sonst `not_found`. Kandidaten werden **vor** der Mehrdeutigkeitsprüfung gefiltert.

use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::project::{Milestone, Project};
use lineal_domain::ids::{MilestoneId, ProjectId, TeamId};
use serde_json::json;
use uuid::Uuid;

/// Ein aufgelöstes Projekt mit allen beteiligten Teams (auch unsichtbaren; für Rechte).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProject {
    pub project: Project,
    pub teams: Vec<TeamId>,
}

fn not_found(reference: &str) -> AppError {
    AppError::not_found(format!("Could not find project \"{reference}\""))
        .with_details(json!({ "reference": reference, "type": "Project" }))
}

/// `ENG-123`-förmige Kennung (Issue) – gehört nicht an ein Projekt-Feld.
fn looks_like_issue_identifier(r: &str) -> bool {
    let Some((key, n)) = r.rsplit_once('-') else {
        return false;
    };
    !key.is_empty()
        && key.len() <= 7
        && key.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && key.chars().all(|c| c.is_ascii_alphanumeric())
        && !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
}

fn parse_project_identifier(r: &str) -> Option<(String, i32)> {
    let rest = r.strip_prefix("P-").or_else(|| r.strip_prefix("p-"))?;
    let (key, n) = rest.rsplit_once('-')?;
    Some((key.to_uppercase(), n.parse().ok()?))
}

fn is_slug_id(s: &str) -> bool {
    s.len() == 12
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Teams des Projekts laden; `Some`, wenn mindestens eines davon sichtbar ist.
async fn visible_teams_of(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    p: &Project,
) -> AppResult<Option<Vec<TeamId>>> {
    let teams = tx.projects().list_teams(p.id).await?;
    let visible = snap.visible_team_ids();
    Ok(teams.iter().any(|t| visible.contains(t)).then_some(teams))
}

/// Löst eine Projekt-Referenz mit einem bereits geladenen Snapshot auf.
pub async fn resolve_in(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    reference: &str,
) -> AppResult<ResolvedProject> {
    let r = reference.trim();
    let mut found: Option<Project> = None;
    // 1. UUID
    if let Ok(uuid) = Uuid::parse_str(r) {
        found = tx
            .projects()
            .get(ProjectId(uuid))
            .await?
            .filter(|p| !p.trashed);
    } else if let Some((key, n)) = parse_project_identifier(r) {
        // 2. Kennung, auch frühere
        found = tx.projects().find_by_key_and_number(&key, n).await?;
        if found.is_none() {
            found = tx.projects().find_by_alias(&r.to_uppercase()).await?;
        }
    } else {
        // 3. Slug: `slugId` oder `<name-slug>-<slugId>`
        let slug = r.rsplit('-').next().unwrap_or(r);
        if is_slug_id(r) {
            found = tx.projects().find_by_slug_id(r).await?;
        } else if r.contains('-') && is_slug_id(slug) {
            found = tx.projects().find_by_slug_id(slug).await?;
        }
    }
    if let Some(p) = found {
        return match visible_teams_of(tx, snap, &p).await? {
            Some(teams) => Ok(ResolvedProject { project: p, teams }),
            None => Err(not_found(reference)),
        };
    }
    // 5. exakter Name (case-insensitiv), Kandidaten vor der Prüfung auf Sichtbarkeit filtern
    let mut candidates = Vec::new();
    for p in tx.projects().find_by_name(r).await? {
        if let Some(teams) = visible_teams_of(tx, snap, &p).await? {
            candidates.push(ResolvedProject { project: p, teams });
        }
    }
    match candidates.len() {
        0 => {
            if looks_like_issue_identifier(r) && parse_project_identifier(r).is_none() {
                return Err(AppError::invalid_request(format!(
                    "\"{reference}\" is an issue identifier; use the issue tools (get_issue) instead."
                ))
                .with_details(json!({ "reference": reference, "type": "Project", "hint": "get_issue" })));
            }
            Err(not_found(reference))
        }
        1 => Ok(candidates.remove(0)),
        _ => {
            let mut cands = Vec::new();
            for c in candidates.iter().take(10) {
                let key = tx
                    .teams()
                    .get(c.project.lead_team_id)
                    .await?
                    .map(|t| t.key)
                    .unwrap_or_default();
                cands.push(json!({
                    "id": c.project.id.to_string(), "name": c.project.name,
                    "key": format!("P-{}-{}", key, c.project.number),
                }));
            }
            Err(AppError::ambiguous_reference(format!(
                "Project name \"{reference}\" matches several projects."
            ))
            .with_details(
                json!({ "reference": reference, "type": "Project", "candidates": cands }),
            ))
        }
    }
}

/// Löst eine Projekt-Referenz auf (lädt den Snapshot selbst).
pub async fn resolve(
    tx: &mut dyn Tx,
    ctx: &crate::context::RequestCtx,
    reference: &str,
) -> AppResult<ResolvedProject> {
    let snap = PolicySnapshot::load(tx, ctx).await?;
    resolve_in(tx, &snap, reference).await
}

/// Löst einen Meilenstein innerhalb eines Projekts auf: UUID oder Name (case-insensitiv).
pub async fn resolve_milestone(
    tx: &mut dyn Tx,
    project: ProjectId,
    reference: &str,
) -> AppResult<Milestone> {
    let r = reference.trim();
    let nf = || {
        AppError::not_found(format!("Could not find milestone \"{reference}\""))
            .with_details(json!({ "reference": reference, "type": "ProjectMilestone" }))
    };
    if let Ok(uuid) = Uuid::parse_str(r)
        && let Some(m) = tx.projects().get_milestone(MilestoneId(uuid)).await?
        && m.project_id == project
        && m.archived_at.is_none()
    {
        return Ok(m);
    }
    tx.projects()
        .find_milestone_by_name(project, r)
        .await?
        .ok_or_else(nf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_shapes() {
        assert_eq!(parse_project_identifier("P-ENG-7"), Some(("ENG".into(), 7)));
        assert_eq!(
            parse_project_identifier("p-web-12"),
            Some(("WEB".into(), 12))
        );
        assert_eq!(parse_project_identifier("ENG-7"), None);
        assert!(looks_like_issue_identifier("ENG-123"));
        assert!(!looks_like_issue_identifier("Checkout Relaunch"));
        assert!(is_slug_id("1a2b3c4d5e6f"));
        assert!(!is_slug_id("1A2B3C4D5E6F"));
    }
}
