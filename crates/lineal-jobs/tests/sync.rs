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
            initial: None,
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
            initial: None,
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
            previous: None,
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
            previous: None,
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
    // Mitgliedschaft + die Person selbst (ihre Gruppen enthalten `tmem:<team>` je Mitgliedschaft).
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].model_name, "TeamMembership");
    assert_eq!(log[0].action, "D");
    let mut groups = log[0].groups.clone();
    groups.sort();
    let mut want = vec![team(e.d.des), format!("user:{}", e.d.bob.0)];
    want.sort();
    assert_eq!(groups, want);
    assert_eq!(
        (log[1].model_name.as_str(), log[1].action.as_str()),
        ("User", "U")
    );
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
        ("Organization", e.d.org.0, "org".to_owned()),
    ] {
        let s = tx.outbox().sync_model(name, id).await.unwrap().unwrap();
        assert!(s.groups.contains(&group), "{name}: {:?}", s.groups);
        assert_eq!(s.data["id"], id.to_string());
    }
    // Workspace-Metadaten sind auch für Gäste lesbar (`orgpub`); Personen tragen `org`, sich selbst
    // und ein `tmem:<team>` je Mitgliedschaft (Gäste sehen nur Personen ihrer Teams).
    let org = tx
        .outbox()
        .sync_model("Organization", e.d.org.0)
        .await
        .unwrap()
        .unwrap();
    assert!(org.groups.contains(&"orgpub".to_owned()));
    let bob = tx
        .outbox()
        .sync_model("User", e.d.bob.0)
        .await
        .unwrap()
        .unwrap();
    assert!(bob.groups.contains(&"org".to_owned()));
    assert!(bob.groups.contains(&format!("user:{}", e.d.bob.0)));
    for t in [e.d.eng, e.d.web, e.d.des] {
        assert!(
            bob.groups.contains(&format!("tmem:{}", t.0)),
            "{:?}",
            bob.groups
        );
    }
    let carol = tx
        .outbox()
        .sync_model("User", e.d.carol.0)
        .await
        .unwrap()
        .unwrap();
    assert!(!carol.groups.contains(&format!("tmem:{}", e.d.eng.0)));
    assert!(carol.groups.contains(&format!("tmem:{}", e.d.web.0)));
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

// --- WP32: Freigaben und Gäste ----------------------------------------------------------------

fn group(g: &str, id: Uuid) -> String {
    format!("{g}:{id}")
}

fn updated(issue: IssueId, team: TeamId, changed: &[&str]) -> DomainEvent {
    DomainEvent::IssueUpdated {
        previous: None,
        issue_id: issue,
        team_id: team,
        changed: changed.iter().map(|c| (*c).to_owned()).collect(),
        mentioned_user_ids: vec![],
    }
}

async fn share(e: &Env, issue: IssueId, user: UserId) {
    sqlx::query(
        "INSERT INTO issue_share (organization_id, issue_id, user_id, shared_by_id) VALUES ($1,$2,$3,$4)",
    )
    .bind(e.d.org.0)
    .bind(issue.0)
    .bind(user.0)
    .bind(e.d.bob.0)
    .execute(&e.pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn shared_issue_carries_the_recipients_user_group_and_unshare_removes_it(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 150, e.d.bob).await;
    let carol = group("user", e.d.carol.0);

    // Freigabe an Carol: das Issue (und nur dieses) trägt `user:<carol>` zusätzlich zum Team.
    share(&e, issue, e.d.carol).await;
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        updated(issue, e.d.eng, &["sharedWith"]),
    ))
    .await;
    let log = e.log().await;
    assert_eq!(log.len(), 1);
    assert_eq!(
        (log[0].action.as_str(), log[0].model_name.as_str()),
        ("U", "Issue")
    );
    let mut groups = log[0].groups.clone();
    groups.sort();
    let mut want = vec![team(e.d.eng), carol.clone()];
    want.sort();
    assert_eq!(groups, want);
    assert!(
        !log[0].groups.contains(&team(e.d.web)),
        "keine weiteren Teams"
    );

    // Weitere Änderungen am Issue erreichen Carol weiter.
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        updated(issue, e.d.eng, &["title"]),
    ))
    .await;
    let log = e.log().await;
    assert!(log.last().unwrap().groups.contains(&carol));

    // Entzug: Carol erhält ein `D` (nur an ihre Gruppe), das Team ein normales `U` ohne `user:`.
    sqlx::query("DELETE FROM issue_share WHERE issue_id = $1 AND user_id = $2")
        .bind(issue.0)
        .bind(e.d.carol.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        updated(issue, e.d.eng, &["sharedWith"]),
    ))
    .await;
    let log = e.log().await;
    let n = log.len();
    let (d, u) = (&log[n - 2], &log[n - 1]);
    assert_eq!(d.action, "D");
    assert_eq!(d.groups, vec![carol.clone()]);
    assert_eq!(u.action, "U");
    assert_eq!(u.groups, vec![team(e.d.eng)]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn comments_on_shared_issues_follow_the_share_and_unshared_ones_stay_in_the_team(
    pool: PgPool,
) {
    let e = env(pool).await;
    let shared = e.issue(e.d.eng, 1, 151, e.d.bob).await;
    let private = e.issue(e.d.eng, 1, 152, e.d.bob).await;
    share(&e, shared, e.d.carol).await;
    let mut comments = vec![];
    for issue in [shared, private] {
        let id = CommentId::new_v7();
        sqlx::query(
            "INSERT INTO comment (id, organization_id, body, user_id, issue_id) VALUES ($1,$2,'Hallo',$3,$4)",
        )
        .bind(id.0)
        .bind(e.d.org.0)
        .bind(e.d.bob.0)
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
        comments.push(id);
    }
    for c in &comments {
        e.handle(&ev(
            e.d.org,
            EventOrigin::User,
            DomainEvent::CommentCreated {
                comment_id: *c,
                mentioned_user_ids: vec![],
            },
        ))
        .await;
    }
    let log = e.log().await;
    let carol = group("user", e.d.carol.0);
    assert!(
        log[0].groups.contains(&carol),
        "Kommentar am geteilten Issue"
    );
    assert!(
        !log[1].groups.contains(&carol),
        "Kommentar am nicht geteilten Issue bleibt im Team"
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn guest_groups_expose_only_workspace_labels_and_people_of_own_teams(pool: PgPool) {
    let e = env(pool).await;
    let mut tx = e.app.store.begin(e.d.org).await.unwrap();
    // Workspace-Label: `org` und `orgpub`; Team-Label: nur das Team.
    let ws_label = Uuid::now_v7();
    let team_label = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO issue_label (id, organization_id, name, color) VALUES ($1,$2,'WP32 Workspace','#ff0000')",
    )
    .bind(ws_label)
    .bind(e.d.org.0)
    .execute(&e.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO issue_label (id, organization_id, team_id, name, color) VALUES ($1,$2,$3,'Intern','#00ff00')")
        .bind(team_label)
        .bind(e.d.org.0)
        .bind(e.d.eng.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let ws = tx
        .outbox()
        .sync_model("IssueLabel", ws_label)
        .await
        .unwrap()
        .unwrap();
    assert!(ws.groups.contains(&"orgpub".to_owned()) && ws.groups.contains(&"org".to_owned()));
    let tl = tx
        .outbox()
        .sync_model("IssueLabel", team_label)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        tl.groups,
        vec![team(e.d.eng)],
        "Team-Labels sind keine Workspace-Daten"
    );
    // Personen tragen `team:`-Gruppen nie, sondern `tmem:` (nur für Gäste des Teams lesbar).
    let alice = tx
        .outbox()
        .sync_model("User", e.d.alice.0)
        .await
        .unwrap()
        .unwrap();
    assert!(
        alice.groups.iter().all(|g| !g.starts_with("team:")),
        "{:?}",
        alice.groups
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn membership_creation_refreshes_the_person_for_team_guests(pool: PgPool) {
    let e = env(pool).await;
    sqlx::query("INSERT INTO team_membership (organization_id, team_id, user_id, owner) VALUES ($1,$2,$3,false) ON CONFLICT DO NOTHING")
        .bind(e.d.org.0)
        .bind(e.d.web.0)
        .bind(e.d.alice.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        DomainEvent::TeamMembershipCreated {
            team_id: e.d.web,
            user_id: e.d.alice,
        },
    ))
    .await;
    let log = e.log().await;
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].model_name, "TeamMembership");
    let user = &log[1];
    assert_eq!(
        (user.model_name.as_str(), user.action.as_str()),
        ("User", "U")
    );
    assert!(user.groups.contains(&format!("tmem:{}", e.d.web.0)));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bootstrap_pages_cover_models_and_resume_after_the_cursor(pool: PgPool) {
    let e = env(pool).await;
    let mut tx = e.app.store.begin(e.d.org).await.unwrap();
    // Seite für Seite, bis nichts mehr kommt: jede Teamzeile genau einmal.
    let mut after = None;
    let mut seen = Vec::new();
    loop {
        let page = tx
            .outbox()
            .sync_bootstrap_page("Team", after, 2)
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        after = page.last().map(|r| r.id);
        seen.extend(page.into_iter().map(|r| r.id));
    }
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM team WHERE NOT trashed")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(seen.len() as i64, total);
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted, seen, "aufsteigend und ohne Dubletten");
    // Der Workspace selbst ist eine Zeile; unbekannte Modelle liefern nichts.
    assert_eq!(
        tx.outbox()
            .sync_bootstrap_page("Organization", None, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        tx.outbox()
            .sync_bootstrap_page("Organization", Some(Uuid::nil()), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        tx.outbox()
            .sync_bootstrap_page("Nonsense", None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    // Issues: Papierkorb bleibt draußen.
    let live = e.issue(e.d.eng, 1, 160, e.d.bob).await;
    let trashed = e.issue(e.d.eng, 1, 161, e.d.bob).await;
    sqlx::query("UPDATE issue SET trashed = true, trashed_at = now() WHERE id = $1")
        .bind(trashed.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let ids: Vec<Uuid> = tx
        .outbox()
        .sync_bootstrap_page("Issue", None, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert!(ids.contains(&live.0) && !ids.contains(&trashed.0));
}

/// Event aus der Outbox (wie vom Worker) an den Sync-Konsumenten geben.
async fn drain_outbox_into_sync(e: &Env) {
    let rows: Vec<(Uuid, String, Value, Option<Uuid>)> = sqlx::query_as(
        "SELECT event_id, origin, payload, actor_id FROM outbox_event ORDER BY sync_id, event_id",
    )
    .fetch_all(&e.pool)
    .await
    .unwrap();
    for (id, origin, payload, actor) in rows {
        let event: DomainEvent = serde_json::from_value(payload).unwrap();
        let origin = match origin.as_str() {
            "system" => EventOrigin::System,
            "import" => EventOrigin::Import,
            _ => EventOrigin::User,
        };
        let stored = StoredEvent {
            id: EventId(id),
            org_id: e.d.org,
            origin,
            actor_id: actor.map(UserId),
            sync_id: 1,
            event,
            created_at: Utc::now(),
        };
        e.handle(&stored).await;
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn removing_a_team_member_reaches_the_removed_user_via_sync(pool: PgPool) {
    let e = env(pool).await;
    // Alice (Owner) entfernt Carol (Gast) aus WEB: Carol bekommt `D TeamMembership` in `user:<id>`.
    e.app
        .remove_team_member(&ctx_for(SeedUser::Alice), "WEB", "carol")
        .await
        .unwrap();
    drain_outbox_into_sync(&e).await;
    let log = e.log().await;
    let row = log
        .iter()
        .find(|r| r.model_name == "TeamMembership" && r.action == "D")
        .expect("TeamMembership D");
    assert!(row.groups.contains(&format!("user:{}", e.d.carol.0)));
    assert_eq!(row.data["teamId"], e.d.web.0.to_string());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn favorites_sync_only_to_their_owner(pool: PgPool) {
    let e = env(pool).await;
    let ctx = ctx_for(SeedUser::Bob);
    let fav = e
        .app
        .create_favorite(
            &ctx,
            lineal_app::usecases::view::CreateFavoriteInput {
                kind: "folder".into(),
                folder_name: Some("Meine".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    e.app.delete_favorite(&ctx, fav.id).await.unwrap();
    drain_outbox_into_sync(&e).await;
    let log: Vec<_> = e
        .log()
        .await
        .into_iter()
        .filter(|r| r.model_name == "Favorite")
        .collect();
    assert_eq!(
        log.iter().map(|r| r.action.as_str()).collect::<Vec<_>>(),
        vec!["I", "D"]
    );
    for r in &log {
        assert_eq!(r.groups, vec![format!("user:{}", e.d.bob.0)]);
    }
}
