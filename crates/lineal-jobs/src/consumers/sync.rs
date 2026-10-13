//! Outbox-Konsument `sync` (WP15, docs/09 §5, docs/11 §3.7): übersetzt Domain-Events in
//! `SyncAction`s und schreibt sie ins `sync_log`; `pg_notify` weckt die WebSocket-Verbindungen.
//!
//! Aktionen: `I` insert, `U` update, `A` archive, `V` unarchive, `D` delete. `data` ist der
//! Objektzustand zum Verarbeitungszeitpunkt (Issue exakt in der GraphQL-Form), `groups` sind die
//! Berechtigungsschlüssel `team:<id>`, `org`, `orgpub`, `tmem:<id>`, `user:<id>` (siehe
//! `lineal-sync`: Freigaben, Gäste); der Sync-Adapter filtert beim Zustellen
//! gegen die dann sichtbaren Teams. Events mit `origin = import` werden normal synchronisiert.
//!
//! Die Verarbeitung ist idempotent (`UNIQUE (event_id, n)`), je Event eine Transaktion.

use crate::EventConsumer;
use async_trait::async_trait;
use chrono::{Duration, SecondsFormat, Utc};
use lineal_app::ports::repos::StoredEvent;
use lineal_app::ports::repos::outbox::NewSyncAction;
use lineal_app::{Actor, App, AppResult, ErrorKind, RequestCtx, Scopes};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::UserId;
use serde_json::{Value, json};
use uuid::Uuid;

/// Aufbewahrung des Sync-Logs (docs/09 §5.3).
pub const RETENTION_DAYS: i64 = 14;

/// Alle N Aktionen wird abgelaufenes Log gelöscht (kein eigener periodischer Job nötig).
const PURGE_EVERY: i64 = 64;

pub struct SyncConsumer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Created,
    Updated,
    Removed,
}

/// Was ein Event für den Sync-Strom bedeutet.
#[derive(Debug, Clone, PartialEq)]
enum Plan {
    /// Zustand und Gruppen aus der Datenbank laden.
    Model {
        model: &'static str,
        id: Uuid,
        op: Op,
        /// Bekannte Gruppen für den Fall, dass das Objekt nicht mehr existiert.
        known_groups: Vec<String>,
        /// Für Issues: geänderte Felder (Wiederherstellung → `V`).
        changed: Vec<String>,
    },
    /// Aus dem Event selbst abgeleitet (keine Zeile zum Laden).
    Direct {
        model: &'static str,
        id: Uuid,
        action: &'static str,
        data: Value,
        groups: Vec<String>,
    },
    /// Persönliche Notification (nur an `user`).
    Notification {
        id: Uuid,
        user: UserId,
        op: NotificationOp,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotificationOp {
    Created,
    Updated,
    Archived,
}

fn team_group(id: impl std::fmt::Display) -> String {
    format!("team:{id}")
}

fn user_group(id: impl std::fmt::Display) -> String {
    format!("user:{id}")
}

fn model(model: &'static str, id: Uuid, op: Op) -> Vec<Plan> {
    vec![Plan::Model {
        model,
        id,
        op,
        known_groups: vec![],
        changed: vec![],
    }]
}

/// Favoriten sind persönlich: die Aktion geht nur an die Gruppe des Besitzers.
fn favorite_plan(id: Uuid, user: UserId, action: &'static str) -> Vec<Plan> {
    vec![Plan::Direct {
        model: "Favorite",
        id,
        action,
        data: json!({ "id": id }),
        groups: vec![user_group(user.0)],
    }]
}

/// Abbildung Event → Sync-Plan. Unbekannte/irrelevante Events (API-Keys, OAuth) → leer.
fn plan(ev: &StoredEvent) -> Vec<Plan> {
    use DomainEvent as E;
    use Op::{Created, Removed, Updated};
    match &ev.event {
        E::IssueCreated {
            issue_id, team_id, ..
        } => vec![Plan::Model {
            model: "Issue",
            id: issue_id.0,
            op: Created,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::IssueUpdated {
            issue_id,
            team_id,
            changed,
            ..
        } => vec![Plan::Model {
            model: "Issue",
            id: issue_id.0,
            op: Updated,
            known_groups: vec![team_group(team_id.0)],
            changed: changed.clone(),
        }],
        E::IssueRemoved { issue_id, team_id } => vec![Plan::Model {
            model: "Issue",
            id: issue_id.0,
            op: Removed,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::IssueTriaged { issue_id, .. }
        | E::IssueSlaSet { issue_id }
        | E::IssueSlaHighRisk { issue_id }
        | E::IssueSlaBreached { issue_id } => model("Issue", issue_id.0, Updated),
        E::IssueRelationCreated { relation_id, .. } => vec![
            Plan::Model {
                model: "IssueRelation",
                id: relation_id.0,
                op: Created,
                known_groups: vec![],
                changed: vec![],
            },
            Plan::Model {
                model: "IssueRelationReverse",
                id: relation_id.0,
                op: Created,
                known_groups: vec![],
                changed: vec![],
            },
        ],
        E::IssueRelationRemoved { relation_id, .. } => {
            model("IssueRelation", relation_id.0, Removed)
        }
        E::CommentCreated { comment_id, .. } => model("Comment", comment_id.0, Created),
        E::CommentUpdated { comment_id, .. } => model("Comment", comment_id.0, Updated),
        E::CommentRemoved { comment_id } => model("Comment", comment_id.0, Removed),
        E::IssueLabelCreated { label_id } => model("IssueLabel", label_id.0, Created),
        E::IssueLabelUpdated { label_id } => model("IssueLabel", label_id.0, Updated),
        E::IssueLabelRemoved { label_id } => model("IssueLabel", label_id.0, Removed),
        E::ReactionCreated { reaction_id } => model("Reaction", reaction_id.0, Created),
        E::ReactionRemoved { reaction_id } => model("Reaction", reaction_id.0, Removed),
        E::AttachmentCreated { attachment_id } => model("Attachment", attachment_id.0, Created),
        E::AttachmentUpdated { attachment_id } => model("Attachment", attachment_id.0, Updated),
        E::AttachmentRemoved { attachment_id } => model("Attachment", attachment_id.0, Removed),
        E::ProjectCreated { project_id } => model("Project", project_id.0, Created),
        E::ProjectUpdated { project_id, .. } => model("Project", project_id.0, Updated),
        E::ProjectRemoved { project_id } => model("Project", project_id.0, Removed),
        E::ProjectMilestoneCreated { milestone_id, .. } => {
            model("ProjectMilestone", milestone_id.0, Created)
        }
        E::ProjectMilestoneUpdated { milestone_id, .. } => {
            model("ProjectMilestone", milestone_id.0, Updated)
        }
        E::ProjectMilestoneRemoved { milestone_id, .. } => {
            model("ProjectMilestone", milestone_id.0, Removed)
        }
        E::ProjectUpdateCreated { update_id, .. } => model("ProjectUpdate", update_id.0, Created),
        E::ProjectUpdateUpdated { update_id, .. } => model("ProjectUpdate", update_id.0, Updated),
        E::ProjectUpdateRemoved { update_id, .. } => model("ProjectUpdate", update_id.0, Removed),
        E::CycleCreated { cycle_id, team_id } => vec![Plan::Model {
            model: "Cycle",
            id: cycle_id.0,
            op: Created,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::CycleUpdated { cycle_id, team_id } => vec![Plan::Model {
            model: "Cycle",
            id: cycle_id.0,
            op: Updated,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::CycleRemoved { cycle_id, team_id } => vec![Plan::Model {
            model: "Cycle",
            id: cycle_id.0,
            op: Removed,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::DocumentCreated { document_id } => model("Document", document_id.0, Created),
        E::DocumentUpdated { document_id } => model("Document", document_id.0, Updated),
        E::DocumentRemoved { document_id } => model("Document", document_id.0, Removed),
        E::InitiativeCreated { initiative_id } => model("Initiative", initiative_id.0, Created),
        E::InitiativeUpdated { initiative_id, .. } => model("Initiative", initiative_id.0, Updated),
        E::InitiativeRemoved { initiative_id } => model("Initiative", initiative_id.0, Removed),
        E::InitiativeUpdateCreated { update_id, .. } => {
            model("InitiativeUpdate", update_id.0, Created)
        }
        E::InitiativeUpdateUpdated { update_id, .. } => {
            model("InitiativeUpdate", update_id.0, Updated)
        }
        E::InitiativeUpdateRemoved { update_id, .. } => {
            model("InitiativeUpdate", update_id.0, Removed)
        }
        E::UserCreated { user_id } => model("User", user_id.0, Created),
        E::UserUpdated { user_id } => model("User", user_id.0, Updated),
        E::TeamCreated { team_id } => model("Team", team_id.0, Created),
        E::TeamUpdated { team_id } => model("Team", team_id.0, Updated),
        E::TeamRemoved { team_id } => vec![Plan::Model {
            model: "Team",
            id: team_id.0,
            op: Removed,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::WorkflowStateCreated { state_id, team_id } => vec![Plan::Model {
            model: "WorkflowState",
            id: state_id.0,
            op: Created,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::WorkflowStateUpdated { state_id, team_id } => vec![Plan::Model {
            model: "WorkflowState",
            id: state_id.0,
            op: Updated,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::WorkflowStateRemoved { state_id, team_id } => vec![Plan::Model {
            model: "WorkflowState",
            id: state_id.0,
            op: Removed,
            known_groups: vec![team_group(team_id.0)],
            changed: vec![],
        }],
        E::TemplateCreated { template_id } => model("Template", template_id.0, Created),
        E::TemplateUpdated { template_id } => model("Template", template_id.0, Updated),
        E::TemplateRemoved { template_id } => model("Template", template_id.0, Removed),
        E::CustomViewCreated { view_id } => model("CustomView", view_id.0, Created),
        E::CustomViewUpdated { view_id } => model("CustomView", view_id.0, Updated),
        E::CustomViewRemoved { view_id } => model("CustomView", view_id.0, Removed),
        E::OrganizationUpdated {} => model("Organization", ev.org_id.0, Updated),
        // Mitgliedschaft: Mitglieder des Teams und der betroffene User selbst. Der User erhält die
        // Aktion auch nach dem Entfernen (persönliche Gruppe), das Team danach nicht mehr.
        // Zusätzlich die Person selbst: ihre Gruppen enthalten `tmem:<team>` je Mitgliedschaft, damit
        // Gäste des Teams sie (nicht mehr) im Verzeichnis haben (Entzug → `D` an `tmem:<team>`).
        E::TeamMembershipCreated { team_id, user_id } => vec![
            Plan::Direct {
                model: "TeamMembership",
                id: user_id.0,
                action: "I",
                data: json!({ "teamId": team_id.0, "userId": user_id.0 }),
                groups: vec![team_group(team_id.0), user_group(user_id.0)],
            },
            Plan::Model {
                model: "User",
                id: user_id.0,
                op: Updated,
                known_groups: vec![],
                changed: vec![],
            },
        ],
        E::TeamMembershipRemoved { team_id, user_id } => vec![
            Plan::Direct {
                model: "TeamMembership",
                id: user_id.0,
                action: "D",
                data: json!({ "teamId": team_id.0, "userId": user_id.0 }),
                groups: vec![team_group(team_id.0), user_group(user_id.0)],
            },
            Plan::Model {
                model: "User",
                id: user_id.0,
                op: Updated,
                known_groups: vec![],
                changed: vec![],
            },
        ],
        E::FavoriteCreated {
            favorite_id,
            user_id,
        } => favorite_plan(*favorite_id, *user_id, "I"),
        E::FavoriteUpdated {
            favorite_id,
            user_id,
        } => favorite_plan(*favorite_id, *user_id, "U"),
        E::FavoriteRemoved {
            favorite_id,
            user_id,
        } => favorite_plan(*favorite_id, *user_id, "D"),
        E::IssueSubscriptionCreated { issue_id, user_id } => vec![Plan::Direct {
            model: "IssueSubscription",
            id: issue_id.0,
            action: "I",
            data: json!({ "issueId": issue_id.0 }),
            groups: vec![user_group(user_id.0)],
        }],
        E::IssueSubscriptionRemoved { issue_id, user_id } => vec![Plan::Direct {
            model: "IssueSubscription",
            id: issue_id.0,
            action: "D",
            data: json!({ "issueId": issue_id.0 }),
            groups: vec![user_group(user_id.0)],
        }],
        E::NotificationCreated {
            notification_id,
            user_id,
        } => vec![Plan::Notification {
            id: notification_id.0,
            user: *user_id,
            op: NotificationOp::Created,
        }],
        E::NotificationUpdated {
            notification_id,
            user_id,
        } => vec![Plan::Notification {
            id: notification_id.0,
            user: *user_id,
            op: NotificationOp::Updated,
        }],
        E::NotificationArchived {
            notification_id,
            user_id,
        } => vec![Plan::Notification {
            id: notification_id.0,
            user: *user_id,
            op: NotificationOp::Archived,
        }],
        E::OAuthAuthorizationRevoked { .. } | E::ApiKeyCreated { .. } | E::ApiKeyRevoked { .. } => {
            vec![]
        }
    }
}

fn ts(t: chrono::DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Notification in der GraphQL-Form (`Notification`), geladen mit den Rechten des Empfängers.
/// `None`, wenn sie für ihn nicht (mehr) sichtbar ist.
async fn notification_data(
    app: &App,
    ev: &StoredEvent,
    user: UserId,
    id: Uuid,
) -> AppResult<Option<Value>> {
    let role = {
        let mut tx = app.store.begin(ev.org_id).await?;
        match tx.users().get(user).await? {
            Some(u) if u.active => u.role,
            _ => return Ok(None),
        }
    };
    let ctx = RequestCtx {
        org_id: ev.org_id,
        actor: Actor::User { id: user, role },
        request_id: format!("sync_{}", ev.id),
        scopes: Scopes::read_only(),
        team_scope: None,

        via_session: false,
        client_ip: None,
        credential_kind: None,
    };
    match app
        .get_notification(&ctx, lineal_domain::ids::NotificationId(id))
        .await
    {
        Ok(item) => {
            let n = &item.view.n;
            Ok(Some(json!({
                "id": n.id.0, "type": n.kind, "category": n.category,
                "title": item.title, "subtitle": item.subtitle, "url": item.url,
                "groupKey": n.group_key,
                "actorId": n.actor_id.map(|a| a.0), "actorName": item.view.actor_name,
                "issueId": n.issue_id.map(|a| a.0), "issueIdentifier": item.view.issue_identifier,
                "projectId": n.project_id.map(|a| a.0), "initiativeId": n.initiative_id,
                "documentId": n.document_id, "commentId": n.comment_id,
                "projectMilestoneId": n.project_milestone_id, "projectUpdateId": n.project_update_id,
                "initiativeUpdateId": n.initiative_update_id, "bundledTypes": n.bundled_types,
                "isPriority": n.is_priority,
                "readAt": n.read_at.map(ts), "snoozedUntilAt": n.snoozed_until_at.map(ts),
                "unsnoozedAt": n.unsnoozed_at.map(ts), "archivedAt": n.archived_at.map(ts),
                "createdAt": ts(n.created_at), "updatedAt": ts(n.updated_at),
            })))
        }
        Err(e) if e.kind == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn is_restore(model: &str, changed: &[String], data: &Value) -> bool {
    model == "Issue"
        && changed.iter().any(|c| c == "archivedAt" || c == "trashed")
        && data["archivedAt"].is_null()
        && data["trashed"] == Value::Bool(false)
}

#[async_trait]
impl EventConsumer for SyncConsumer {
    fn name(&self) -> &'static str {
        "sync"
    }

    async fn handle(&self, app: &App, ev: &StoredEvent) -> AppResult<()> {
        let plans = plan(ev);
        if plans.is_empty() {
            return Ok(());
        }
        // Notifications vorab laden (eigene Transaktion mit den Rechten des Empfängers).
        let mut notif: Vec<Option<Value>> = Vec::new();
        for p in &plans {
            if let Plan::Notification { id, user, .. } = p {
                notif.push(notification_data(app, ev, *user, *id).await?);
            }
        }
        let mut notif = notif.into_iter();

        let mut tx = app.store.begin(ev.org_id).await?;
        let mut out: Vec<NewSyncAction> = Vec::new();
        let push = |out: &mut Vec<NewSyncAction>,
                    action: &str,
                    model: &str,
                    id: Uuid,
                    data: Value,
                    groups: Vec<String>| {
            if groups.is_empty() {
                return;
            }
            out.push(NewSyncAction {
                event_id: ev.id,
                n: i16::try_from(out.len()).unwrap_or(i16::MAX),
                sync_id: ev.sync_id,
                action: action.to_owned(),
                model_name: model.to_owned(),
                model_id: id,
                data,
                groups,
                origin: ev.origin,
                actor_id: ev.actor_id,
            });
        };

        for p in plans {
            match p {
                Plan::Direct {
                    model,
                    id,
                    action,
                    data,
                    groups,
                } => push(&mut out, action, model, id, data, groups),
                Plan::Notification { id, user, op } => {
                    let data = notif.next().flatten();
                    let archived = data.as_ref().is_some_and(|d| !d["archivedAt"].is_null());
                    let action = match op {
                        NotificationOp::Created => "I",
                        NotificationOp::Updated => "U",
                        NotificationOp::Archived if archived || data.is_none() => "A",
                        NotificationOp::Archived => "V",
                    };
                    push(
                        &mut out,
                        action,
                        "Notification",
                        id,
                        data.unwrap_or_else(|| json!({ "id": id })),
                        vec![user_group(user.0)],
                    );
                }
                Plan::Model {
                    model,
                    id,
                    op,
                    known_groups,
                    changed,
                } => {
                    // Beide Seiten einer Relation werden unter demselben Modellnamen geführt.
                    let log_name = if model == "IssueRelationReverse" {
                        "IssueRelation"
                    } else {
                        model
                    };
                    let state = tx.outbox().sync_model(model, id).await?;
                    match (op, state) {
                        (Op::Removed, Some(s)) => {
                            let action = if model == "Comment" { "U" } else { "A" };
                            push(&mut out, action, log_name, id, s.data, s.groups);
                        }
                        (Op::Removed, None) => {
                            let groups = if known_groups.is_empty() {
                                tx.outbox()
                                    .sync_last_groups(log_name, id)
                                    .await?
                                    .unwrap_or_default()
                            } else {
                                known_groups
                            };
                            push(&mut out, "D", log_name, id, json!({}), groups);
                        }
                        (_, None) => {} // inzwischen gelöscht; das Removed-Event folgt
                        (op, Some(s)) => {
                            let action = match op {
                                Op::Created => "I",
                                _ if is_restore(model, &changed, &s.data) => "V",
                                _ => "U",
                            };
                            // Wechselt das Objekt die Gruppen (z. B. Issue in anderes Team), erfahren
                            // die bisherigen Empfänger, dass es für sie verschwunden ist.
                            if op == Op::Updated
                                && let Some(prev) =
                                    tx.outbox().sync_last_groups(log_name, id).await?
                            {
                                let gone: Vec<String> =
                                    prev.into_iter().filter(|g| !s.groups.contains(g)).collect();
                                push(&mut out, "D", log_name, id, json!({}), gone);
                            }
                            push(&mut out, action, log_name, id, s.data, s.groups);
                        }
                    }
                }
            }
        }

        if let Some(last) = tx.outbox().sync_append(&out).await?
            && last % PURGE_EVERY == 0
        {
            let before = app.clock.now() - Duration::days(RETENTION_DAYS);
            tx.outbox().sync_purge(before).await?;
        }
        tx.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_domain::events::EventOrigin;
    use lineal_domain::ids::{EventId, IssueId, NotificationId, OrgId, TeamId};

    fn stored(event: DomainEvent) -> StoredEvent {
        StoredEvent {
            id: EventId::new_v7(),
            org_id: OrgId::new_v7(),
            origin: EventOrigin::Import,
            actor_id: None,
            sync_id: 1,
            event,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn every_event_type_is_mapped_or_explicitly_skipped() {
        // Neue Events dürfen nicht stillschweigend aus dem Sync fallen: `plan` ist ein
        // erschöpfendes `match`; hier nur ein Stichprobentest der Zuordnung.
        let team = TeamId::new_v7();
        let issue = IssueId::new_v7();
        let p = plan(&stored(DomainEvent::IssueRemoved {
            issue_id: issue,
            team_id: team,
        }));
        assert_eq!(p.len(), 1);
        assert!(matches!(
            &p[0],
            Plan::Model { model: "Issue", op: Op::Removed, known_groups, .. }
                if known_groups == &vec![format!("team:{}", team.0)]
        ));
        assert!(
            plan(&stored(DomainEvent::ApiKeyCreated {
                user_id: UserId::new_v7()
            }))
            .is_empty()
        );
        let user = UserId::new_v7();
        let p = plan(&stored(DomainEvent::NotificationArchived {
            notification_id: NotificationId::new_v7(),
            user_id: user,
        }));
        assert!(matches!(&p[0], Plan::Notification { user: u, .. } if *u == user));
    }
}
