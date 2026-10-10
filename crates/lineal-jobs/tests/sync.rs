#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP15: Outbox-Konsument `sync` (Event → SyncAction im `sync_log`) gegen echte Datenbank (Seed `acme`).

use chrono::Utc;
use lineal_app::ports::Store;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::ports::repos::outbox::SyncActionRow;
use lineal_app::usecases::notification::MarkNotificationInput;
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::*;
use lineal_jobs::consumers::sync::SyncConsumer;
use lineal_jobs::{EventConsumer, Worker, WorkerConfig};
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct Env {
    app: Arc<App>,
    pool: PgPool,
    d: SeedData,
}

async fn env(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let d = SeedData::acme();
    let mut tx = store.begin(d.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    Env {
        app: test_app(Arc::new(store)),
        pool,
        d,
    }
}

fn ev(org: OrgId, origin: EventOrigin, event: DomainEvent) -> StoredEvent {
    StoredEvent {
        id: EventId::new_v7(),
        org_id: org,
        origin,
        actor_id: None,
        sync_id: 7,
        event,
        created_at: Utc::now(),
    }
}

impl Env {
    async fn issue(&self, team: TeamId, slot: u128, number: i32, creator: UserId) -> IssueId {
        let id = IssueId::new_v7();
        sqlx::query(
            "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) \
             VALUES ($1,$2,$3,$4,'Login schlägt fehl',$5,$6)",
        )
        .bind(id.0)
        .bind(self.d.org.0)
        .bind(team.0)
        .bind(number)
        .bind(seed_workflow_state_ids(slot)[1].0)
        .bind(creator.0)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    async fn handle(&self, e: &StoredEvent) {
        SyncConsumer.handle(&self.app, e).await.unwrap();
    }

    /// Alle Zeilen des Logs (ohne Gruppenfilter).
    async fn log(&self) -> Vec<SyncActionRow> {
        let mut tx = self.app.store.begin(self.d.org).await.unwrap();
        let head = tx.outbox().sync_head().await.unwrap().head;
        let groups: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT unnest(sync_groups) FROM sync_log")
                .fetch_all(&self.pool)
                .await
                .unwrap();
        tx.outbox().sync_read(0, head, &groups, 1000).await.unwrap()
    }
}

fn team(t: TeamId) -> String {
    format!("team:{}", t.0)
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issue_event_becomes_graphql_shaped_action_for_its_team_only(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.des, 3, 101, e.d.bob).await;
    // Import-Events werden normal synchronisiert.
    let created = ev(
        e.d.org,
        EventOrigin::Import,
        DomainEvent::IssueCreated {
            issue_id: issue,
            team_id: e.d.des,
            mentioned_user_ids: vec![],
        },
    );
    e.handle(&created).await;
    // at-least-once: Wiederholung ändert nichts
    e.handle(&created).await;

    let log = e.log().await;
    assert_eq!(log.len(), 1);
    let row = &log[0];
    assert_eq!(
        (row.action.as_str(), row.model_name.as_str()),
        ("I", "Issue")
    );
    assert_eq!(row.model_id, issue.0);
    assert_eq!(row.groups, vec![team(e.d.des)]);
    assert_eq!(row.sync_id, 7);
    let d = &row.data;
    assert_eq!(d["id"], issue.0.to_string());
    assert_eq!(d["identifier"], "DES-101");
    assert_eq!(d["title"], "Login schlägt fehl");
    assert_eq!(d["stateName"], "Todo");
    assert_eq!(d["stateType"], "unstarted");
    assert_eq!(d["teamId"], e.d.des.0.to_string());
    assert_eq!(d["trashed"], false);
    assert_eq!(d["labelIds"], serde_json::json!([]));
    assert!(d["createdAt"].as_str().unwrap().ends_with('Z'));
    assert!(d["assigneeId"].is_null());
    // keine E-Mails oder Hashes im Payload
    assert!(!d.to_string().contains('@'));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn team_move_tells_the_old_team_that_the_issue_is_gone(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 102, e.d.alice).await;
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::IssueCreated {
            issue_id: issue,
            team_id: e.d.eng,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    sqlx::query("UPDATE issue SET team_id = $2, state_id = $3, number = 103 WHERE id = $1")
        .bind(issue.0)
        .bind(e.d.web.0)
        .bind(seed_workflow_state_ids(2)[1].0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::IssueUpdated {
            issue_id: issue,
            team_id: e.d.web,
            changed: vec!["teamId".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    let log = e.log().await;
    let tail: Vec<(&str, &Vec<String>)> = log[1..]
        .iter()
        .map(|r| (r.action.as_str(), &r.groups))
        .collect();
    assert_eq!(
        tail,
        vec![("D", &vec![team(e.d.eng)]), ("U", &vec![team(e.d.web)]),]
    );
    assert_eq!(log[2].data["identifier"], "WEB-103");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn removal_archive_and_restore(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 104, e.d.alice).await;
    // Papierkorb: Zeile existiert noch → A mit Zustand
    sqlx::query("UPDATE issue SET trashed = true, archived_at = now() WHERE id = $1")
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let removed = DomainEvent::IssueRemoved {
        issue_id: issue,
        team_id: e.d.eng,
    };
    e.handle(&ev(e.d.org, EventOrigin::User, removed.clone()))
        .await;
    // Wiederherstellen → V
    sqlx::query("UPDATE issue SET trashed = false, archived_at = NULL WHERE id = $1")
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::IssueUpdated {
            issue_id: issue,
            team_id: e.d.eng,
            changed: vec!["trashed".into(), "archivedAt".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    // Endgültig gelöscht → D mit den Gruppen des Events
    sqlx::query("DELETE FROM issue WHERE id = $1")
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(e.d.org, EventOrigin::User, removed)).await;
    let log = e.log().await;
    let actions: Vec<&str> = log.iter().map(|r| r.action.as_str()).collect();
    assert_eq!(actions, ["A", "V", "D"]);
    assert_eq!(log[0].data["trashed"], true);
    assert_eq!(log[2].groups, vec![team(e.d.eng)]);
    assert_eq!(log[2].data, serde_json::json!({}));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn comment_removal_falls_back_to_last_known_groups(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.des, 3, 105, e.d.bob).await;
    let comment = CommentId::new_v7();
    sqlx::query(
        "INSERT INTO comment (id, organization_id, body, user_id, issue_id) VALUES ($1,$2,'Hallo',$3,$4)",
    )
    .bind(comment.0)
    .bind(e.d.org.0)
    .bind(e.d.bob.0)
    .bind(issue.0)
    .execute(&e.pool)
    .await
    .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    sqlx::query("DELETE FROM comment WHERE id = $1")
        .bind(comment.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::CommentRemoved {
            comment_id: comment,
        },
    ))
    .await;
    let log = e.log().await;
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].groups, vec![team(e.d.des)]);
    assert_eq!(log[0].data["body"], "Hallo");
    assert_eq!(
        (log[1].action.as_str(), &log[1].groups),
        ("D", &vec![team(e.d.des)])
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn notification_actions_carry_only_the_owners_group(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 106, e.d.alice).await;
    let n = NotificationId::new_v7();
    sqlx::query(
        "INSERT INTO notification (id, organization_id, user_id, type, category, target_type, actor_id, \
         issue_id, dedup_key) VALUES ($1,$2,$3,'issueAssignedToYou','assignments','issue',$4,$5,'x')",
    )
    .bind(n.0)
    .bind(e.d.org.0)
    .bind(e.d.bob.0)
    .bind(e.d.alice.0)
    .bind(issue.0)
    .execute(&e.pool)
    .await
    .unwrap();
    let bob = e.d.bob;
    for event in [
        DomainEvent::NotificationCreated {
            notification_id: n,
            user_id: bob,
        },
        DomainEvent::NotificationUpdated {
            notification_id: n,
            user_id: bob,
        },
    ] {
        e.handle(&ev(e.d.org, EventOrigin::System, event)).await;
    }
    sqlx::query("UPDATE notification SET archived_at = now() WHERE id = $1")
        .bind(n.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::NotificationArchived {
            notification_id: n,
            user_id: bob,
        },
    ))
    .await;
    sqlx::query("UPDATE notification SET archived_at = NULL WHERE id = $1")
        .bind(n.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::NotificationArchived {
            notification_id: n,
            user_id: bob,
        },
    ))
    .await;
    let log = e.log().await;
    let actions: Vec<&str> = log.iter().map(|r| r.action.as_str()).collect();
    assert_eq!(actions, ["I", "U", "A", "V"]);
    for row in &log {
        assert_eq!(
            row.groups,
            vec![format!("user:{}", bob.0)],
            "nur der Besitzer"
        );
        assert_eq!(row.model_name, "Notification");
    }
    assert_eq!(log[0].data["type"], "issueAssignedToYou");
    assert_eq!(log[0].data["issueIdentifier"], "ENG-106");
    assert!(log[0].data["title"].is_string());
    assert!(log[2].data["archivedAt"].is_string());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn membership_actions_reach_team_and_affected_user(pool: PgPool) {
    let e = env(pool).await;
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::TeamMembershipRemoved {
            team_id: e.d.des,
            user_id: e.d.bob,
        },
    ))
    .await;
    // Ereignisse ohne Sync-Bedeutung erzeugen nichts.
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::ApiKeyCreated { user_id: e.d.bob },
    ))
    .await;
    let log = e.log().await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].model_name, "TeamMembership");
    assert_eq!(log[0].action, "D");
    let mut groups = log[0].groups.clone();
    groups.sort();
    let mut want = vec![team(e.d.des), format!("user:{}", e.d.bob.0)];
    want.sort();
    assert_eq!(groups, want);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn every_model_loader_is_valid_sql(pool: PgPool) {
    let e = env(pool).await;
    let mut tx = e.app.store.begin(e.d.org).await.unwrap();
    for name in [
        "Issue",
        "Comment",
        "Attachment",
        "Reaction",
        "IssueRelation",
        "IssueRelationReverse",
        "WorkflowState",
        "IssueLabel",
        "Cycle",
        "Project",
        "ProjectMilestone",
        "ProjectUpdate",
        "Initiative",
        "InitiativeUpdate",
        "Document",
        "Team",
        "Template",
        "CustomView",
        "User",
        "Organization",
    ] {
        let got = tx.outbox().sync_model(name, Uuid::now_v7()).await;
        assert!(matches!(got, Ok(None)), "{name}: {got:?}");
    }
    // Existierende Objekte aus dem Seed liefern Daten und Gruppen.
    for (name, id, group) in [
        ("Team", e.d.des.0, team(e.d.des)),
        (
            "WorkflowState",
            seed_workflow_state_ids(3)[0].0,
            team(e.d.des),
        ),
        ("User", e.d.bob.0, "org".to_owned()),
        ("Organization", e.d.org.0, "org".to_owned()),
    ] {
        let s = tx.outbox().sync_model(name, id).await.unwrap().unwrap();
        assert_eq!(s.groups, vec![group], "{name}");
        assert_eq!(s.data["id"], id.to_string());
    }
    // `User` verrät keine E-Mail.
    let u = tx
        .outbox()
        .sync_model("User", e.d.bob.0)
        .await
        .unwrap()
        .unwrap();
    assert!(!u.data.to_string().contains('@'));
    assert!(
        tx.outbox()
            .sync_model("Nonsense", Uuid::now_v7())
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn worker_feeds_sync_log_and_notification_use_cases_emit_events(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 107, e.d.alice).await;
    let n = NotificationId::new_v7();
    sqlx::query(
        "INSERT INTO notification (id, organization_id, user_id, type, category, target_type, actor_id, \
         issue_id, dedup_key) VALUES ($1,$2,$3,'issueAssignedToYou','assignments','issue',$4,$5,'y')",
    )
    .bind(n.0)
    .bind(e.d.org.0)
    .bind(e.d.bob.0)
    .bind(e.d.alice.0)
    .bind(issue.0)
    .execute(&e.pool)
    .await
    .unwrap();

    // mark / archive aus den Use-Cases erzeugen Events nur für den Besitzer
    let bob = ctx_for(SeedUser::Bob);
    e.app
        .mark_notification(
            &bob,
            n,
            MarkNotificationInput {
                read: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    e.app.archive_notification(&bob, n, true).await.unwrap();
    // Fremde Notification → not_found und kein Event
    let alice = ctx_for(SeedUser::Alice);
    let err = e
        .app
        .archive_notification(&alice, n, true)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);

    let types: Vec<(String, Value)> =
        sqlx::query_as("SELECT type, payload FROM outbox_event ORDER BY sync_id")
            .fetch_all(&e.pool)
            .await
            .unwrap();
    let types: Vec<&str> = types.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(types, ["NotificationUpdated", "NotificationArchived"]);

    // Der Worker verteilt sie an den Konsumenten und schreibt das Log.
    let worker = Worker::with(
        e.app.clone(),
        vec![Arc::new(SyncConsumer)],
        vec![],
        WorkerConfig::default(),
    );
    worker.tick().await.unwrap();
    let log = e.log().await;
    let actions: Vec<&str> = log.iter().map(|r| r.action.as_str()).collect();
    assert_eq!(actions, ["U", "A"]);
    assert!(
        log.iter()
            .all(|r| r.groups == vec![format!("user:{}", e.d.bob.0)])
    );
    assert_eq!(
        e.app
            .store
            .begin(e.d.org)
            .await
            .unwrap()
            .outbox()
            .sync_head()
            .await
            .unwrap()
            .head,
        log[1].id
    );
}
