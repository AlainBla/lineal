//! Relationen und „Als Duplikat markieren“ (docs/03 §4).

use super::fields::{change_state, first_state_of};
use super::history::{self, Delta};
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::issue::{HistorySource, Issue, Relation};
use crate::ports::repos::notification::{SubscribeOutcome, SubscriptionReason};
use crate::ports::repos::team::Team;
use crate::resolve;
use crate::resolve::user::actor_user_id;
use chrono::{DateTime, Utc};
use lineal_domain::enums::{RelationType, StateType};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{IssueId, IssueRelationId};
use lineal_domain::rules::state::TransitionSource;
use serde_json::{Value, json};
use uuid::Uuid;

/// Relationsänderungen eines Aufrufs (Append-only und explizites Entfernen, docs/07 §2.5).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelationChanges {
    pub blocks: Vec<String>,
    pub blocked_by: Vec<String>,
    pub related_to: Vec<String>,
    pub remove_blocks: Vec<String>,
    pub remove_blocked_by: Vec<String>,
    pub remove_related_to: Vec<String>,
    /// `Some(Some(ref))` setzt, `Some(None)` entfernt das Duplikat-Verhältnis.
    pub duplicate_of: Option<Option<String>>,
}

impl RelationChanges {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
            && self.blocked_by.is_empty()
            && self.related_to.is_empty()
            && self.remove_blocks.is_empty()
            && self.remove_blocked_by.is_empty()
            && self.remove_related_to.is_empty()
            && self.duplicate_of.is_none()
    }
}

/// Alles, was die Relationslogik aus dem Aufruf braucht.
pub(super) struct Env<'a> {
    pub ctx: &'a RequestCtx,
    pub snap: &'a PolicySnapshot,
    pub ids: &'a dyn crate::ports::IdGen,
    pub now: DateTime<Utc>,
    pub source: HistorySource,
}

fn rel(kind: &str, identifier: &str, action: &str) -> Value {
    json!({ "type": kind, "identifier": identifier, "action": action })
}

async fn target(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    issue: &Issue,
    reference: &str,
) -> AppResult<Issue> {
    let other = resolve::issue::resolve_in(tx, env.snap, env.ctx, reference).await?;
    if other.trashed {
        return Err(resolve::issue::issue_not_found(reference));
    }
    // Die Relation verändert auch das Gegenstück: Schreibrecht auf dessen Team nötig.
    env.snap
        .require_team(env.ctx, crate::policy::Action::UpdateIssue, other.team_id)?;
    if other.id == issue.id {
        return Err(
            AppError::invalid_request("an issue cannot relate to itself")
                .with_details(json!({ "field": "relation", "reason": "self_relation" })),
        );
    }
    Ok(other)
}

async fn write_other_history(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    other: &Issue,
    change: Value,
) -> AppResult<()> {
    let delta = Delta {
        relation_changes: vec![change],
        ..Default::default()
    };
    history::record(
        tx,
        env.ids.new_uuid(),
        other,
        actor_user_id(env.ctx),
        env.source,
        &delta,
        env.now,
    )
    .await
}

/// Legt eine Relation an (idempotent). `own_type`/`other_type` sind die Sichten aus Sicht von
/// `issue` bzw. `other` für die History.
#[allow(clippy::too_many_arguments)]
async fn add(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    issue: &Issue,
    other: &Issue,
    from: IssueId,
    kind: RelationType,
    own_type: &str,
    other_type: &str,
    own: &mut Delta,
) -> AppResult<()> {
    let to = if from == issue.id { other.id } else { issue.id };
    if kind == RelationType::Blocks {
        let existing = tx.issues().relations_of(issue.id).await?;
        if existing.iter().any(|r| {
            r.relation_type == RelationType::Blocks
                && r.issue_id == to
                && r.related_issue_id == from
        }) {
            return Err(AppError::invalid_request(
                "the issues already block each other in the opposite direction",
            )
            .with_details(json!({ "field": "blocks", "reason": "blocks_both_ways" })));
        }
    }
    let id = IssueRelationId(env.ids.new_uuid());
    let created = tx
        .issues()
        .insert_relation(id, from, to, kind, actor_user_id(env.ctx), env.now)
        .await?;
    if created {
        own.relation_changes
            .push(rel(own_type, &other.identifier, "added"));
        write_other_history(tx, env, other, rel(other_type, &issue.identifier, "added")).await?;
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::IssueRelationCreated {
                    relation_id: id,
                    issue_id: issue.id,
                },
                env.ctx,
            ))
            .await?;
    }
    Ok(())
}

/// Legt bei einer Erwähnung (`ENG-123` im Text) eine `related`-Relation an, sofern das
/// Gegenstück sichtbar und beschreibbar ist und noch keine Relation zwischen beiden besteht
/// (docs/03 §4.2). Unsichtbare oder unbekannte Kennungen werden stillschweigend übergangen.
pub(super) async fn link_mention(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    issue: &Issue,
    reference: &str,
) -> AppResult<bool> {
    let other = match resolve::issue::resolve_in(tx, env.snap, env.ctx, reference).await {
        Ok(o) => o,
        Err(e) if e.kind == crate::error::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    if other.id == issue.id
        || other.trashed
        || env
            .snap
            .require_team(env.ctx, crate::policy::Action::UpdateIssue, other.team_id)
            .is_err()
    {
        return Ok(false);
    }
    let existing = tx.issues().relations_of(issue.id).await?;
    if existing
        .iter()
        .any(|r| r.issue_id == other.id || r.related_issue_id == other.id)
    {
        return Ok(false);
    }
    let mut own = Delta::default();
    add(
        tx,
        env,
        issue,
        &other,
        issue.id,
        RelationType::Related,
        "relatedTo",
        "relatedTo",
        &mut own,
    )
    .await?;
    if !own.is_empty() {
        history::record(
            tx,
            env.ids.new_uuid(),
            issue,
            actor_user_id(env.ctx),
            env.source,
            &own,
            env.now,
        )
        .await?;
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn remove(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    issue: &Issue,
    other: &Issue,
    from: IssueId,
    kind: RelationType,
    own_type: &str,
    other_type: &str,
    own: &mut Delta,
) -> AppResult<()> {
    let to = if from == issue.id { other.id } else { issue.id };
    if let Some(r) = tx.issues().delete_relation(from, to, kind).await? {
        own.relation_changes
            .push(rel(own_type, &other.identifier, "removed"));
        write_other_history(
            tx,
            env,
            other,
            rel(other_type, &issue.identifier, "removed"),
        )
        .await?;
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::IssueRelationRemoved {
                    relation_id: r.id,
                    issue_id: issue.id,
                },
                env.ctx,
            ))
            .await?;
    }
    Ok(())
}

/// Folgt der Duplikat-Kette bis zum kanonischen Ende (docs/03 §4.3).
async fn canonical_end(tx: &mut dyn Tx, start: Issue, dup: IssueId) -> AppResult<Issue> {
    let mut cur = start;
    for _ in 0..16 {
        let rels = tx.issues().relations_of(cur.id).await?;
        let next = rels
            .iter()
            .find(|r| r.relation_type == RelationType::Duplicate && r.issue_id == cur.id)
            .map(|r| r.related_issue_id);
        match next {
            Some(n) if n == dup => {
                return Err(
                    AppError::invalid_request("duplicate chain leads back to the issue")
                        .with_details(
                            json!({ "field": "duplicateOf", "reason": "duplicate_cycle" }),
                        ),
                );
            }
            Some(n) => {
                cur = tx
                    .issues()
                    .get(n, false)
                    .await?
                    .ok_or_else(|| AppError::internal("duplicate target vanished"))?;
            }
            None => return Ok(cur),
        }
    }
    Err(AppError::invalid_request("duplicate chain is too long")
        .with_details(json!({ "field": "duplicateOf", "reason": "duplicate_chain_too_long" })))
}

/// Markiert `issue` als Duplikat von `canonical` (immer vom Duplikat aus) und setzt den
/// system-verwalteten `duplicate`-Status.
async fn mark_duplicate(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    team: &Team,
    issue: &mut Issue,
    reference: &str,
    own: &mut Delta,
) -> AppResult<()> {
    let first = target(tx, env, issue, reference).await?;
    let canonical = canonical_end(tx, first, issue.id).await?;
    if canonical.id == issue.id {
        return Err(
            AppError::invalid_request("an issue cannot be a duplicate of itself")
                .with_details(json!({ "field": "duplicateOf", "reason": "self_relation" })),
        );
    }
    // Bisheriges duplicateOf ersetzen.
    let existing = tx.issues().relations_of(issue.id).await?;
    for r in existing
        .iter()
        .filter(|r| r.relation_type == RelationType::Duplicate && r.issue_id == issue.id)
    {
        if r.related_issue_id == canonical.id {
            continue;
        }
        if let Some(old) = tx.issues().get(r.related_issue_id, false).await? {
            remove(
                tx,
                env,
                issue,
                &old,
                issue.id,
                RelationType::Duplicate,
                "duplicateOf",
                "duplicates",
                own,
            )
            .await?;
        }
    }
    add(
        tx,
        env,
        issue,
        &canonical,
        issue.id,
        RelationType::Duplicate,
        "duplicateOf",
        "duplicates",
        own,
    )
    .await?;
    transfer_subscribers(tx, env, issue, &canonical).await?;
    if issue.state_type != StateType::Duplicate {
        let dup_state = first_state_of(tx, team.id, StateType::Duplicate)
            .await?
            .ok_or_else(|| AppError::internal("the team has no duplicate state"))?;
        change_state(
            tx,
            team,
            issue,
            &dup_state,
            TransitionSource::MarkDuplicate,
            None,
            env.now,
            true,
        )
        .await?;
    }
    Ok(())
}

/// Abonnenten des Duplikats werden zusätzlich Abonnenten des Originals (docs/03 §4.3), sofern
/// sie das Original sehen dürfen; am Duplikat behalten sie ihr Abo.
async fn transfer_subscribers(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    dup: &Issue,
    canonical: &Issue,
) -> AppResult<()> {
    let subs = tx.notifications().issue_subscribers(dup.id).await?;
    if subs.is_empty() {
        return Ok(());
    }
    let mut allowed = Vec::new();
    {
        let mut fan =
            crate::usecases::notification::fanout::Fan::new(&mut *tx, env.now, None).await?;
        for s in subs {
            if fan.can_see_any(s, &[canonical.team_id]).await? {
                allowed.push(s);
            }
        }
    }
    for s in allowed {
        let outcome = tx
            .notifications()
            .subscribe_issue(
                canonical.id,
                s,
                None,
                SubscriptionReason::AddedByOther,
                env.now,
            )
            .await?;
        if outcome != SubscribeOutcome::Unchanged {
            tx.outbox()
                .push(NewEvent::from_ctx(
                    DomainEvent::IssueSubscriptionCreated {
                        issue_id: canonical.id,
                        user_id: s,
                    },
                    env.ctx,
                ))
                .await?;
        }
    }
    Ok(())
}

/// Entfernt `duplicateOf`; steht das Issue noch im Duplicate-State, wechselt es auf den
/// Default-State des Teams (docs/03 §4.3).
async fn unmark_duplicate(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    team: &Team,
    issue: &mut Issue,
    own: &mut Delta,
) -> AppResult<()> {
    let existing = tx.issues().relations_of(issue.id).await?;
    for r in existing
        .iter()
        .filter(|r| r.relation_type == RelationType::Duplicate && r.issue_id == issue.id)
    {
        if let Some(old) = tx.issues().get(r.related_issue_id, false).await? {
            remove(
                tx,
                env,
                issue,
                &old,
                issue.id,
                RelationType::Duplicate,
                "duplicateOf",
                "duplicates",
                own,
            )
            .await?;
        }
    }
    if issue.state_type == StateType::Duplicate {
        let s = super::fields::default_state(tx, team).await?;
        change_state(
            tx,
            team,
            issue,
            &s,
            TransitionSource::Update,
            None,
            env.now,
            false,
        )
        .await?;
    }
    Ok(())
}

/// Wendet alle Relationsänderungen an. Die History des Issues selbst sammelt `own`; die des
/// Gegenübers wird sofort geschrieben (docs/03 §4.2: Eintrag auf beiden Issues).
pub(super) async fn apply(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    team: &Team,
    issue: &mut Issue,
    c: &RelationChanges,
    own: &mut Delta,
) -> AppResult<()> {
    for r in &c.blocks {
        let o = target(tx, env, issue, r).await?;
        add(
            tx,
            env,
            issue,
            &o,
            issue.id,
            RelationType::Blocks,
            "blocks",
            "blockedBy",
            own,
        )
        .await?;
    }
    for r in &c.blocked_by {
        let o = target(tx, env, issue, r).await?;
        add(
            tx,
            env,
            issue,
            &o,
            o.id,
            RelationType::Blocks,
            "blockedBy",
            "blocks",
            own,
        )
        .await?;
    }
    for r in &c.related_to {
        let o = target(tx, env, issue, r).await?;
        add(
            tx,
            env,
            issue,
            &o,
            issue.id,
            RelationType::Related,
            "relatedTo",
            "relatedTo",
            own,
        )
        .await?;
    }
    for r in &c.remove_blocks {
        let o = target(tx, env, issue, r).await?;
        remove(
            tx,
            env,
            issue,
            &o,
            issue.id,
            RelationType::Blocks,
            "blocks",
            "blockedBy",
            own,
        )
        .await?;
    }
    for r in &c.remove_blocked_by {
        let o = target(tx, env, issue, r).await?;
        remove(
            tx,
            env,
            issue,
            &o,
            o.id,
            RelationType::Blocks,
            "blockedBy",
            "blocks",
            own,
        )
        .await?;
    }
    for r in &c.remove_related_to {
        let o = target(tx, env, issue, r).await?;
        remove(
            tx,
            env,
            issue,
            &o,
            issue.id,
            RelationType::Related,
            "relatedTo",
            "relatedTo",
            own,
        )
        .await?;
    }
    match &c.duplicate_of {
        Some(Some(r)) => mark_duplicate(tx, env, team, issue, r, own).await?,
        Some(None) => unmark_duplicate(tx, env, team, issue, own).await?,
        None => {}
    }
    Ok(())
}

/// Relationen eines Issues aus Sicht des Issues (`similar` wird nicht ausgegeben).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RelationsView {
    pub blocks: Vec<RelatedIssue>,
    pub blocked_by: Vec<RelatedIssue>,
    pub related_to: Vec<RelatedIssue>,
    pub duplicate_of: Option<RelatedIssue>,
    pub duplicates: Vec<RelatedIssue>,
}

/// Verwandtes Issue (nur Kennung und Titel).
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedIssue {
    pub uuid: Uuid,
    pub identifier: String,
    pub title: String,
}

/// Baut die Relationssicht; Gegenüber, die der Akteur nicht sehen darf oder die im Papierkorb
/// liegen, fehlen.
pub(super) async fn view(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    issue: IssueId,
) -> AppResult<RelationsView> {
    let rels: Vec<Relation> = tx.issues().relations_of(issue).await?;
    let mut out = RelationsView::default();
    for r in rels {
        let (other_id, outgoing) = if r.issue_id == issue {
            (r.related_issue_id, true)
        } else {
            (r.issue_id, false)
        };
        let Some(other) = tx.issues().get(other_id, false).await? else {
            continue;
        };
        if other.trashed || !resolve::issue::can_see(tx, snap, ctx, &other).await? {
            continue;
        }
        let item = RelatedIssue {
            uuid: other.id.0,
            identifier: other.identifier.clone(),
            title: other.title.clone(),
        };
        match (r.relation_type, outgoing) {
            (RelationType::Blocks, true) => out.blocks.push(item),
            (RelationType::Blocks, false) => out.blocked_by.push(item),
            (RelationType::Related, _) => out.related_to.push(item),
            (RelationType::Duplicate, true) => out.duplicate_of = Some(item),
            (RelationType::Duplicate, false) => out.duplicates.push(item),
            (RelationType::Similar, _) => {}
        }
    }
    Ok(out)
}
