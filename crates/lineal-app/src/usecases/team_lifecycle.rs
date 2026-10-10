//! usecases – Team-Lebenszyklus (WP16, docs/02 §2.7): stilllegen, wiederherstellen, löschen
//! (30 Tage Papierkorb), gelöschtes Team wiederherstellen, endgültiges Entfernen (Purge-Job).
//!
//! Key-Änderung mit Historie und „Copy settings“ liegen in `team.rs` (`update_team`/`create_team`).
//!
//! Umgesetzt: `handleSubTeamsOnRetirement`/`handleSubTeams`, Dokument-Kaskade
//! (`DocumentRepo::cascade_trash/cascade_restore`) für Team- und Projekt-Dokumente, Projekte nur
//! dieses Teams wandern in den Papierkorb (und kommen zurück), Multi-Team-Projekte verlieren das
//! Team (neues Lead-Team: das mit den meisten Issues im Projekt, bei Gleichstand das älteste).
//! Offen: `openIssuesOnRetirement` (`cancel`/`moveToTeam`) – nur `keep`.

use super::team::{NestOptions, TeamView, emit};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot, TeamTree, team_not_found};
use crate::ports::Tx;
use crate::ports::repos::document::DocParent;
use crate::ports::repos::project::ProjectPatch;
use crate::ports::repos::team::Team;
use crate::resolve;
use crate::resolve::team::Visibility;
use chrono::{DateTime, Duration, Utc};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{ProjectId, TeamId};
use serde_json::json;
use uuid::Uuid;

/// Umgang mit aktiven Sub-Teams beim Stilllegen bzw. Löschen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubTeamHandling {
    /// Sub-Teams mit stilllegen bzw. mit löschen.
    Cascade,
    /// Sub-Teams werden Root-Teams (docs/02 §3.5).
    Unnest,
}

impl SubTeamHandling {
    /// `retire`/`delete` (Kaskade) oder `unnest`.
    pub fn parse(s: &str) -> AppResult<Self> {
        match s {
            "retire" | "delete" => Ok(Self::Cascade),
            "unnest" => Ok(Self::Unnest),
            other => Err(
                AppError::invalid_request(format!("unknown sub-team handling `{other}`"))
                    .with_details(json!({ "field": "handleSubTeams" })),
            ),
        }
    }
}

/// Wie offene Issues beim Stilllegen behandelt werden (docs/02 §2.7, Festlegung Lineal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenIssuesOnRetirement {
    #[default]
    Keep,
}

/// Ergebnis des Purge-Laufs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamPurgeStats {
    pub processed: usize,
    pub errors: Vec<String>,
}

fn view(tree: &TeamTree, id: TeamId) -> AppResult<TeamView> {
    let team = tree.get(id).cloned().ok_or_else(team_not_found)?;
    Ok(TeamView {
        display_name: tree.display_name(id),
        team,
    })
}

fn active_children(tree: &TeamTree, id: TeamId) -> Vec<Team> {
    tree.children(id)
        .into_iter()
        .filter(|c| !c.trashed && c.retired_at.is_none())
        .cloned()
        .collect()
}

fn needs_handling() -> AppError {
    AppError::invalid_request("The team has active sub-teams; say how to handle them").with_details(
        json!({ "field": "handleSubTeams", "values": ["retire", "delete", "unnest"] }),
    )
}

impl App {
    /// Alle aktiven Nachfahren (nicht stillgelegt/gelöscht), tiefste zuletzt.
    fn active_descendants(tree: &TeamTree, id: TeamId) -> Vec<Team> {
        let mut out = Vec::new();
        let mut queue = vec![id];
        while let Some(cur) = queue.pop() {
            for c in active_children(tree, cur) {
                queue.push(c.id);
                out.push(c);
            }
        }
        out
    }

    /// Legt das Team still: read-only, aus Sidebar und Pickern verschwunden (docs/02 §2.7).
    /// Aktive Sub-Teams verlangen `handle_sub_teams` (`Cascade` = mit stilllegen, `Unnest`).
    pub async fn retire_team(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        handle_sub_teams: Option<SubTeamHandling>,
        _open_issues: OpenIssuesOnRetirement,
    ) -> AppResult<TeamView> {
        let (team_id, children) = {
            let mut tx = self.store.begin(ctx.org_id).await?;
            let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
            let team =
                resolve::team::resolve_in(&mut *tx, &snap, reference, Visibility::Management)
                    .await?;
            snap.require_team(ctx, Action::RetireTeam, team.id)?;
            if team.trashed {
                return Err(team_not_found());
            }
            if team.retired_at.is_some() {
                return Err(AppError::invalid_request("The team is already retired"));
            }
            (team.id, active_children(&snap.tree, team.id))
        };
        if !children.is_empty() {
            match handle_sub_teams {
                None => return Err(needs_handling()),
                Some(SubTeamHandling::Unnest) => {
                    for c in &children {
                        self.nest_team(ctx, &c.id.to_string(), None, NestOptions::default())
                            .await?;
                    }
                }
                Some(SubTeamHandling::Cascade) => {}
            }
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_team(ctx, Action::RetireTeam, team_id)?;
        let now = self.clock.now();
        let mut targets = vec![team_id];
        targets.extend(
            Self::active_descendants(&snap.tree, team_id)
                .iter()
                .map(|t| t.id),
        );
        for t in targets {
            snap.require_team(ctx, Action::RetireTeam, t)?;
            tx.webhooks().set_team_retired(t, Some(now), now).await?;
            emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: t }).await?;
        }
        let tree = TeamTree::new(tx.teams().list_all().await?);
        let v = view(&tree, team_id)?;
        tx.commit().await?;
        Ok(v)
    }

    /// Hebt das Stilllegen auf. Bei einem Sub-Team muss der Parent aktiv sein.
    pub async fn restore_team(&self, ctx: &RequestCtx, reference: &str) -> AppResult<TeamView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, reference, Visibility::Management).await?;
        snap.require_team(ctx, Action::RestoreTeam, team.id)?;
        if team.trashed {
            return Err(team_not_found());
        }
        if team.retired_at.is_none() {
            return Err(AppError::invalid_request("The team is not retired"));
        }
        if let Some(p) = team.parent_id.and_then(|p| snap.tree.get(p))
            && (p.retired_at.is_some() || p.trashed)
        {
            return Err(AppError::invalid_request(
                "The parent team is retired or deleted; restore it first",
            )
            .with_details(json!({ "field": "parent" })));
        }
        let now = self.clock.now();
        tx.webhooks().set_team_retired(team.id, None, now).await?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        let tree = TeamTree::new(tx.teams().list_all().await?);
        let v = view(&tree, team.id)?;
        tx.commit().await?;
        Ok(v)
    }

    /// Löscht das Team (Papierkorb, `trashRetentionDays` Frist). Für Nicht-Admins sofort unsichtbar.
    pub async fn delete_team(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        handle_sub_teams: Option<SubTeamHandling>,
    ) -> AppResult<TeamView> {
        let (team_id, children) = {
            let mut tx = self.store.begin(ctx.org_id).await?;
            let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
            let team =
                resolve::team::resolve_in(&mut *tx, &snap, reference, Visibility::Management)
                    .await?;
            snap.require_team(ctx, Action::DeleteTeam, team.id)?;
            if team.trashed {
                return Err(team_not_found());
            }
            (team.id, active_children(&snap.tree, team.id))
        };
        if !children.is_empty() {
            match handle_sub_teams {
                None => return Err(needs_handling()),
                Some(SubTeamHandling::Unnest) => {
                    for c in &children {
                        self.nest_team(ctx, &c.id.to_string(), None, NestOptions::default())
                            .await?;
                    }
                }
                Some(SubTeamHandling::Cascade) => {}
            }
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_team(ctx, Action::DeleteTeam, team_id)?;
        let now = self.clock.now();
        // Das Team und (bei Kaskade) seine noch vorhandenen Nachfahren, Blätter zuerst.
        let mut targets = vec![team_id];
        let mut rest = snap.tree.descendants(team_id);
        rest.retain(|t| !t.trashed);
        for t in &rest {
            snap.require_team(ctx, Action::DeleteTeam, t.id)?;
        }
        targets.extend(rest.iter().rev().map(|t| t.id));
        targets.reverse();
        for t in targets {
            self.trash_team_in(&mut *tx, ctx, &snap.tree, t, now)
                .await?;
        }
        let tree = TeamTree::new(tx.teams().list_all().await?);
        let v = view(&tree, team_id)?;
        tx.commit().await?;
        Ok(v)
    }

    async fn trash_team_in(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        tree: &TeamTree,
        team: TeamId,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        tx.webhooks().set_team_trashed(team, true, now).await?;
        tx.documents()
            .cascade_trash(DocParent::Team(team), now)
            .await?;
        // Projekte nur dieses Teams wandern in den Papierkorb.
        for pid in tx.webhooks().trash_sole_projects(team, now).await? {
            let id = ProjectId(pid);
            tx.documents()
                .cascade_trash(DocParent::Project(id), now)
                .await?;
            emit(tx, ctx, DomainEvent::ProjectUpdated { project_id: id }).await?;
        }
        // Multi-Team-Projekte: Team entfernen, bei Bedarf neues Lead-Team.
        for (pid, lead) in tx.webhooks().shared_team_projects(team).await? {
            let id = ProjectId(pid);
            if lead == team {
                let counts = tx.webhooks().project_issue_counts(pid).await?;
                let others: Vec<TeamId> = tx
                    .projects()
                    .list_teams(id)
                    .await?
                    .into_iter()
                    .filter(|t| *t != team)
                    .collect();
                // Meiste Issues, bei Gleichstand das älteste Team (Reihenfolge der Anlage).
                let order = |t: &TeamId| tree.all().position(|x| x.id == *t).unwrap_or(usize::MAX);
                let count = |t: &TeamId| counts.iter().find(|(c, _)| c == t).map_or(0, |(_, n)| *n);
                if let Some(new_lead) = others
                    .iter()
                    .copied()
                    .min_by_key(|t| (std::cmp::Reverse(count(t)), order(t)))
                {
                    if let Some(project) = tx.projects().get(id).await? {
                        let old_key = tree
                            .get(project.lead_team_id)
                            .map(|t| t.key.clone())
                            .unwrap_or_default();
                        let number = tx.teams().next_project_number(new_lead).await?;
                        tx.projects()
                            .add_alias(id, &format!("P-{}-{}", old_key, project.number), &old_key)
                            .await?;
                        tx.projects()
                            .update(
                                id,
                                &ProjectPatch {
                                    lead_team_id: Some(new_lead),
                                    number: Some(number),
                                    ..Default::default()
                                },
                                now,
                            )
                            .await?;
                    }
                }
            }
            let actor = crate::resolve::user::actor_user_id(ctx);
            tx.projects()
                .detach_team_issues(id, team, actor, now)
                .await?;
            tx.projects().remove_team(id, team).await?;
            emit(tx, ctx, DomainEvent::ProjectUpdated { project_id: id }).await?;
        }
        emit(tx, ctx, DomainEvent::TeamUpdated { team_id: team }).await?;
        Ok(())
    }

    /// Stellt ein gelöschtes Team wieder her (Admin/Owner, innerhalb von `trashRetentionDays`).
    /// Mit dem Team gelöschte Nachfahren, Projekte und Dokumente kommen zurück; aus Multi-Team-
    /// Projekten entfernte Zuordnungen nicht.
    pub async fn restore_deleted_team(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<TeamView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        // Gelöschte Teams lösen die normalen Resolver nicht auf: UUID oder Key.
        let reference = reference.trim();
        let found = match Uuid::parse_str(reference) {
            Ok(u) => snap.tree.get(TeamId(u)).cloned(),
            Err(_) => {
                let key = reference.to_uppercase();
                snap.tree.all().find(|t| t.trashed && t.key == key).cloned()
            }
        };
        let team = found.filter(|t| t.trashed).ok_or_else(team_not_found)?;
        snap.require_team(ctx, Action::RestoreDeletedTeam, team.id)?;
        let now = self.clock.now();
        let trashed_at = team.trashed_at.ok_or_else(team_not_found)?;
        if trashed_at + Duration::days(snap.org.limits.trash_retention_days) <= now {
            return Err(team_not_found());
        }
        if let Some(p) = team.parent_id.and_then(|p| snap.tree.get(p))
            && (p.trashed || p.retired_at.is_some())
        {
            return Err(AppError::invalid_request(
                "The parent team is deleted or retired; restore it first",
            )
            .with_details(json!({ "field": "parent" })));
        }
        if let Some(max) = snap.org.limits.max_teams {
            let active = snap.tree.all().filter(|t| !t.trashed).count() as i64;
            if active >= max {
                return Err(
                    AppError::limit_exceeded("The maximum number of teams is reached.")
                        .with_details(json!({ "limit": "maxTeams" })),
                );
            }
        }
        // Das Team und die zugleich gelöschten Nachfahren (Eltern vor Kindern).
        let mut targets = vec![team.clone()];
        let mut queue = vec![team.id];
        while let Some(cur) = queue.pop() {
            for c in snap.tree.children(cur) {
                if c.trashed && c.trashed_at == Some(trashed_at) {
                    targets.push(c.clone());
                    queue.push(c.id);
                }
            }
        }
        for t in targets {
            tx.webhooks().set_team_trashed(t.id, false, now).await?;
            tx.documents()
                .cascade_restore(DocParent::Team(t.id))
                .await?;
            if let Some(at) = t.trashed_at {
                for pid in tx.webhooks().restore_sole_projects(t.id, at, now).await? {
                    let id = ProjectId(pid);
                    tx.documents()
                        .cascade_restore(DocParent::Project(id))
                        .await?;
                    emit(
                        &mut *tx,
                        ctx,
                        DomainEvent::ProjectUpdated { project_id: id },
                    )
                    .await?;
                }
            }
            emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: t.id }).await?;
        }
        let tree = TeamTree::new(tx.teams().list_all().await?);
        let v = view(&tree, team.id)?;
        tx.commit().await?;
        Ok(v)
    }

    /// Periodischer Job: löscht Teams endgültig, deren Papierkorb-Frist abgelaufen ist
    /// (`trashRetentionDays` des Workspace); die Keys werden erst dadurch frei.
    pub async fn run_team_purge(&self, now: DateTime<Utc>) -> AppResult<TeamPurgeStats> {
        let mut sys = self.store.begin_system().await?;
        // Untergrenze 1 Tag; die Frist je Workspace prüft der Aufrufer unten.
        let candidates = sys
            .webhooks()
            .list_teams_due_for_purge(now - Duration::days(1))
            .await?;
        drop(sys);
        let mut stats = TeamPurgeStats::default();
        for (org, team) in candidates {
            match self.purge_team_once(org, team, now).await {
                Ok(true) => stats.processed += 1,
                Ok(false) => {}
                Err(e) => stats.errors.push(format!("team {team}: {e}")),
            }
        }
        Ok(stats)
    }

    async fn purge_team_once(
        &self,
        org: lineal_domain::ids::OrgId,
        team: TeamId,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut tx = self.store.begin(org).await?;
        let Some(t) = tx.teams().get(team).await? else {
            return Ok(false);
        };
        let retention = tx.orgs().get().await?.map_or(30, |o| {
            crate::policy::OrgPolicy::from_json(&o.security_settings, &o.limits)
                .limits
                .trash_retention_days
        });
        let Some(trashed_at) = t.trashed_at.filter(|_| t.trashed) else {
            return Ok(false);
        };
        if trashed_at + Duration::days(retention) > now {
            return Ok(false);
        }
        // Sub-Teams, die noch am Team hängen, würden den Parent-FK verletzen: erst lösen
        // (gelöschte Sub-Teams folgen im selben Lauf).
        let keys = tx.attachments().storage_keys_for_team(team).await?;
        tx.webhooks().purge_team(team).await?;
        let orphans = tx.attachments().unreferenced_storage_keys(&keys).await?;
        let ctx = RequestCtx::system(org, "req_team_purge");
        emit(&mut *tx, &ctx, DomainEvent::TeamRemoved { team_id: team }).await?;
        tx.commit().await?;
        self.delete_blobs(&orphans).await;
        Ok(true)
    }
}
