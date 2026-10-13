//! WP08: Fan-out, Inbox-Aktionen, Abos, Einstellungen und Jobs gegen echte Datenbank (Seed `acme`).
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Issues und Kommentare entstehen hier per SQL (WP06/WP09 liefern die Use-Cases erst parallel).

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::repos::notification::SubscriptionReason;
use lineal_app::ports::repos::{NewEvent, StoredEvent};
use lineal_app::ports::{Clock, Mail, Mailer, Store};
use lineal_app::usecases::notification::{
    ListNotificationsInput, MarkNotificationInput, NotificationSettingsPatch,
    SetObjectSubscriptionInput,
};
use lineal_app::{App, AppResult, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::*;
use lineal_jobs::consumers::notifications::NotificationsConsumer;
use lineal_jobs::periodic::{
    due_reminders::DueRemindersJob, notification_cap::NotificationCapJob, snooze::SnoozeJob,
};
use lineal_jobs::{EventConsumer, PeriodicJob};
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Default)]
struct RecordingMailer(Mutex<Vec<Mail>>);

#[async_trait]
impl Mailer for RecordingMailer {
    async fn send(&self, mail: Mail) -> AppResult<()> {
        self.0.lock().unwrap().push(mail);
        Ok(())
    }
}

/// Uhr, die nur über `Env::advance` vorrückt (Basis wie `FixedClock::default()`).
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
    mails: Arc<RecordingMailer>,
    /// Sekunden, die `handle` nach jedem Event vorrückt (nur bei der Standard-Uhr): Ereignisse
    /// liegen dann außerhalb des Bündelungsfensters von einer Minute (docs/05 §6.6).
    tick: Option<Arc<AtomicI64>>,
}

fn t(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap()
}

async fn env_at(pool: PgPool, clock: Option<DateTime<Utc>>) -> Env {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mails = Arc::new(RecordingMailer::default());
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.mailer = mails.clone();
    let tick = match clock {
        Some(c) => {
            app.clock = Arc::new(lineal_testkit::fake::FixedClock(c));
            None
        }
        None => {
            let offset = Arc::new(AtomicI64::new(0));
            app.clock = Arc::new(ManualClock {
                base: lineal_testkit::fake::FixedClock::default().0,
                offset: offset.clone(),
            });
            Some(offset)
        }
    };
    Env {
        app: Arc::new(app),
        pool,
        d: SeedData::acme(),
        mails,
        tick,
    }
}

async fn env(pool: PgPool) -> Env {
    env_at(pool, None).await
}

fn ev(org: OrgId, origin: EventOrigin, actor: Option<UserId>, event: DomainEvent) -> StoredEvent {
    StoredEvent {
        id: EventId::new_v7(),
        org_id: org,
        origin,
        actor_id: actor,
        sync_id: 1,
        event,
        created_at: Utc::now(),
    }
}

impl Env {
    fn user_ev(&self, actor: UserId, event: DomainEvent) -> StoredEvent {
        ev(self.d.org, EventOrigin::User, Some(actor), event)
    }

    async fn handle(&self, e: &StoredEvent) {
        NotificationsConsumer.handle(&self.app, e).await.unwrap();
        self.advance(120);
    }

    /// Rückt die Standard-Uhr vor (No-op bei fester Uhr).
    fn advance(&self, secs: i64) {
        if let Some(t) = &self.tick {
            t.fetch_add(secs, Ordering::SeqCst);
        }
    }

    /// Issue per SQL; `slot` 1 = ENG, 2 = WEB, 3 = DES; Status-Index 1 = Todo, 3 = Done.
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

    async fn comment(
        &self,
        issue: IssueId,
        author: UserId,
        parent: Option<CommentId>,
    ) -> CommentId {
        let id = CommentId::new_v7();
        sqlx::query(
            "INSERT INTO comment (id, organization_id, body, user_id, issue_id, parent_id) \
             VALUES ($1,$2,'Kannst du dir die Logs ansehen?',$3,$4,$5)",
        )
        .bind(id.0)
        .bind(self.d.org.0)
        .bind(author.0)
        .bind(issue.0)
        .bind(parent.map(|p| p.0))
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    async fn subscribe(&self, issue: IssueId, user: UserId) {
        let mut tx = self.app.store.begin(self.d.org).await.unwrap();
        tx.notifications()
            .subscribe_issue(issue, user, None, SubscriptionReason::Manual, Utc::now())
            .await
            .unwrap();
        tx.commit().await.unwrap();
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

    async fn count(&self, user: UserId) -> usize {
        self.kinds(user).await.len()
    }

    async fn first_notification(&self, user: UserId) -> NotificationId {
        let id: Uuid = sqlx::query_scalar(
            "SELECT id FROM notification WHERE user_id = $1 ORDER BY created_at, id LIMIT 1",
        )
        .bind(user.0)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        NotificationId(id)
    }
}

fn eng_issue_created(issue: IssueId, d: &SeedData, mentioned: Vec<UserId>) -> DomainEvent {
    DomainEvent::IssueCreated {
        issue_id: issue,
        team_id: d.eng,
        mentioned_user_ids: mentioned,
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn mention_in_comment_notifies_only_the_mentioned_user_once(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob, carol) = (e.d.alice, e.d.bob, e.d.carol);
    // Bob legt das Issue an und ist damit Subscriber; Carol (Gast, nur WEB) ist per SQL Subscriber.
    let issue = e.issue(e.d.eng, 1, 901, bob, None, 1).await;
    e.handle(&e.user_ev(bob, eng_issue_created(issue, &e.d, vec![])))
        .await;
    e.subscribe(issue, carol).await;
    assert_eq!(
        e.count(bob).await,
        0,
        "Ersteller erhält nichts über die eigene Anlage"
    );

    let comment = e.comment(issue, alice, None).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![bob],
        },
    ))
    .await;

    // Bob ist Subscriber UND erwähnt: genau eine Notification mit dem spezifischsten Typ.
    assert_eq!(e.kinds(bob).await, vec!["issueCommentMention"]);
    assert_eq!(
        e.count(alice).await,
        0,
        "der Autor erhält keine Notification"
    );
    assert_eq!(e.count(carol).await, 0, "Carol sieht ENG nicht");
    // Der Kommentierende wurde Subscriber (reason comment).
    let reasons: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM issue_subscription WHERE user_id = $1 AND issue_id = $2",
    )
    .bind(alice.0)
    .bind(issue.0)
    .fetch_all(&e.pool)
    .await
    .unwrap();
    assert_eq!(reasons, vec!["comment"]);

    // Idempotenz: dasselbe Event noch einmal erzeugt nichts Neues.
    let again = e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![bob],
        },
    );
    e.handle(&again).await;
    e.handle(&again).await;
    assert_eq!(
        e.count(bob).await,
        2,
        "zweites Event = zweite Notification, Wiederholung nicht"
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn event_redelivery_is_idempotent(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 902, e.d.alice, Some(e.d.bob), 1).await;
    let event = e.user_ev(e.d.alice, eng_issue_created(issue, &e.d, vec![]));
    e.handle(&event).await;
    e.handle(&event).await;
    assert_eq!(e.kinds(e.d.bob).await, vec!["issueAssignedToYou"]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn assignee_change_notifies_new_assignee_and_subscribes_them(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 903, e.d.alice, Some(e.d.bob), 1).await;
    e.handle(&e.user_ev(
        e.d.alice,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: issue,
            team_id: e.d.eng,
            changed: vec!["assigneeId".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.kinds(e.d.bob).await, vec!["issueAssignedToYou"]);
    assert_eq!(e.count(e.d.alice).await, 0);
    let active: bool = sqlx::query_scalar(
        "SELECT active FROM issue_subscription WHERE user_id = $1 AND issue_id = $2",
    )
    .bind(e.d.bob.0)
    .bind(issue.0)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert!(active);

    // Selbstzuweisung: Akteur = Assignee → keine Notification (Self-Suppression).
    let own = e.issue(e.d.eng, 1, 904, e.d.alice, Some(e.d.bob), 1).await;
    e.handle(&e.user_ev(
        e.d.bob,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: own,
            team_id: e.d.eng,
            changed: vec!["assigneeId".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.count(e.d.bob).await, 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn import_events_create_no_notifications(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 905, e.d.alice, Some(e.d.bob), 1).await;
    let comment = e.comment(issue, e.d.alice, None).await;
    for event in [
        eng_issue_created(issue, &e.d, vec![e.d.bob]),
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![e.d.bob],
        },
    ] {
        e.handle(&ev(e.d.org, EventOrigin::Import, Some(e.d.alice), event))
            .await;
    }
    assert_eq!(e.count(e.d.bob).await, 0);
    let subs: i64 = sqlx::query_scalar("SELECT count(*) FROM issue_subscription")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(subs, 0, "auch keine Abos aus Importen");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_team_issue_notifies_only_members(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    // DES ist privat: nur Bob ist Mitglied; Alice (Owner, nicht Mitglied) sieht es nicht.
    let issue = e.issue(e.d.des, 3, 906, bob, Some(alice), 1).await;
    e.subscribe(issue, alice).await;
    e.subscribe(issue, bob).await;
    let comment = e.comment(issue, bob, None).await;
    // Bob erwähnt Alice und sich selbst (Self-Suppression für Bob, Sichtbarkeit für Alice).
    e.handle(&e.user_ev(
        bob,
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![alice, bob],
        },
    ))
    .await;
    assert_eq!(e.count(alice).await, 0);
    assert_eq!(e.count(bob).await, 0);
    // Zuweisung an Alice im privaten Team: ebenfalls verworfen, und kein Abo für Alice entsteht neu.
    e.handle(&e.user_ev(
        bob,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: issue,
            team_id: e.d.des,
            changed: vec!["assigneeId".into()],
            mentioned_user_ids: vec![alice],
        },
    ))
    .await;
    assert_eq!(e.count(alice).await, 0);

    // Positivkontrolle: Systemereignis (kein Akteur) erreicht das Mitglied Bob, nicht Alice.
    let sys_comment = e.comment(issue, alice, None).await;
    e.handle(&ev(
        e.d.org,
        EventOrigin::System,
        None,
        DomainEvent::CommentCreated {
            comment_id: sys_comment,
            mentioned_user_ids: vec![bob, alice],
        },
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueCommentMention"]);
    assert_eq!(e.count(alice).await, 0);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn inbox_read_snooze_archive_flow(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 907, alice, Some(bob), 1).await;
    e.handle(&e.user_ev(alice, eng_issue_created(issue, &e.d, vec![])))
        .await;
    let c1 = e.comment(issue, alice, None).await;
    e.subscribe(issue, bob).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: c1,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(
        e.kinds(bob).await,
        vec!["issueAssignedToYou", "issueNewComment"]
    );

    let ctx = ctx_for(SeedUser::Bob);
    let list = e
        .app
        .list_notifications(
            &ctx,
            ListNotificationsInput {
                page: Default::default(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(list.items.len(), 2);
    assert_eq!(
        list.items[0].view.n.kind, "issueNewComment",
        "neueste zuerst"
    );
    assert_eq!(list.items[0].title, "Login schlägt fehl");
    assert!(list.items[0].url.contains("/acme/issue/ENG-907"));
    assert!(list.items[0].subtitle.contains("commented on ENG-907"));
    assert_eq!(
        e.app.unread_notification_count(&ctx).await.unwrap(),
        1,
        "eine Gruppe"
    );

    // gelesen / ungelesen (gilt für die ganze Gruppe, docs/05 §6.7)
    let id = list.items[0].view.n.id;
    let read = e
        .app
        .mark_notification(
            &ctx,
            id,
            MarkNotificationInput {
                read: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(read.view.n.read_at.is_some());
    let unread_only = ListNotificationsInput {
        unread_only: true,
        ..Default::default()
    };
    assert_eq!(
        e.app
            .list_notifications(&ctx, unread_only.clone())
            .await
            .unwrap()
            .items
            .len(),
        0,
        "die ganze Gruppe ist gelesen"
    );
    let back = e
        .app
        .mark_notification(
            &ctx,
            id,
            MarkNotificationInput {
                read: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(back.view.n.read_at.is_none());
    assert_eq!(
        e.app
            .list_notifications(&ctx, unread_only.clone())
            .await
            .unwrap()
            .items
            .len(),
        2
    );

    // Öffnen liest die ganze Gruppe.
    e.app.open_notification(&ctx, id).await.unwrap();
    assert!(
        e.app
            .list_notifications(&ctx, unread_only)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 0);

    // Validierung
    let err = e
        .app
        .mark_notification(&ctx, id, MarkNotificationInput::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    let past = e.app.clock.now() - Duration::hours(1);
    let err = e
        .app
        .mark_notification(
            &ctx,
            id,
            MarkNotificationInput {
                snoozed_until_at: Some(Some(past)),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);

    // Fremde Notifications → not_found.
    let alice_ctx = ctx_for(SeedUser::Alice);
    let err = e.app.get_notification(&alice_ctx, id).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    let err = e
        .app
        .archive_notification(&alice_ctx, id, true)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);

    // Archivieren (Gruppe) und Undo.
    e.app.archive_notification(&ctx, id, true).await.unwrap();
    let all = ListNotificationsInput::default();
    assert!(
        e.app
            .list_notifications(&ctx, all.clone())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let with_archived = ListNotificationsInput {
        include_archived: true,
        ..Default::default()
    };
    assert_eq!(
        e.app
            .list_notifications(&ctx, with_archived)
            .await
            .unwrap()
            .items
            .len(),
        2
    );
    e.app.archive_notification(&ctx, id, false).await.unwrap();
    assert_eq!(
        e.app
            .list_notifications(&ctx, all)
            .await
            .unwrap()
            .items
            .len(),
        2
    );

    // mark all read + archive all.
    sqlx::query("UPDATE notification SET read_at = NULL")
        .execute(&e.pool)
        .await
        .unwrap();
    let count_events = |kind: &'static str| {
        let pool = e.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM outbox_event WHERE payload->>'type' = $1",
            )
            .bind(kind)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let (updated_before, archived_before) = (
        count_events("NotificationUpdated").await,
        count_events("NotificationArchived").await,
    );
    assert_eq!(
        e.app.mark_all_notifications_read(&ctx, None).await.unwrap(),
        2
    );
    // Bulk-Aktionen erzeugen je Notification ein Event (Grundlage der Sync-Aktionen).
    assert_eq!(
        count_events("NotificationUpdated").await,
        updated_before + 2
    );
    assert_eq!(
        e.app
            .archive_all_notifications(&ctx, Some("read"))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        count_events("NotificationArchived").await,
        archived_before + 2
    );
    assert_eq!(
        e.app
            .archive_all_notifications(&ctx, Some("all"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        count_events("NotificationArchived").await,
        archived_before + 2,
        "ohne Treffer keine Events"
    );
    let err = e
        .app
        .archive_all_notifications(&ctx, Some("bogus"))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn archive_all_completed_only_archives_closed_issues(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let open = e.issue(e.d.eng, 1, 908, alice, Some(bob), 1).await;
    let done = e.issue(e.d.eng, 1, 909, alice, Some(bob), 3).await;
    for i in [open, done] {
        e.handle(&e.user_ev(alice, eng_issue_created(i, &e.d, vec![])))
            .await;
    }
    let ctx = ctx_for(SeedUser::Bob);
    assert_eq!(
        e.app
            .archive_all_notifications(&ctx, Some("completed"))
            .await
            .unwrap(),
        1
    );
    let left = e
        .app
        .list_notifications(&ctx, Default::default())
        .await
        .unwrap();
    assert_eq!(left.items.len(), 1);
    assert_eq!(
        left.items[0].view.issue_identifier.as_deref(),
        Some("ENG-908")
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn snooze_resurfaces_via_job_and_new_activity_ends_snooze(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 910, alice, Some(bob), 1).await;
    e.handle(&e.user_ev(alice, eng_issue_created(issue, &e.d, vec![])))
        .await;
    let ctx = ctx_for(SeedUser::Bob);
    let id = e.first_notification(bob).await;
    e.app.open_notification(&ctx, id).await.unwrap();
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 0);

    let now = e.app.clock.now();
    let until = now + Duration::hours(2);
    let snoozed = e
        .app
        .mark_notification(
            &ctx,
            id,
            MarkNotificationInput {
                snoozed_until_at: Some(Some(until)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(snoozed.view.n.snoozed_until_at, Some(until));
    assert!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let show = ListNotificationsInput {
        include_snoozed: true,
        ..Default::default()
    };
    assert_eq!(
        e.app
            .list_notifications(&ctx, show)
            .await
            .unwrap()
            .items
            .len(),
        1
    );

    // Vor Ablauf passiert nichts.
    SnoozeJob
        .run(&e.app, now + Duration::hours(1))
        .await
        .unwrap();
    assert!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );

    // Nach Ablauf: wieder vorgelegt als ungelesen.
    SnoozeJob
        .run(&e.app, until + Duration::minutes(1))
        .await
        .unwrap();
    let list = e
        .app
        .list_notifications(&ctx, Default::default())
        .await
        .unwrap();
    assert_eq!(list.items.len(), 1);
    let n = &list.items[0].view.n;
    assert!(n.read_at.is_none());
    assert!(n.unsnoozed_at.is_some());
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 1);

    // Unsnooze per null.
    let id = n.id;
    e.app
        .mark_notification(
            &ctx,
            id,
            MarkNotificationInput {
                snoozed_until_at: Some(Some(until + Duration::days(1))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    e.app
        .mark_notification(
            &ctx,
            id,
            MarkNotificationInput {
                snoozed_until_at: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .len(),
        1
    );

    // Neue Aktivität in der Gruppe beendet ein Snooze sofort.
    e.app
        .mark_notification(
            &ctx,
            id,
            MarkNotificationInput {
                snoozed_until_at: Some(Some(until + Duration::days(2))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let comment = e.comment(issue, alice, None).await;
    e.subscribe(issue, bob).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    let list = e
        .app
        .list_notifications(&ctx, Default::default())
        .await
        .unwrap();
    assert_eq!(list.items.len(), 2, "gesamte Gruppe sichtbar");
    assert!(
        list.items
            .iter()
            .all(|i| i.view.n.unsnoozed_at.is_some() || i.view.n.snoozed_until_at.is_none())
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn cap_archives_oldest_beyond_2000_and_keeps_snoozed_last(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 911, alice, None, 1).await;
    let now = e.app.clock.now();
    // 2005 Notifications für Bob (je 1 s Abstand); die 3 ältesten sind gesnoozt (bleiben erhalten).
    sqlx::query(
        "INSERT INTO notification (organization_id, user_id, type, category, target_type, actor_id, issue_id, \
         dedup_key, created_at, updated_at, snoozed_until_at) \
         SELECT $1, $2, 'issueNewComment', 'commentsAndReplies', 'issue', $3, $4, 'seed:' || g, \
                $5::timestamptz - (g || ' seconds')::interval, $5::timestamptz - (g || ' seconds')::interval, \
                CASE WHEN g > 2002 THEN $5::timestamptz + interval '30 days' END \
         FROM generate_series(1, 2005) g",
    )
    .bind(e.d.org.0)
    .bind(bob.0)
    .bind(alice.0)
    .bind(issue.0)
    .bind(now)
    .execute(&e.pool)
    .await
    .unwrap();
    // Alice (anderer User) hat wenige und bleibt unberührt.
    sqlx::query(
        "INSERT INTO notification (organization_id, user_id, type, category, target_type, actor_id, issue_id, dedup_key) \
         VALUES ($1,$2,'issueMention','mentions','issue',$3,$4,'seed:a')",
    )
    .bind(e.d.org.0)
    .bind(alice.0)
    .bind(bob.0)
    .bind(issue.0)
    .execute(&e.pool)
    .await
    .unwrap();

    NotificationCapJob.run(&e.app, now).await.unwrap();
    let unarchived: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notification WHERE user_id = $1 AND archived_at IS NULL",
    )
    .bind(bob.0)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(unarchived, 2000);
    let archived: Vec<i32> = sqlx::query_scalar(
        "SELECT substr(dedup_key, 6)::int FROM notification WHERE user_id = $1 AND archived_at IS NOT NULL \
         ORDER BY 1",
    )
    .bind(bob.0)
    .fetch_all(&e.pool)
    .await
    .unwrap();
    // Älteste nicht gesnoozte (g = 2000, 1999, ...) werden archiviert; gesnoozte (g > 2002) bleiben.
    assert_eq!(archived.len(), 5);
    assert!(
        archived.iter().all(|g| *g <= 2002 && *g >= 1998),
        "{archived:?}"
    );
    let alice_unarchived: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notification WHERE user_id = $1 AND archived_at IS NULL",
    )
    .bind(alice.0)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(alice_unarchived, 1);

    // Idempotent; archivierte Einträge werden nach 90 Tagen entfernt.
    NotificationCapJob.run(&e.app, now).await.unwrap();
    NotificationCapJob
        .run(&e.app, now + Duration::days(91))
        .await
        .unwrap();
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM notification WHERE user_id = $1")
        .bind(bob.0)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(total, 2000);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn subscribe_unsubscribe_and_added_by_other(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 912, e.d.alice, None, 1).await;
    let (alice_ctx, bob_ctx) = (ctx_for(SeedUser::Alice), ctx_for(SeedUser::Bob));

    // Bob abonniert manuell.
    let r = e
        .app
        .subscribe_issue(&bob_ctx, "ENG-912", None)
        .await
        .unwrap();
    assert!(r.changed && r.user_id == e.d.bob);
    assert!(
        !e.app
            .subscribe_issue(&bob_ctx, "ENG-912", None)
            .await
            .unwrap()
            .changed
    );
    // Alice fügt Bob erneut hinzu: schon aktiv → keine Änderung; dann beendet Bob das Abo.
    assert!(
        e.app
            .unsubscribe_issue(&bob_ctx, &issue.to_string(), None)
            .await
            .unwrap()
            .changed
    );
    let subs = e
        .app
        .list_issue_subscriptions(&bob_ctx, "ENG-912")
        .await
        .unwrap();
    assert_eq!(subs.len(), 1);
    assert!(!subs[0].active);

    // Alice fügt Bob hinzu (addedByOther) → Event; der Fan-out erzeugt issueSubscribed für Bob.
    let r = e
        .app
        .subscribe_issue(&alice_ctx, "ENG-912", Some("bob"))
        .await
        .unwrap();
    assert!(r.changed);
    let events: Vec<(String, Option<Uuid>)> =
        sqlx::query_as("SELECT type, actor_id FROM outbox_event ORDER BY sync_id, created_at")
            .fetch_all(&e.pool)
            .await
            .unwrap();
    let created: Vec<_> = events
        .iter()
        .filter(|(t, _)| t == "IssueSubscriptionCreated")
        .collect();
    assert_eq!(created.len(), 2);
    assert_eq!(
        created[1].1,
        Some(e.d.alice.0),
        "actor_id wird mitgeschrieben"
    );
    let stored = ev(
        e.d.org,
        EventOrigin::User,
        Some(e.d.alice),
        DomainEvent::IssueSubscriptionCreated {
            issue_id: issue,
            user_id: e.d.bob,
        },
    );
    e.handle(&stored).await;
    assert_eq!(e.kinds(e.d.bob).await, vec!["issueSubscribed"]);
    // Selbst-Abo erzeugt keine Notification (Self-Suppression).
    let own = ev(
        e.d.org,
        EventOrigin::User,
        Some(e.d.bob),
        DomainEvent::IssueSubscriptionCreated {
            issue_id: issue,
            user_id: e.d.bob,
        },
    );
    e.handle(&own).await;
    assert_eq!(e.count(e.d.bob).await, 1);

    // Unbekanntes Issue / Issue im privaten Team → not_found (auch als Owner ohne Mitgliedschaft).
    let err = e
        .app
        .subscribe_issue(&bob_ctx, "ENG-9999", None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    let private = e.issue(e.d.des, 3, 913, e.d.bob, None, 1).await;
    let err = e
        .app
        .subscribe_issue(&alice_ctx, "DES-913", None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    let err = e
        .app
        .subscribe_issue(&alice_ctx, &private.to_string(), None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    // Bob (Mitglied) darf; Alice kann Bob dort aber nicht hinzufügen, ohne das Issue zu sehen.
    assert!(
        e.app
            .subscribe_issue(&bob_ctx, "DES-913", None)
            .await
            .unwrap()
            .changed
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn comment_does_not_revive_unsubscription_but_assignment_does(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 914, alice, None, 1).await;
    let bob_ctx = ctx_for(SeedUser::Bob);
    e.app
        .subscribe_issue(&bob_ctx, "ENG-914", None)
        .await
        .unwrap();
    e.app
        .unsubscribe_issue(&bob_ctx, "ENG-914", None)
        .await
        .unwrap();

    // Bob kommentiert: das beendete Abo bleibt beendet.
    let c = e.comment(issue, bob, None).await;
    e.handle(&e.user_ev(
        bob,
        DomainEvent::CommentCreated {
            comment_id: c,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    let active: bool =
        sqlx::query_scalar("SELECT active FROM issue_subscription WHERE user_id = $1")
            .bind(bob.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(!active);
    // Alice kommentiert: Bob (beendet) erhält nichts.
    let c2 = e.comment(issue, alice, None).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: c2,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.count(bob).await, 0);
    // Zuweisung reaktiviert.
    sqlx::query("UPDATE issue SET assignee_id = $1 WHERE id = $2")
        .bind(bob.0)
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&e.user_ev(
        alice,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: issue,
            team_id: e.d.eng,
            changed: vec!["assigneeId".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    let active: bool =
        sqlx::query_scalar("SELECT active FROM issue_subscription WHERE user_id = $1")
            .bind(bob.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(active);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn reply_notifies_thread_participants_and_mention_in_reply_subscribes_thread_only(
    pool: PgPool,
) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 915, alice, None, 1).await;
    // Bob ist KEIN Subscriber, aber Teilnehmer des Threads (Wurzel von Bob).
    let root = e.comment(issue, bob, None).await;
    let reply = e.comment(issue, alice, Some(root)).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: reply,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueNewComment"]);

    // Erwähnung in einer Antwort: nur Thread-Abo, kein Issue-Abo.
    let carol_in_web = e.issue(e.d.web, 2, 916, alice, None, 1).await;
    let wroot = e.comment(carol_in_web, alice, None).await;
    let wreply = e.comment(carol_in_web, bob, Some(wroot)).await;
    e.handle(&e.user_ev(
        bob,
        DomainEvent::CommentCreated {
            comment_id: wreply,
            mentioned_user_ids: vec![e.d.carol],
        },
    ))
    .await;
    assert_eq!(e.kinds(e.d.carol).await, vec!["issueCommentMention"]);
    let rows: Vec<(Option<Uuid>, String)> = sqlx::query_as(
        "SELECT thread_comment_id, reason FROM issue_subscription WHERE user_id = $1",
    )
    .bind(e.d.carol.0)
    .fetch_all(&e.pool)
    .await
    .unwrap();
    assert_eq!(rows, vec![(Some(wroot.0), "mention".to_owned())]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn status_change_to_closed_notifies_subscribers_and_team_subscribers(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 917, alice, None, 3).await; // Done
    e.subscribe(issue, bob).await;
    // Alice abonniert das Team ENG mit issueStatusChanged.
    let alice_ctx = ctx_for(SeedUser::Alice);
    e.app
        .set_notification_subscription(
            &alice_ctx,
            SetObjectSubscriptionInput {
                kind: lineal_app::ports::repos::notification::ObjectKind::Team,
                target: e.d.eng.0,
                context_view_type: Some("activeIssues".into()),
                types: Some(vec!["issueStatusChanged".into()]),
                include_sub_initiative_updates: None,
                active: None,
            },
        )
        .await
        .unwrap();
    let carol_actor = e.d.carol;
    e.handle(&ev(
        e.d.org,
        EventOrigin::User,
        Some(carol_actor),
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: issue,
            team_id: e.d.eng,
            changed: vec!["stateId".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueStatusChanged"]);
    assert_eq!(e.kinds(alice).await, vec!["issueStatusChanged"]);

    // Wechsel in einen offenen Status löst issueStatusChanged nicht aus.
    let open = e.issue(e.d.eng, 1, 918, alice, None, 1).await;
    e.subscribe(open, bob).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: open,
            team_id: e.d.eng,
            changed: vec!["stateId".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.count(bob).await, 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn team_subscription_issue_created_and_sla_events(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let alice_ctx = ctx_for(SeedUser::Alice);
    use lineal_app::ports::repos::notification::ObjectKind;
    // Default-Typen für ganze Teams: SLA-Risiko und -Bruch.
    let sub = e
        .app
        .set_notification_subscription(
            &alice_ctx,
            SetObjectSubscriptionInput {
                kind: ObjectKind::Team,
                target: e.d.eng.0,
                context_view_type: None,
                types: None,
                include_sub_initiative_updates: None,
                active: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(sub.types, vec!["issueSlaHighRisk", "issueSlaBreached"]);
    let issue = e.issue(e.d.eng, 1, 919, bob, Some(bob), 1).await;
    e.handle(&ev(
        e.d.org,
        EventOrigin::System,
        None,
        DomainEvent::IssueSlaBreached { issue_id: issue },
    ))
    .await;
    assert_eq!(e.kinds(alice).await, vec!["issueSlaBreached"]);
    assert_eq!(e.kinds(bob).await, vec!["issueSlaBreached"], "Assignee");
    let priority: bool =
        sqlx::query_scalar("SELECT is_priority FROM notification WHERE user_id = $1")
            .bind(alice.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(priority, "SLA-Bruch ist immer Priorität");

    // Abo auf issueCreated (Kontext activeIssues) mit Default-Typ; Ersteller Bob wird benachrichtigt? Nein, Alice.
    e.app
        .set_notification_subscription(
            &alice_ctx,
            SetObjectSubscriptionInput {
                kind: ObjectKind::Team,
                target: e.d.eng.0,
                context_view_type: Some("activeIssues".into()),
                types: None,
                include_sub_initiative_updates: None,
                active: None,
            },
        )
        .await
        .unwrap();
    let new_issue = e.issue(e.d.eng, 1, 920, bob, None, 1).await;
    e.handle(&e.user_ev(bob, eng_issue_created(new_issue, &e.d, vec![])))
        .await;
    assert_eq!(
        e.kinds(alice).await,
        vec!["issueSlaBreached", "issueCreated"]
    );
    assert_eq!(
        e.count(bob).await,
        1,
        "Bob: nur SLA; eigene Anlage erzeugt nichts"
    );

    // Pausiertes Abo erzeugt nichts.
    e.app
        .set_notification_subscription(
            &alice_ctx,
            SetObjectSubscriptionInput {
                kind: ObjectKind::Team,
                target: e.d.eng.0,
                context_view_type: Some("activeIssues".into()),
                types: None,
                include_sub_initiative_updates: None,
                active: Some(false),
            },
        )
        .await
        .unwrap();
    let third = e.issue(e.d.eng, 1, 921, bob, None, 1).await;
    e.handle(&e.user_ev(bob, eng_issue_created(third, &e.d, vec![])))
        .await;
    assert_eq!(e.count(alice).await, 2);

    // Zielprüfung: privates Team DES ist für Alice nicht abonnierbar.
    let err = e
        .app
        .set_notification_subscription(
            &alice_ctx,
            SetObjectSubscriptionInput {
                kind: ObjectKind::Team,
                target: e.d.des.0,
                context_view_type: None,
                types: None,
                include_sub_initiative_updates: None,
                active: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(
        e.app
            .list_notification_subscriptions(&alice_ctx)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_update_reaches_project_subscribers(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let project = Uuid::now_v7();
    let status: Uuid = sqlx::query_scalar("SELECT id FROM project_status LIMIT 1")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project (id, organization_id, name, lead_team_id, number, status_id, creator_id) \
         VALUES ($1,$2,'Notification Test Project',$3,101,$4,$5)",
    )
    .bind(project)
    .bind(e.d.org.0)
    .bind(e.d.eng.0)
    .bind(status)
    .bind(alice.0)
    .execute(&e.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1,$2,$3)",
    )
    .bind(e.d.org.0)
    .bind(project)
    .bind(e.d.eng.0)
    .execute(&e.pool)
    .await
    .unwrap();
    let update = Uuid::now_v7();
    sqlx::query("INSERT INTO project_update (id, organization_id, project_id, user_id, body) VALUES ($1,$2,$3,$4,'On track')")
        .bind(update)
        .bind(e.d.org.0)
        .bind(project)
        .bind(alice.0)
        .execute(&e.pool)
        .await
        .unwrap();
    use lineal_app::ports::repos::notification::ObjectKind;
    e.app
        .set_notification_subscription(
            &ctx_for(SeedUser::Bob),
            SetObjectSubscriptionInput {
                kind: ObjectKind::Project,
                target: project,
                context_view_type: None,
                types: None,
                include_sub_initiative_updates: None,
                active: None,
            },
        )
        .await
        .unwrap();
    e.handle(&e.user_ev(
        alice,
        DomainEvent::ProjectUpdateCreated {
            update_id: ProjectUpdateId(update),
            project_id: ProjectId(project),
        },
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["projectUpdateCreated"]);
    assert_eq!(e.count(alice).await, 0);
    let ctx = ctx_for(SeedUser::Bob);
    let item = &e
        .app
        .list_notifications(&ctx, Default::default())
        .await
        .unwrap()
        .items[0];
    assert_eq!(item.title, "Notification Test Project");
    assert_eq!(item.view.n.category, "postsAndUpdates");
    assert!(item.url.contains("/acme/project/"));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn due_reminders_fire_once_per_stage_after_nine_local(pool: PgPool) {
    // 2026-10-10 07:15 UTC = 09:15 Berlin (Sommerzeit).
    let e = env(pool).await;
    let bob = e.d.bob;
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();
    let tomorrow_issue = e.issue(e.d.eng, 1, 922, e.d.alice, Some(bob), 1).await;
    let overdue_issue = e.issue(e.d.eng, 1, 923, e.d.alice, Some(bob), 1).await;
    let later_issue = e.issue(e.d.eng, 1, 924, e.d.alice, Some(bob), 1).await;
    let closed_issue = e.issue(e.d.eng, 1, 925, e.d.alice, Some(bob), 3).await;
    for (i, d) in [
        (tomorrow_issue, today + Duration::days(1)),
        (overdue_issue, today - Duration::days(3)),
        (later_issue, today + Duration::days(5)),
        (closed_issue, today - Duration::days(1)),
    ] {
        sqlx::query("UPDATE issue SET due_date = $1 WHERE id = $2")
            .bind(d)
            .bind(i.0)
            .execute(&e.pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE \"user\" SET timezone = 'Europe/Berlin' WHERE id = $1")
        .bind(bob.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let now = e.app.clock.now();

    // Ohne aktivierte Erinnerungen passiert nichts.
    assert_eq!(e.app.create_due_reminders(now).await.unwrap(), 0);
    e.app
        .update_notification_settings(
            &ctx_for(SeedUser::Bob),
            NotificationSettingsPatch {
                due_date_reminders: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Vor 09:00 lokal (06:59 UTC = 08:59 Berlin) nichts.
    assert_eq!(
        e.app
            .create_due_reminders(t(2026, 10, 10, 6, 59))
            .await
            .unwrap(),
        0
    );
    DueRemindersJob.run(&e.app, now).await.unwrap();
    let kinds = e.kinds(bob).await;
    assert_eq!(
        kinds,
        vec!["issueDue", "issueDue"],
        "morgen fällig + überfällig, nicht später, nicht geschlossen"
    );
    // Einmalig je Issue/Datum/Stufe.
    assert_eq!(
        e.app
            .create_due_reminders(now + Duration::hours(3))
            .await
            .unwrap(),
        0
    );
    assert_eq!(e.count(bob).await, 2);
    let item = &e
        .app
        .list_notifications(&ctx_for(SeedUser::Bob), Default::default())
        .await
        .unwrap()
        .items[0];
    assert!(item.subtitle.contains("is due soon"));
    assert_eq!(item.view.n.category, "reminders");
    assert!(item.view.n.actor_id.is_none());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn email_digest_respects_delay_category_and_read_state(pool: PgPool) {
    let e = env_at(pool, Some(t(2026, 10, 7, 10, 0))).await; // Mittwoch 12:00 Berlin
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 926, alice, None, 1).await;
    e.subscribe(issue, bob).await;
    let comment = e.comment(issue, alice, None).await;
    // Mention (mittlere Dringlichkeit, E-Mail Default an) und Kommentar (niedrig, E-Mail aus).
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![bob],
        },
    ))
    .await;
    let c2 = e.comment(issue, alice, None).await;
    // Ohne menschlichen Akteur (Integration): wird nicht mit der Erwähnung gebündelt (docs/05 §6.6).
    e.handle(&ev(
        e.d.org,
        EventOrigin::System,
        None,
        DomainEvent::CommentCreated {
            comment_id: c2,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(
        e.kinds(bob).await,
        vec!["issueCommentMention", "issueNewComment"]
    );
    let now = e.app.clock.now();

    // Vor Ablauf der 5 Minuten: keine Mail.
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::minutes(2))
            .await
            .unwrap(),
        0
    );
    // Danach: genau eine Mail mit der Mention; der Kommentar bleibt ohne Mail (Kategorie aus).
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::minutes(6))
            .await
            .unwrap(),
        1
    );
    {
        let mails = e.mails.0.lock().unwrap();
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].to, "bob@acme.example");
        assert!(
            mails[0]
                .body
                .contains("mentioned you in a comment on ENG-926"),
            "{}",
            mails[0].body
        );
        assert!(!mails[0].body.contains("commented on ENG-926"));
    }
    let emailed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notification WHERE emailed_at IS NOT NULL")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(emailed, 1);
    // Nicht erneut.
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::hours(1))
            .await
            .unwrap(),
        0
    );

    // Kategorie commentsAndReplies per Kanal-Einstellung aktivieren: niedrige Dringlichkeit 30 min.
    e.app
        .update_notification_settings(
            &ctx_for(SeedUser::Bob),
            NotificationSettingsPatch {
                channels: vec![lineal_app::ports::repos::notification::ChannelPreference {
                    category: "commentsAndReplies".into(),
                    channel: "email".into(),
                    enabled: true,
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::minutes(29))
            .await
            .unwrap(),
        0,
        "noch vor Ablauf"
    );
    // Gelesene Notifications werden nicht gemailt.
    let id = e.first_notification(bob).await;
    let _ = id;
    sqlx::query("UPDATE notification SET read_at = $1 WHERE type = 'issueNewComment'")
        .bind(now)
        .execute(&e.pool)
        .await
        .unwrap();
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::hours(2))
            .await
            .unwrap(),
        0
    );
    // E-Mail global aus: nichts.
    sqlx::query("UPDATE notification SET read_at = NULL")
        .execute(&e.pool)
        .await
        .unwrap();
    e.app
        .update_notification_settings(
            &ctx_for(SeedUser::Bob),
            NotificationSettingsPatch {
                email_enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::hours(2))
            .await
            .unwrap(),
        0
    );
    assert_eq!(e.mails.0.lock().unwrap().len(), 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn settings_roundtrip_and_validation(pool: PgPool) {
    let e = env(pool).await;
    let ctx = ctx_for(SeedUser::Bob);
    let s = e.app.get_notification_settings(&ctx).await.unwrap();
    assert!(s.email_enabled && s.email_hold_outside_work_hours && s.email_immediate_urgent);
    assert!(!s.email_only_priority && !s.due_date_reminders);
    assert_eq!(s.email_format, "digest");
    assert_eq!(
        s.priority_categories,
        vec!["assignments", "mentions", "reminders", "triage"]
    );
    assert_eq!(s.channels.len(), 33);
    let on = |s: &lineal_app::usecases::notification::NotificationSettings, c: &str, ch: &str| {
        s.channels
            .iter()
            .find(|p| p.category == c && p.channel == ch)
            .unwrap()
            .enabled
    };
    assert!(
        on(&s, "mentions", "email")
            && !on(&s, "reactions", "email")
            && on(&s, "reactions", "desktop")
    );

    let s = e
        .app
        .update_notification_settings(
            &ctx,
            NotificationSettingsPatch {
                email_format: Some("immediate".into()),
                email_only_priority: Some(true),
                priority_categories: Some(vec!["mentions".into()]),
                channels: vec![lineal_app::ports::repos::notification::ChannelPreference {
                    category: "reactions".into(),
                    channel: "email".into(),
                    enabled: true,
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(s.email_format, "immediate");
    assert!(s.email_only_priority && s.email_enabled);
    assert_eq!(s.priority_categories, vec!["mentions"]);
    assert!(on(&s, "reactions", "email"));
    assert_eq!(e.app.get_notification_settings(&ctx).await.unwrap(), s);

    for bad in [
        NotificationSettingsPatch {
            email_format: Some("weekly".into()),
            ..Default::default()
        },
        NotificationSettingsPatch {
            priority_categories: Some(vec!["nope".into()]),
            ..Default::default()
        },
        NotificationSettingsPatch {
            channels: vec![lineal_app::ports::repos::notification::ChannelPreference {
                category: "mentions".into(),
                channel: "inbox".into(),
                enabled: false,
            }],
            ..Default::default()
        },
    ] {
        let err = e
            .app
            .update_notification_settings(&ctx, bad)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidRequest);
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn access_loss_hides_existing_notifications(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 927, alice, Some(bob), 1).await;
    e.handle(&e.user_ev(alice, eng_issue_created(issue, &e.d, vec![])))
        .await;
    let ctx = ctx_for(SeedUser::Bob);
    assert_eq!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    // Issue wandert ins private Team DES; Alice (nicht Mitglied) verliert den Zugriff.
    sqlx::query("UPDATE issue SET team_id = $1, state_id = $2 WHERE id = $3")
        .bind(e.d.des.0)
        .bind(seed_workflow_state_ids(3)[1].0)
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    // Bob ist Mitglied in DES: sichtbar. Für Alice ist nichts entstanden. Gegenprobe via Alice-Notification:
    sqlx::query(
        "INSERT INTO notification (organization_id, user_id, type, category, target_type, actor_id, issue_id, dedup_key) \
         VALUES ($1,$2,'issueMention','mentions','issue',$3,$4,'x')",
    )
    .bind(e.d.org.0)
    .bind(alice.0)
    .bind(bob.0)
    .bind(issue.0)
    .execute(&e.pool)
    .await
    .unwrap();
    let alice_ctx = ctx_for(SeedUser::Alice);
    assert!(
        e.app
            .list_notifications(&alice_ctx, Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        e.app.unread_notification_count(&alice_ctx).await.unwrap(),
        0
    );
    assert_eq!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn blocking_relation_and_reaction_notifications(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let blocker = e.issue(e.d.eng, 1, 928, alice, None, 1).await;
    let blocked = e.issue(e.d.eng, 1, 929, alice, Some(bob), 1).await;
    let relation = Uuid::now_v7();
    sqlx::query("INSERT INTO issue_relation (id, organization_id, issue_id, related_issue_id, type) VALUES ($1,$2,$3,$4,'blocks')")
        .bind(relation)
        .bind(e.d.org.0)
        .bind(blocker.0)
        .bind(blocked.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&e.user_ev(
        alice,
        DomainEvent::IssueRelationCreated {
            relation_id: IssueRelationId(relation),
            issue_id: blocker,
        },
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueBlocking"]);

    // Reaktion auf Bobs Kommentar → Autor erhält issueCommentReaction.
    let comment = e.comment(blocked, bob, None).await;
    let reaction = Uuid::now_v7();
    sqlx::query("INSERT INTO reaction (id, organization_id, emoji, user_id, comment_id) VALUES ($1,$2,'👍',$3,$4)")
        .bind(reaction)
        .bind(e.d.org.0)
        .bind(alice.0)
        .bind(comment.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&e.user_ev(
        alice,
        DomainEvent::ReactionCreated {
            reaction_id: ReactionId(reaction),
        },
    ))
    .await;
    assert_eq!(
        e.kinds(bob).await,
        vec!["issueBlocking", "issueCommentReaction"]
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn mention_in_issue_description_and_priority_urgent(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 930, alice, Some(bob), 1).await;
    sqlx::query("UPDATE issue SET priority = 1 WHERE id = $1")
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    // Anlage: Bob ist zugewiesen UND erwähnt UND Urgent → genau eine Notification (Mention gewinnt).
    e.handle(&e.user_ev(alice, eng_issue_created(issue, &e.d, vec![bob])))
        .await;
    assert_eq!(e.kinds(bob).await, vec!["issueMention"]);
    // Priorität nachträglich auf Urgent: der Assignee wird informiert.
    let other = e.issue(e.d.eng, 1, 931, alice, Some(bob), 1).await;
    sqlx::query("UPDATE issue SET priority = 1 WHERE id = $1")
        .bind(other.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&e.user_ev(
        alice,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: other,
            team_id: e.d.eng,
            changed: vec!["priority".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(
        e.kinds(bob).await,
        vec!["issueMention", "issuePriorityUrgent"]
    );
}

#[test]
fn consumer_and_job_names_match_registry() {
    assert_eq!(NotificationsConsumer.name(), "notifications");
    assert_eq!(SnoozeJob.name(), "snooze");
    assert_eq!(NotificationCapJob.name(), "notification_cap");
    assert_eq!(DueRemindersJob.name(), "due_reminders");
    let _ = NewEvent::new(DomainEvent::OrganizationUpdated {}, EventOrigin::System);
}

struct AlwaysFails(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait]
impl EventConsumer for AlwaysFails {
    fn name(&self) -> &'static str {
        "always_fails"
    }
    async fn handle(&self, _app: &App, _ev: &StoredEvent) -> AppResult<()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(lineal_app::AppError::internal("boom"))
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn worker_gives_up_on_an_event_after_max_attempts(pool: PgPool) {
    let e = env(pool).await;
    let mut tx = e.app.store.begin(e.d.org).await.unwrap();
    tx.outbox()
        .push(NewEvent::new(
            DomainEvent::OrganizationUpdated {},
            EventOrigin::System,
        ))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let worker = lineal_jobs::Worker::with(
        e.app.clone(),
        vec![Arc::new(AlwaysFails(calls.clone()))],
        vec![],
        lineal_jobs::WorkerConfig {
            max_event_attempts: 3,
            ..Default::default()
        },
    );
    let mut delivered = Vec::new();
    for _ in 0..5 {
        delivered.push(worker.tick().await.unwrap().deliveries);
    }
    assert_eq!(
        delivered,
        vec![0, 0, 1, 0, 0],
        "beim dritten Fehlversuch aufgegeben"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn other_users_notifications_cannot_be_read_or_changed(pool: PgPool) {
    let e = env(pool).await;
    let issue = e.issue(e.d.eng, 1, 940, e.d.alice, Some(e.d.bob), 1).await;
    e.handle(&e.user_ev(e.d.alice, eng_issue_created(issue, &e.d, vec![])))
        .await;
    let id = e.first_notification(e.d.bob).await;
    let alice = ctx_for(SeedUser::Alice);
    for err in [
        e.app.get_notification(&alice, id).await.unwrap_err(),
        e.app.open_notification(&alice, id).await.unwrap_err(),
        e.app
            .mark_notification(
                &alice,
                id,
                MarkNotificationInput {
                    read: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err(),
        e.app
            .mark_notification(
                &alice,
                id,
                MarkNotificationInput {
                    snoozed_until_at: Some(Some(e.app.clock.now() + Duration::hours(1))),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err(),
        e.app
            .archive_notification(&alice, id, true)
            .await
            .unwrap_err(),
    ] {
        assert_eq!(err.kind, ErrorKind::NotFound);
    }
    // Alices Massenaktionen berühren Bobs Notification nicht.
    assert_eq!(
        e.app
            .mark_all_notifications_read(&alice, None)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        e.app.archive_all_notifications(&alice, None).await.unwrap(),
        0
    );
    let (read, archived): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT read_at, archived_at FROM notification WHERE id = $1")
            .bind(id.0)
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(read.is_none() && archived.is_none());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn removal_from_private_team_hides_notifications_everywhere_and_stops_email(pool: PgPool) {
    let e = env_at(pool, Some(t(2026, 10, 7, 10, 0))).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    // Bob ist Mitglied des privaten Teams DES und wird dort zugewiesen (E-Mail für assignments an).
    let issue = e.issue(e.d.des, 3, 941, alice, Some(bob), 1).await;
    sqlx::query("UPDATE issue SET title = 'Geheimes Redesign' WHERE id = $1")
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&ev(
        e.d.org,
        EventOrigin::System,
        None,
        eng_issue_created(issue, &e.d, vec![]),
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueAssignedToYou"]);
    let ctx = ctx_for(SeedUser::Bob);
    let id = e.first_notification(bob).await;
    assert_eq!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 1);

    // Bob verliert die Mitgliedschaft in DES.
    sqlx::query("DELETE FROM team_membership WHERE user_id = $1 AND team_id = $2")
        .bind(bob.0)
        .bind(e.d.des.0)
        .execute(&e.pool)
        .await
        .unwrap();
    assert!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 0);
    assert_eq!(
        e.app.get_notification(&ctx, id).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        e.app.open_notification(&ctx, id).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
    let mark = MarkNotificationInput {
        read: Some(true),
        ..Default::default()
    };
    assert_eq!(
        e.app
            .mark_notification(&ctx, id, mark)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        e.app
            .archive_notification(&ctx, id, true)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    assert_eq!(
        e.app.mark_all_notifications_read(&ctx, None).await.unwrap(),
        0
    );
    assert_eq!(
        e.app.archive_all_notifications(&ctx, None).await.unwrap(),
        0
    );

    // Digest: weder Titel noch Text gelangen per E-Mail an Bob.
    let now = e.app.clock.now();
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::hours(1))
            .await
            .unwrap(),
        0
    );
    assert!(e.mails.0.lock().unwrap().is_empty());
    let emailed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notification WHERE emailed_at IS NOT NULL")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(emailed, 0);

    // Kontrolle: mit Mitgliedschaft geht die Mail raus und enthält keine fremde E-Mail-Adresse.
    sqlx::query("INSERT INTO team_membership (organization_id, team_id, user_id, owner) VALUES ($1,$2,$3,false)")
        .bind(e.d.org.0)
        .bind(e.d.des.0)
        .bind(bob.0)
        .execute(&e.pool)
        .await
        .unwrap();
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::hours(1))
            .await
            .unwrap(),
        1
    );
    let mails = e.mails.0.lock().unwrap();
    assert!(mails[0].body.contains("Geheimes Redesign"));
    assert!(
        !mails[0].body.contains("@acme.example"),
        "{}",
        mails[0].body
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn read_only_token_cannot_change_inbox_or_subscriptions(pool: PgPool) {
    let e = env(pool).await;
    let mut ctx = ctx_for(SeedUser::Bob);
    ctx.scopes.write = false;
    let err = e
        .app
        .update_notification_settings(
            &ctx,
            NotificationSettingsPatch {
                due_date_reminders: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, lineal_app::ErrorKind::Forbidden);
    let err = e
        .app
        .mark_all_notifications_read(&ctx, None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, lineal_app::ErrorKind::Forbidden);
}

// --- Paket 6: Bündelung, Dringlichkeit, Freigaben, Gruppen ---

fn assignee_changed(issue: IssueId, d: &SeedData) -> DomainEvent {
    DomainEvent::IssueUpdated {
        previous: None,
        issue_id: issue,
        team_id: d.eng,
        changed: vec!["assigneeId".into()],
        mentioned_user_ids: vec![],
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bundling_keeps_priority_and_does_not_duplicate_types(pool: PgPool) {
    let e = env_at(pool, Some(t(2026, 10, 7, 10, 0))).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    // Issue ist bereits erledigt: der Statuswechsel erzeugt für den Assignee `issueStatusChanged`
    // (Kategorie statusChanges, kein Priority-Default).
    let issue = e.issue(e.d.eng, 1, 950, alice, Some(bob), 3).await;
    e.handle(&e.user_ev(alice, assignee_changed(issue, &e.d)))
        .await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: issue,
            team_id: e.d.eng,
            changed: vec!["stateId".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    // Doppelte Zuweisung (Anlage + Zuweisung) erzeugt keinen zweiten gleichen Eintrag.
    e.handle(&e.user_ev(alice, assignee_changed(issue, &e.d)))
        .await;
    let rows: Vec<(String, bool, Vec<String>)> = sqlx::query_as(
        "SELECT type, is_priority, bundled_types FROM notification WHERE user_id = $1",
    )
    .bind(bob.0)
    .fetch_all(&e.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "issueAssignedToYou");
    assert!(
        rows[0].1,
        "isPriority darf durch die Bündelung nicht fallen"
    );
    assert_eq!(
        rows[0].2,
        vec!["issueAssignedToYou", "issueStatusChanged"],
        "keine doppelten Typen"
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn opposite_events_end_in_the_final_state(pool: PgPool) {
    let e = env_at(pool, Some(t(2026, 10, 7, 10, 0))).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 951, alice, Some(bob), 1).await;
    e.handle(&e.user_ev(alice, assignee_changed(issue, &e.d)))
        .await;
    sqlx::query("UPDATE issue SET assignee_id = NULL WHERE id = $1")
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(&e.user_ev(
        alice,
        DomainEvent::IssueUpdated {
            previous: Some(lineal_domain::events::IssuePrevious {
                assignee_id: Some(bob),
                ..Default::default()
            }),
            issue_id: issue,
            team_id: e.d.eng,
            changed: vec!["assigneeId".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueUnassignedFromYou"]);
}

async fn make_urgent(e: &Env, issue: IssueId) {
    sqlx::query("UPDATE issue SET priority = 1 WHERE id = $1")
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn urgent_assignment_is_mailed_immediately_unless_disabled(pool: PgPool) {
    let e = env_at(pool, Some(t(2026, 10, 7, 10, 0))).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let urgent = e.issue(e.d.eng, 1, 952, alice, Some(bob), 1).await;
    make_urgent(&e, urgent).await;
    e.handle(&e.user_ev(alice, assignee_changed(urgent, &e.d)))
        .await;
    let now = e.app.clock.now();
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::seconds(10))
            .await
            .unwrap(),
        1,
        "Urgent-Zuweisung geht sofort raus"
    );

    // Normale Zuweisung: 5 Minuten Verzögerung.
    let normal = e.issue(e.d.eng, 1, 953, alice, Some(bob), 1).await;
    e.handle(&e.user_ev(alice, assignee_changed(normal, &e.d)))
        .await;
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::seconds(10))
            .await
            .unwrap(),
        0
    );

    // Mit abgeschalteter Sofort-Regel gilt auch Urgent die 5 Minuten.
    e.app
        .update_notification_settings(
            &ctx_for(SeedUser::Bob),
            NotificationSettingsPatch {
                email_immediate_urgent: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let urgent2 = e.issue(e.d.eng, 1, 954, alice, Some(bob), 1).await;
    make_urgent(&e, urgent2).await;
    e.handle(&e.user_ev(alice, assignee_changed(urgent2, &e.d)))
        .await;
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::seconds(20))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::minutes(6))
            .await
            .unwrap(),
        1
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn priority_change_bundled_into_assignment_still_mails_immediately(pool: PgPool) {
    let e = env_at(pool, Some(t(2026, 10, 7, 10, 0))).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 955, alice, Some(bob), 1).await;
    e.handle(&e.user_ev(alice, assignee_changed(issue, &e.d)))
        .await;
    make_urgent(&e, issue).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: issue,
            team_id: e.d.eng,
            changed: vec!["priority".into()],
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.count(bob).await, 1, "gebündelt");
    let now = e.app.clock.now();
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::seconds(10))
            .await
            .unwrap(),
        1
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn guest_with_share_is_mentioned_and_notified(pool: PgPool) {
    let e = env_at(pool, Some(t(2026, 10, 7, 10, 0))).await;
    let (alice, carol) = (e.d.alice, e.d.carol);
    let _issue = e.issue(e.d.eng, 1, 956, alice, None, 1).await;
    sqlx::query(
        r#"UPDATE organization SET limits = limits || '{"issueSharingEnabled": true}'::jsonb"#,
    )
    .execute(&e.pool)
    .await
    .unwrap();
    let actx = ctx_for(SeedUser::Alice);
    let input = |body: &str| lineal_app::usecases::comment::CreateCommentInput {
        parent: Some(lineal_app::usecases::comment::ParentSpec::Issue(
            "ENG-956".into(),
        )),
        reply_to: None,
        body: body.into(),
        quoted_text: None,
    };
    // Ohne Freigabe: keine Erwähnung.
    let m = e
        .app
        .create_comment(&actx, input("Bitte ansehen @carol"))
        .await
        .unwrap();
    assert!(m.mentioned_user_ids.is_empty());
    e.app.share_issue(&actx, "ENG-956", "carol").await.unwrap();
    let m = e
        .app
        .create_comment(&actx, input("Jetzt geteilt @carol"))
        .await
        .unwrap();
    assert_eq!(m.mentioned_user_ids, vec![carol]);
    let comment: CommentId = m.view.comment.id;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![carol],
        },
    ))
    .await;
    assert_eq!(e.kinds(carol).await, vec!["issueCommentMention"]);
    let cctx = ctx_for(SeedUser::Carol);
    assert_eq!(
        e.app
            .list_notifications(&cctx, Default::default())
            .await
            .unwrap()
            .items
            .len(),
        1,
        "Inbox zeigt die Notification des geteilten Issues"
    );
    let id = e.first_notification(carol).await;
    e.app.get_notification(&cctx, id).await.unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn read_state_applies_to_the_whole_group(pool: PgPool) {
    let e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 957, alice, None, 1).await;
    e.subscribe(issue, bob).await;
    let c1 = e.comment(issue, alice, None).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: c1,
            mentioned_user_ids: vec![bob],
        },
    ))
    .await;
    // Anderer Akteur: wird nicht mit der ersten Notification gebündelt, bleibt aber in derselben Gruppe.
    let c2 = e.comment(issue, e.d.carol, None).await;
    e.handle(&e.user_ev(
        e.d.carol,
        DomainEvent::CommentCreated {
            comment_id: c2,
            mentioned_user_ids: vec![],
        },
    ))
    .await;
    assert_eq!(e.count(bob).await, 2);
    let ctx = ctx_for(SeedUser::Bob);
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 1);
    let id = e.first_notification(bob).await;
    e.app
        .mark_notification(
            &ctx,
            id,
            MarkNotificationInput {
                read: Some(true),
                snoozed_until_at: None,
            },
        )
        .await
        .unwrap();
    let unread: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notification WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(bob.0)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(unread, 0, "gelesen gilt für die ganze Gruppe");
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 0);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn notifications_of_deleted_comments_and_trashed_issues_are_hidden_and_not_mailed(
    pool: PgPool,
) {
    let e = env_at(pool, Some(t(2026, 10, 7, 10, 0))).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let issue = e.issue(e.d.eng, 1, 958, alice, None, 1).await;
    e.subscribe(issue, bob).await;
    let comment = e.comment(issue, alice, None).await;
    e.handle(&e.user_ev(
        alice,
        DomainEvent::CommentCreated {
            comment_id: comment,
            mentioned_user_ids: vec![bob],
        },
    ))
    .await;
    let ctx = ctx_for(SeedUser::Bob);
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 1);
    sqlx::query("UPDATE comment SET deleted_at = now() WHERE id = $1")
        .bind(comment.0)
        .execute(&e.pool)
        .await
        .unwrap();
    assert_eq!(e.app.unread_notification_count(&ctx).await.unwrap(), 0);
    let now = e.app.clock.now();
    assert_eq!(
        e.app
            .send_email_digests(now + Duration::minutes(10))
            .await
            .unwrap(),
        0,
        "keine Mail zu gelöschtem Kommentar"
    );
    sqlx::query("UPDATE comment SET deleted_at = NULL WHERE id = $1")
        .bind(comment.0)
        .execute(&e.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE issue SET trashed = true WHERE id = $1")
        .bind(issue.0)
        .execute(&e.pool)
        .await
        .unwrap();
    assert!(
        e.app
            .list_notifications(&ctx, Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
}
