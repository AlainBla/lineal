//! F.17: Abo auf User/„assigned“, „Einladung angenommen“, Hinweis beim Teilen (Seed `acme`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use lineal_app::ports::Store;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::ports::repos::notification::ObjectKind;
use lineal_app::usecases::invitation::{AcceptInvitationInput, InviteUsersInput};
use lineal_app::usecases::notification::SetObjectSubscriptionInput;
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::*;
use lineal_jobs::EventConsumer;
use lineal_jobs::consumers::notifications::NotificationsConsumer;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

async fn setup(pool: &PgPool) -> (Arc<App>, SeedData) {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = Arc::new(SystemClock);
    (Arc::new(app), SeedData::acme())
}

struct SystemClock;
impl lineal_app::ports::Clock for SystemClock {
    fn now(&self) -> chrono::DateTime<Utc> {
        Utc::now()
    }
}

async fn issue(
    pool: &PgPool,
    d: &SeedData,
    team: TeamId,
    slot: u128,
    number: i32,
    assignee: Option<UserId>,
) -> IssueId {
    let id = IssueId::new_v7();
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id, assignee_id) \
         VALUES ($1,$2,$3,$4,'Login schlägt fehl',$5,$6,$7)",
    )
    .bind(id.0)
    .bind(d.org.0)
    .bind(team.0)
    .bind(number)
    .bind(seed_workflow_state_ids(slot)[1].0)
    .bind(d.alice.0)
    .bind(assignee.map(|a| a.0))
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn kinds(pool: &PgPool, user: UserId) -> Vec<String> {
    sqlx::query_scalar("SELECT type FROM notification WHERE user_id = $1 ORDER BY created_at, id")
        .bind(user.0)
        .fetch_all(pool)
        .await
        .unwrap()
}

fn sub(target: UserId, context: Option<&str>) -> SetObjectSubscriptionInput {
    SetObjectSubscriptionInput {
        kind: ObjectKind::User,
        target: target.0,
        context_view_type: context.map(str::to_owned),
        types: None,
        include_sub_initiative_updates: None,
        active: None,
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn user_assigned_subscription_notifies_on_new_assignment(pool: PgPool) {
    let (app, d) = setup(&pool).await;
    let bob = ctx_for(SeedUser::Bob);
    // Falscher Kontext, Selbst-Abo und unbekannter User werden abgelehnt.
    assert_eq!(
        app.set_notification_subscription(&bob, sub(d.alice, Some("backlog")))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        app.set_notification_subscription(&bob, sub(d.bob, None))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        app.set_notification_subscription(&bob, sub(UserId::new_v7(), None))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    let s = app
        .set_notification_subscription(&bob, sub(d.alice, None))
        .await
        .unwrap();
    assert_eq!(s.kind, ObjectKind::User);
    assert_eq!(s.context_view_type.as_deref(), Some("assigned"));
    assert_eq!(s.types, vec!["issueAddedToView".to_owned()]);
    // Idempotent (Upsert) und in der Liste.
    app.set_notification_subscription(&bob, sub(d.alice, Some("assigned")))
        .await
        .unwrap();
    let list = app.list_notification_subscriptions(&bob).await.unwrap();
    assert_eq!(
        list.iter().filter(|s| s.kind == ObjectKind::User).count(),
        1
    );

    let issue = issue(&pool, &d, d.eng, 1, 941, Some(d.alice)).await;
    let ev = StoredEvent {
        id: EventId::new_v7(),
        org_id: d.org,
        origin: EventOrigin::User,
        actor_id: Some(d.alice),
        sync_id: 1,
        event: DomainEvent::IssueCreated {
            issue_id: issue,
            team_id: d.eng,
            mentioned_user_ids: vec![],
            initial: None,
        },
        created_at: Utc::now(),
    };
    NotificationsConsumer.handle(&app, &ev).await.unwrap();
    assert_eq!(kinds(&pool, d.bob).await, vec!["issueAddedToView"]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn accepted_invitation_notifies_the_inviter(pool: PgPool) {
    let (app, d) = setup(&pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let invited = app
        .invite_users(
            &alice,
            InviteUsersInput {
                emails: vec!["neu@acme.example".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    app.accept_invitation(AcceptInvitationInput {
        token: invited[0].token.clone(),
        email: "neu@acme.example".into(),
        name: Some("Neu".into()),
        password: Some("correct-horse".into()),
    })
    .await
    .unwrap();
    assert_eq!(kinds(&pool, d.alice).await, vec!["invitationAccepted"]);
    let items = app
        .list_notifications(&alice, Default::default())
        .await
        .unwrap()
        .items;
    assert_eq!(items[0].title, "Invitation accepted");
    assert!(items[0].subtitle.ends_with("accepted your invitation"));
    assert_eq!(items[0].view.n.category, "system");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sharing_an_issue_notifies_the_recipient(pool: PgPool) {
    let (app, d) = setup(&pool).await;
    // DES ist privat; Bob ist Mitglied, Carol nicht.
    let hidden = issue(&pool, &d, d.des, 3, 942, None).await;
    sqlx::query(
        "UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": true}'::jsonb",
    )
    .execute(&pool)
    .await
    .unwrap();
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);
    assert!(
        app.list_notifications(&carol, Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let r = app
        .share_issue(&bob, &hidden.0.to_string(), &d.carol.0.to_string())
        .await
        .unwrap();
    assert!(r.shared);
    assert_eq!(kinds(&pool, d.carol).await, vec!["issueShared"]);
    let items = app
        .list_notifications(&carol, Default::default())
        .await
        .unwrap()
        .items;
    assert_eq!(items.len(), 1);
    assert!(
        items[0].subtitle.ends_with("with you"),
        "{}",
        items[0].subtitle
    );
    // Bob (Teilender) bekommt keinen Hinweis.
    assert!(kinds(&pool, d.bob).await.is_empty());
}
