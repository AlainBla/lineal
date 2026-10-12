//! WP32: neue Notification-Typen (Kommentare an Initiativen, Dokumenten, Meilensteinen und Berichten),
//! Dokument- und View-Abos, `issueUnassignedFromYou`/`issueReopened`/`issueUnblocked` und die
//! 1-Minuten-Bündelung gegen echte Datenbank (Seed `acme`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use lineal_app::ports::repos::StoredEvent;
use lineal_app::ports::repos::notification::SubscriptionReason;
use lineal_app::ports::{Clock, Store, Tx};
use lineal_app::usecases::notification::views::{ViewIssueEvent, ViewSubscriptionSource};
use lineal_app::{App, AppResult, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::events::{DomainEvent, EventOrigin, IssuePrevious};
use lineal_domain::ids::*;
use lineal_jobs::EventConsumer;
use lineal_jobs::consumers::notifications::{
    NotificationsConsumer, NotificationsWithViewsConsumer,
};
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Uhr, die nur über `Env::advance` vorrückt.
struct ManualClock {
    base: DateTime<Utc>,
    offset: Arc<AtomicI64>,
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        self.base + Duration::seconds(self.offset.load(Ordering::SeqCst))
    }
}

struct Env {
    app: Arc<App>,
    pool: PgPool,
    d: SeedData,
    offset: Arc<AtomicI64>,
}

async fn env(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    let offset = Arc::new(AtomicI64::new(0));
    app.clock = Arc::new(ManualClock {
        base: lineal_testkit::fake::FixedClock::default().0,
        offset: offset.clone(),
    });
    Env {
        app: Arc::new(app),
        pool,
        d: SeedData::acme(),
        offset,
    }
}

fn user_ev(org: OrgId, actor: UserId, event: DomainEvent) -> StoredEvent {
    StoredEvent {
        id: EventId::new_v7(),
        org_id: org,
        origin: EventOrigin::User,
        actor_id: Some(actor),
        sync_id: 1,
        event,
        created_at: Utc::now(),
    }
}

impl Env {
    fn ev(&self, actor: UserId, event: DomainEvent) -> StoredEvent {
        user_ev(self.d.org, actor, event)
    }

    /// Verarbeitet das Event; die Uhr bleibt stehen (Ereignisse im selben Fenster).
    async fn handle(&self, e: &StoredEvent) {
        NotificationsConsumer.handle(&self.app, e).await.unwrap();
    }

    fn advance(&self, secs: i64) {
        self.offset.fetch_add(secs, Ordering::SeqCst);
    }

    async fn issue(
        &self,
        team: TeamId,
        slot: u128,
        number: i32,
        creator: UserId,
        assignee: Option<UserId>,
        state_index: usize,
    ) -> IssueId {
        let id = IssueId::new_v7();
        sqlx::query(
            "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id, assignee_id) \
             VALUES ($1,$2,$3,$4,'Login schlägt fehl',$5,$6,$7)",
        )
        .bind(id.0)
        .bind(self.d.org.0)
        .bind(team.0)
        .bind(number)
        .bind(seed_workflow_state_ids(slot)[state_index].0)
        .bind(creator.0)
        .bind(assignee.map(|a| a.0))
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    async fn set_state(&self, issue: IssueId, slot: u128, state_index: usize) {
        sqlx::query("UPDATE issue SET state_id = $2 WHERE id = $1")
            .bind(issue.0)
            .bind(seed_workflow_state_ids(slot)[state_index].0)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn subscribe(&self, issue: IssueId, user: UserId) {
        let mut tx = self.app.store.begin(self.d.org).await.unwrap();
        tx.notifications()
            .subscribe_issue(
                issue,
                user,
                None,
                SubscriptionReason::Manual,
                self.app.clock.now(),
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn project(&self, number: i32, teams: &[TeamId]) -> Uuid {
        let project = Uuid::now_v7();
        let status: Uuid = sqlx::query_scalar("SELECT id FROM project_status LIMIT 1")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO project (id, organization_id, name, lead_team_id, number, status_id, creator_id) \
             VALUES ($1,$2,'Notification Test Project',$3,$4,$5,$6)",
        )
        .bind(project)
        .bind(self.d.org.0)
        .bind(teams.first().map(|t| t.0))
        .bind(number)
        .bind(status)
        .bind(self.d.alice.0)
        .execute(&self.pool)
        .await
        .unwrap();
        for t in teams {
            sqlx::query(
                "INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1,$2,$3)",
            )
            .bind(self.d.org.0)
            .bind(project)
            .bind(t.0)
            .execute(&self.pool)
            .await
            .unwrap();
        }
        project
    }

    async fn milestone(&self, project: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO project_milestone (id, organization_id, project_id, name) VALUES ($1,$2,$3,'Beta')")
            .bind(id)
            .bind(self.d.org.0)
            .bind(project)
            .execute(&self.pool)
            .await
            .unwrap();
        id
    }

    async fn project_update(&self, project: Uuid, author: UserId) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO project_update (id, organization_id, project_id, user_id, body) VALUES ($1,$2,$3,$4,'On track')")
            .bind(id)
            .bind(self.d.org.0)
            .bind(project)
            .bind(author.0)
            .execute(&self.pool)
            .await
            .unwrap();
        id
    }

    async fn initiative(&self, number: i32) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO initiative (id, organization_id, number, name) VALUES ($1,$2,$3,'Q4 Ziele')")
            .bind(id)
            .bind(self.d.org.0)
            .bind(number)
            .execute(&self.pool)
            .await
            .unwrap();
        id
    }

    async fn initiative_update(&self, initiative: Uuid, author: UserId) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO initiative_update (id, organization_id, initiative_id, user_id, body) VALUES ($1,$2,$3,$4,'Läuft')")
            .bind(id)
            .bind(self.d.org.0)
            .bind(initiative)
            .bind(author.0)
            .execute(&self.pool)
            .await
            .unwrap();
        id
    }

    /// Dokument am Projekt (`project`), Team (`team`) oder an der Initiative.
    async fn document(
        &self,
        project: Option<Uuid>,
        team: Option<TeamId>,
        initiative: Option<Uuid>,
        creator: UserId,
    ) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO document (id, organization_id, title, project_id, team_id, initiative_id, creator_id) \
             VALUES ($1,$2,'Spezifikation',$3,$4,$5,$6)",
        )
        .bind(id)
        .bind(self.d.org.0)
        .bind(project)
        .bind(team.map(|t| t.0))
        .bind(initiative)
        .bind(creator.0)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    /// Kommentar an einem Elternobjekt; `column` ist die Spalte (`issue_id`, `document_id` …).
    async fn comment_on(
        &self,
        column: &str,
        parent: Uuid,
        author: UserId,
        reply_to: Option<CommentId>,
    ) -> CommentId {
        let id = CommentId::new_v7();
        sqlx::query(&format!(
            "INSERT INTO comment (id, organization_id, body, user_id, {column}, parent_id) \
             VALUES ($1,$2,'Bitte prüfen',$3,$4,$5)"
        ))
        .bind(id.0)
        .bind(self.d.org.0)
        .bind(author.0)
        .bind(parent)
        .bind(reply_to.map(|p| p.0))
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    async fn subscribe_object(&self, user: SeedUser, kind: &str, target: Uuid, types: &[&str]) {
        use lineal_app::ports::repos::notification::ObjectKind;
        use lineal_app::usecases::notification::SetObjectSubscriptionInput;
        let kind = match kind {
            "project" => ObjectKind::Project,
            "initiative" => ObjectKind::Initiative,
            other => panic!("unexpected {other}"),
        };
        self.app
            .set_notification_subscription(
                &ctx_for(user),
                SetObjectSubscriptionInput {
                    kind,
                    target,
                    context_view_type: None,
                    types: Some(types.iter().map(|t| (*t).to_owned()).collect()),
                    include_sub_initiative_updates: None,
                    active: None,
                },
            )
            .await
            .unwrap();
    }

    async fn kinds(&self, user: UserId) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT type FROM notification WHERE user_id = $1 ORDER BY created_at, id",
        )
        .bind(user.0)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    async fn count_events(&self, kind: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM outbox_event WHERE payload->>'type' = $1")
            .bind(kind)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn comment_created(c: CommentId, mentioned: Vec<UserId>) -> DomainEvent {
    DomainEvent::CommentCreated {
        comment_id: c,
        mentioned_user_ids: mentioned,
    }
}

// --- Kommentare an weiteren Objekten -------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn milestone_comments_notify_project_subscribers_and_mentions(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let project = e.project(120, &[e.d.eng]).await;
    let milestone = e.milestone(project).await;
    e.subscribe_object(SeedUser::Bob, "project", project, &["projectNewComment"])
        .await;
    let c = e
        .comment_on("project_milestone_id", milestone, alice, None)
        .await;
    e.handle(&e.ev(alice, comment_created(c, vec![]))).await;
    assert_eq!(e.kinds(bob).await, vec!["projectMilestoneNewComment"]);
    let n: (String, Option<Uuid>, Option<Uuid>, String) = sqlx::query_as(
        "SELECT target_type, project_id, project_milestone_id, group_key FROM notification WHERE user_id = $1",
    )
    .bind(bob.0)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(n.0, "project");
    assert_eq!(n.1, Some(project));
    assert_eq!(n.2, Some(milestone), "Meilenstein bleibt erkennbar");
    assert_eq!(
        n.3,
        format!("project:{project}"),
        "gruppiert unter dem Projekt"
    );

    // Erwähnung: spezifischerer Typ, nach Ablauf des Bündelfensters als eigene Notification.
    e.advance(120);
    let c2 = e
        .comment_on("project_milestone_id", milestone, alice, None)
        .await;
    e.handle(&e.ev(alice, comment_created(c2, vec![bob]))).await;
    assert_eq!(
        e.kinds(bob).await,
        vec![
            "projectMilestoneNewComment",
            "projectMilestoneCommentMention"
        ]
    );
    assert_eq!(e.kinds(alice).await.len(), 0);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_update_comment_notifies_the_report_author_and_thread_participants(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let project = e.project(121, &[e.d.eng]).await;
    let update = e.project_update(project, alice).await;
    let c = e.comment_on("project_update_id", update, bob, None).await;
    e.handle(&e.ev(bob, comment_created(c, vec![]))).await;
    assert_eq!(e.kinds(alice).await, vec!["projectUpdateNewComment"]);
    assert_eq!(e.kinds(bob).await.len(), 0, "Autor des Kommentars: keine");
    let row: (Option<Uuid>,) =
        sqlx::query_as("SELECT project_update_id FROM notification WHERE user_id = $1")
            .bind(alice.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(row.0, Some(update));

    // Antwort von Alice: Bob (Thread-Teilnehmer) erfährt davon.
    e.advance(120);
    let reply = e
        .comment_on("project_update_id", update, alice, Some(c))
        .await;
    e.handle(&e.ev(alice, comment_created(reply, vec![]))).await;
    assert_eq!(e.kinds(bob).await, vec!["projectUpdateNewComment"]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn initiative_and_initiative_update_comments_exclude_guests(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob, carol) = (e.d.alice, e.d.bob, e.d.carol);
    let initiative = e.initiative(101).await;
    e.subscribe_object(
        SeedUser::Bob,
        "initiative",
        initiative,
        &["initiativeNewComment"],
    )
    .await;
    let c = e.comment_on("initiative_id", initiative, alice, None).await;
    // Carol (Gast) ist erwähnt, sieht aber keine Initiativen: nichts.
    e.handle(&e.ev(alice, comment_created(c, vec![carol])))
        .await;
    assert_eq!(e.kinds(bob).await, vec!["initiativeNewComment"]);
    assert!(
        e.kinds(carol).await.is_empty(),
        "Gäste sehen keine Initiativen"
    );

    e.advance(120);
    let update = e.initiative_update(initiative, bob).await;
    let uc = e
        .comment_on("initiative_update_id", update, alice, None)
        .await;
    e.handle(&e.ev(alice, comment_created(uc, vec![]))).await;
    assert!(
        e.kinds(bob)
            .await
            .contains(&"initiativeUpdateNewComment".to_owned())
    );
    let group: String = sqlx::query_scalar(
        "SELECT group_key FROM notification WHERE user_id = $1 AND type = 'initiativeUpdateNewComment'",
    )
    .bind(bob.0)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(group, format!("initiative:{initiative}"));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn document_comments_notify_subscribers_and_respect_visibility(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob, carol) = (e.d.alice, e.d.bob, e.d.carol);
    // Team-Dokument in ENG: Carol (Gast, nur WEB) sieht es nicht.
    let doc = e.document(None, Some(e.d.eng), None, bob).await;
    // Der Ersteller abonniert (wie der Use-Case), Carol per SQL – sie darf nichts erhalten.
    {
        let mut tx = e.app.store.begin(e.d.org).await.unwrap();
        for u in [bob, carol] {
            tx.notifications()
                .subscribe_document(doc, u, SubscriptionReason::Creator, e.app.clock.now())
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
    }
    let c = e.comment_on("document_id", doc, alice, None).await;
    e.handle(&e.ev(alice, comment_created(c, vec![]))).await;
    assert_eq!(e.kinds(bob).await, vec!["documentNewComment"]);
    assert!(e.kinds(carol).await.is_empty(), "Gast ohne Zugriff auf ENG");
    // Alice kommentierte: sie ist nun Abonnentin (reason comment), aber ohne Selbst-Notification.
    assert!(e.kinds(alice).await.is_empty());
    let reasons: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM document_subscription WHERE user_id = $1 AND document_id = $2",
    )
    .bind(alice.0)
    .bind(doc)
    .fetch_all(&e.pool)
    .await
    .unwrap();
    assert_eq!(reasons, vec!["comment"]);

    // Erwähnung eines Users ohne Zugriff erzeugt weder Notification noch Abo.
    e.advance(120);
    let c2 = e.comment_on("document_id", doc, alice, None).await;
    e.handle(&e.ev(alice, comment_created(c2, vec![carol])))
        .await;
    assert!(e.kinds(carol).await.is_empty());
    let subs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM document_subscription WHERE user_id = $1")
            .bind(carol.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(subs, 1, "nur das per SQL angelegte Abo, kein neues");
    // Aber: Erwähnung von Bob (sieht ENG) → documentCommentMention.
    e.advance(120);
    let c3 = e.comment_on("document_id", doc, alice, None).await;
    e.handle(&e.ev(alice, comment_created(c3, vec![bob]))).await;
    assert_eq!(
        e.kinds(bob).await,
        vec![
            "documentNewComment",
            "documentNewComment",
            "documentCommentMention"
        ]
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn document_subscribe_use_cases_follow_visibility(pool: PgPool) {
    let e = env(pool).await;
    let doc = e.document(None, Some(e.d.eng), None, e.d.alice).await;
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);

    assert!(!e.app.is_document_subscribed(&bob, doc).await.unwrap());
    let r = e.app.subscribe_document(&bob, doc).await.unwrap();
    assert!(r.subscribed && r.changed);
    assert!(e.app.is_document_subscribed(&bob, doc).await.unwrap());
    assert!(
        !e.app.subscribe_document(&bob, doc).await.unwrap().changed,
        "idempotent"
    );
    let r = e.app.unsubscribe_document(&bob, doc).await.unwrap();
    assert!(!r.subscribed && r.changed);
    assert!(!e.app.is_document_subscribed(&bob, doc).await.unwrap());
    // Manuelles Abonnieren reaktiviert.
    assert!(e.app.subscribe_document(&bob, doc).await.unwrap().changed);

    // Carol (Gast, kein ENG): not_found, nie forbidden.
    for res in [
        e.app.subscribe_document(&carol, doc).await.map(|_| ()),
        e.app.unsubscribe_document(&carol, doc).await.map(|_| ()),
        e.app.is_document_subscribed(&carol, doc).await.map(|_| ()),
    ] {
        assert_eq!(res.unwrap_err().kind, ErrorKind::NotFound);
    }
    // Unbekanntes Dokument und Lese-Token.
    assert_eq!(
        e.app
            .subscribe_document(&bob, Uuid::now_v7())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    let mut ro = ctx_for(SeedUser::Bob);
    ro.scopes.write = false;
    assert_eq!(
        e.app.subscribe_document(&ro, doc).await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
}

// --- Übergänge: alte Werte im Event ------------------------------------------------------------

fn updated(
    issue: IssueId,
    team: TeamId,
    changed: &[&str],
    previous: Option<IssuePrevious>,
) -> DomainEvent {
    DomainEvent::IssueUpdated {
        previous,
        issue_id: issue,
        team_id: team,
        changed: changed.iter().map(|c| (*c).to_owned()).collect(),
        mentioned_user_ids: vec![],
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn reassignment_tells_the_previous_assignee(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    // Vorher Bob, jetzt Alice zugewiesen; Carol-unabhängig.
    let issue = e.issue(e.d.eng, 1, 951, alice, Some(alice), 1).await;
    e.handle(&e.ev(
        alice,
        updated(
            issue,
            e.d.eng,
            &["assigneeId"],
            Some(IssuePrevious {
                assignee_id: Some(bob),
                state_type: None,
            }),
        ),
    ))
    .await;
    // Alice ist Akteurin (keine Selbst-Notification); Bob verliert die Zuweisung.
    assert_eq!(e.kinds(bob).await, vec!["issueUnassignedFromYou"]);
    assert!(e.kinds(alice).await.is_empty());

    // Zuweisung entfernt (kein neuer Assignee): ebenfalls `issueUnassignedFromYou`.
    e.advance(120);
    let issue2 = e.issue(e.d.eng, 1, 952, alice, None, 1).await;
    e.handle(&e.ev(
        alice,
        updated(
            issue2,
            e.d.eng,
            &["assigneeId"],
            Some(IssuePrevious {
                assignee_id: Some(bob),
                state_type: None,
            }),
        ),
    ))
    .await;
    assert_eq!(e.kinds(bob).await.len(), 2);
    // Ohne erfasste alte Werte: nichts (Altdaten, Systemänderungen).
    e.advance(120);
    let issue3 = e.issue(e.d.eng, 1, 953, alice, None, 1).await;
    e.handle(&e.ev(alice, updated(issue3, e.d.eng, &["assigneeId"], None)))
        .await;
    assert_eq!(e.kinds(bob).await.len(), 2);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn reopening_notifies_subscribers_only_from_closed_to_open(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    // Zustand 3 = Done; Bob abonniert.
    let issue = e.issue(e.d.eng, 1, 954, alice, None, 1).await;
    e.subscribe(issue, bob).await;
    // Done → Todo: wieder geöffnet.
    e.handle(&e.ev(
        alice,
        updated(
            issue,
            e.d.eng,
            &["stateId"],
            Some(IssuePrevious {
                assignee_id: None,
                state_type: Some("completed".into()),
            }),
        ),
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueReopened"]);
    // Todo → Started: keine Wiedereröffnung.
    e.advance(120);
    e.handle(&e.ev(
        alice,
        updated(
            issue,
            e.d.eng,
            &["stateId"],
            Some(IssuePrevious {
                assignee_id: None,
                state_type: Some("unstarted".into()),
            }),
        ),
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueReopened"]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn closing_the_last_blocker_unblocks_the_blocked_issue(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let blocker1 = e.issue(e.d.eng, 1, 955, alice, None, 1).await;
    let blocker2 = e.issue(e.d.eng, 1, 956, alice, None, 1).await;
    let blocked = e.issue(e.d.eng, 1, 957, alice, Some(bob), 1).await;
    for b in [blocker1, blocker2] {
        sqlx::query(
            "INSERT INTO issue_relation (organization_id, issue_id, related_issue_id, type) VALUES ($1,$2,$3,'blocks')",
        )
        .bind(e.d.org.0)
        .bind(b.0)
        .bind(blocked.0)
        .execute(&e.pool)
        .await
        .unwrap();
    }
    // Erster Blocker erledigt: noch ein offener → nichts.
    e.set_state(blocker1, 1, 3).await;
    e.handle(&e.ev(
        alice,
        updated(
            blocker1,
            e.d.eng,
            &["stateId"],
            Some(IssuePrevious {
                assignee_id: None,
                state_type: Some("unstarted".into()),
            }),
        ),
    ))
    .await;
    assert!(e.kinds(bob).await.is_empty());
    // Zweiter erledigt: der Assignee des blockierten Issues erfährt es.
    e.set_state(blocker2, 1, 3).await;
    e.handle(&e.ev(
        alice,
        updated(
            blocker2,
            e.d.eng,
            &["stateId"],
            Some(IssuePrevious {
                assignee_id: None,
                state_type: Some("unstarted".into()),
            }),
        ),
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueUnblocked"]);
    let issue_id: Uuid = sqlx::query_scalar("SELECT issue_id FROM notification WHERE user_id = $1")
        .bind(bob.0)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(issue_id, blocked.0, "Bezug ist das blockierte Issue");
}

// --- Bündelung ----------------------------------------------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn events_of_the_same_actor_within_a_minute_become_one_notification(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 960, alice, Some(bob), 1).await;
    e.subscribe(issue, bob).await;

    // 1. Zuweisung (spezifischer), 2. Kommentar, 3. Statuswechsel nach Done – alles von Alice.
    let created = e.ev(
        alice,
        DomainEvent::IssueCreated {
            issue_id: issue,
            team_id: e.d.eng,
            mentioned_user_ids: vec![],
        },
    );
    e.handle(&created).await;
    let c = e.comment_on("issue_id", issue.0, alice, None).await;
    let comment_ev = e.ev(alice, comment_created(c, vec![]));
    e.advance(20);
    e.handle(&comment_ev).await;
    e.set_state(issue, 1, 3).await;
    e.advance(20);
    e.handle(&e.ev(
        alice,
        updated(
            issue,
            e.d.eng,
            &["stateId"],
            Some(IssuePrevious {
                assignee_id: None,
                state_type: Some("unstarted".into()),
            }),
        ),
    ))
    .await;

    assert_eq!(e.kinds(bob).await, vec!["issueAssignedToYou"]);
    let (types, comment): (Vec<String>, Option<Uuid>) =
        sqlx::query_as("SELECT bundled_types, comment_id FROM notification WHERE user_id = $1")
            .bind(bob.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(
        types,
        vec![
            "issueAssignedToYou",
            "issueNewComment",
            "issueStatusChanged"
        ],
        "alle Ereignisse, spezifischster Typ vorn in `type`"
    );
    assert_eq!(comment, None, "der Haupttyp behält seine Bezüge");
    // Jede Zusammenführung meldet ein Update für den Sync-Strom.
    assert_eq!(e.count_events("NotificationUpdated").await, 2);
    assert_eq!(e.count_events("NotificationCreated").await, 1);

    // Wiederholte Zustellung (at-least-once) ändert nichts mehr.
    e.handle(&comment_ev).await;
    e.handle(&created).await;
    let types: Vec<String> =
        sqlx::query_scalar("SELECT bundled_types FROM notification WHERE user_id = $1")
            .bind(bob.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(types.len(), 3);
    assert_eq!(e.kinds(bob).await.len(), 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bundling_resets_read_state_and_respects_window_and_actor(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob, carol) = (e.d.alice, e.d.bob, e.d.carol);
    let _ = carol;
    let issue = e.issue(e.d.eng, 1, 961, alice, None, 1).await;
    e.subscribe(issue, bob).await;
    let c1 = e.comment_on("issue_id", issue.0, alice, None).await;
    e.handle(&e.ev(alice, comment_created(c1, vec![]))).await;
    // Bob liest.
    sqlx::query("UPDATE notification SET read_at = $2 WHERE user_id = $1")
        .bind(bob.0)
        .bind(Utc::now())
        .execute(&e.pool)
        .await
        .unwrap();
    // Zweiter Kommentar nach 30 s: gleiche Notification, wieder ungelesen.
    e.advance(30);
    let c2 = e.comment_on("issue_id", issue.0, alice, None).await;
    e.handle(&e.ev(alice, comment_created(c2, vec![]))).await;
    assert_eq!(e.kinds(bob).await, vec!["issueNewComment"]);
    let read: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT read_at FROM notification WHERE user_id = $1")
            .bind(bob.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(read.is_none(), "neue Aktivität macht die Gruppe ungelesen");

    // Anderer Akteur (Bob → Alice ist keine Selbst-Sache): Kommentar von Bob an Alice-Abo.
    e.subscribe(issue, alice).await;
    let c3 = e.comment_on("issue_id", issue.0, bob, None).await;
    e.handle(&e.ev(bob, comment_created(c3, vec![]))).await;
    assert_eq!(e.kinds(alice).await, vec!["issueNewComment"]);
    // Alice kommentiert wieder: für Bob neu (anderer Akteur als Bobs vorherige Notification? gleiche
    // Alice, aber > 60 s später) → eigene Notification.
    e.advance(61);
    let c4 = e.comment_on("issue_id", issue.0, alice, None).await;
    e.handle(&e.ev(alice, comment_created(c4, vec![]))).await;
    assert_eq!(e.kinds(bob).await.len(), 2, "außerhalb des Fensters");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn system_events_are_never_bundled(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 962, alice, Some(bob), 1).await;
    e.subscribe(issue, bob).await;
    let mut sys = e.ev(alice, DomainEvent::IssueSlaHighRisk { issue_id: issue });
    sys.origin = EventOrigin::System;
    sys.actor_id = None;
    e.handle(&sys).await;
    let mut sys2 = e.ev(alice, DomainEvent::IssueSlaBreached { issue_id: issue });
    sys2.origin = EventOrigin::System;
    sys2.actor_id = None;
    e.handle(&sys2).await;
    assert_eq!(e.kinds(bob).await.len(), 2, "ohne Akteur keine Bündelung");
}

// --- View-Abos ----------------------------------------------------------------------------------

struct FakeViews {
    recipients: Vec<(UserId, &'static str)>,
    seen: Mutex<Vec<(bool, Vec<String>)>>,
}

#[async_trait]
impl ViewSubscriptionSource for FakeViews {
    async fn recipients(
        &self,
        _tx: &mut dyn Tx,
        event: &ViewIssueEvent<'_>,
    ) -> AppResult<Vec<(UserId, &'static str)>> {
        self.seen
            .lock()
            .unwrap()
            .push((event.is_new, event.changed.to_vec()));
        Ok(self.recipients.clone())
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn view_subscribers_get_notifications_with_visibility_and_self_suppression(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob, carol) = (e.d.alice, e.d.bob, e.d.carol);
    let views = Arc::new(FakeViews {
        // Alice ist die Akteurin, Carol (Gast) sieht ENG nicht.
        recipients: vec![
            (bob, "issueAddedToView"),
            (alice, "issueAddedToView"),
            (carol, "issueAddedToView"),
            (bob, "issueStatusChangedAll"),
        ],
        seen: Mutex::new(vec![]),
    });
    let consumer = NotificationsWithViewsConsumer {
        views: views.clone(),
    };
    let issue = e.issue(e.d.eng, 1, 970, alice, None, 1).await;
    let ev = e.ev(
        alice,
        DomainEvent::IssueCreated {
            issue_id: issue,
            team_id: e.d.eng,
            mentioned_user_ids: vec![],
        },
    );
    consumer.handle(&e.app, &ev).await.unwrap();
    // Bob: spezifischster Typ (Abo-Typen gleichrangig → erster bleibt), Alice/Carol nichts.
    assert_eq!(e.kinds(bob).await.len(), 1);
    assert!(e.kinds(alice).await.is_empty());
    assert!(e.kinds(carol).await.is_empty());
    assert_eq!(views.seen.lock().unwrap()[0], (true, vec![]));

    // Änderung: die Quelle erhält die geänderten Felder.
    e.advance(120);
    consumer
        .handle(
            &e.app,
            &e.ev(alice, updated(issue, e.d.eng, &["stateId"], None)),
        )
        .await
        .unwrap();
    assert_eq!(
        views.seen.lock().unwrap()[1],
        (false, vec!["stateId".to_owned()])
    );
    assert_eq!(e.kinds(bob).await.len(), 2);

    // Ohne Quelle (Standard-Konsument) entstehen keine View-Benachrichtigungen.
    e.advance(120);
    let issue2 = e.issue(e.d.eng, 1, 971, alice, None, 1).await;
    e.handle(&e.ev(
        alice,
        DomainEvent::IssueCreated {
            issue_id: issue2,
            team_id: e.d.eng,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.kinds(bob).await.len(), 2);
}
