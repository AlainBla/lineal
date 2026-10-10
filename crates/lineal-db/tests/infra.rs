//! Beispiel für `#[sqlx::test]`: Outbox und Jobs gegen eine frische Datenbank.

use chrono::Utc;
use lineal_app::ports::Store;
use lineal_app::ports::repos::{NewEvent, NewJob};
use lineal_db::PgStore;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{IssueId, OrgId, TeamId};
use serde_json::json;
use sqlx::PgPool;

fn created() -> NewEvent {
    NewEvent::new(
        DomainEvent::IssueCreated {
            issue_id: IssueId::new_v7(),
            team_id: TeamId::new_v7(),
        },
        EventOrigin::User,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn outbox_push_claim_deliver_process(pool: PgPool) {
    let store = PgStore::new(pool);
    let org = OrgId::new_v7();

    let mut tx = store.begin(org).await.unwrap();
    let sync_a = tx.last_sync_id().await.unwrap();
    assert_eq!(
        tx.last_sync_id().await.unwrap(),
        sync_a,
        "same tx, same lastSyncId"
    );
    tx.outbox().push(created()).await.unwrap();
    tx.outbox().push(created()).await.unwrap();
    tx.commit().await.unwrap();

    let mut tx = store.begin(org).await.unwrap();
    assert!(tx.last_sync_id().await.unwrap() > sync_a, "monotone");
    drop(tx); // Rollback

    let mut tx = store.begin_system().await.unwrap();
    let events = tx
        .outbox()
        .claim_undelivered("search_index", 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].org_id, org);
    assert_eq!(events[0].sync_id, sync_a);
    let ids: Vec<_> = events.iter().map(|e| e.id).collect();
    tx.outbox()
        .mark_delivered("search_index", &ids)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut tx = store.begin_system().await.unwrap();
    assert!(
        tx.outbox()
            .claim_undelivered("search_index", 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        tx.outbox()
            .claim_undelivered("sync", 100)
            .await
            .unwrap()
            .len(),
        2
    );
    let both = ["search_index".to_string(), "sync".to_string()];
    assert_eq!(
        tx.outbox().mark_processed(&both).await.unwrap(),
        0,
        "sync not delivered yet"
    );
    assert_eq!(
        tx.outbox()
            .mark_processed(&["search_index".to_string()])
            .await
            .unwrap(),
        2
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn rolled_back_tx_leaves_no_event(pool: PgPool) {
    let store = PgStore::new(pool);
    let mut tx = store.begin(OrgId::new_v7()).await.unwrap();
    tx.outbox().push(created()).await.unwrap();
    drop(tx);
    let mut tx = store.begin_system().await.unwrap();
    assert!(
        tx.outbox()
            .claim_undelivered("x", 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn jobs_enqueue_claim_complete_retry(pool: PgPool) {
    let store = PgStore::new(pool);
    let now = Utc::now();

    let mut tx = store.begin_system().await.unwrap();
    let mut keyed = NewJob::now("demo", json!({"n": 1}), now);
    keyed.unique_key = Some("demo:1".into());
    assert!(tx.jobs().enqueue(keyed.clone()).await.unwrap().is_some());
    assert!(
        tx.jobs().enqueue(keyed).await.unwrap().is_none(),
        "unique_key dedups"
    );
    let later = NewJob {
        run_at: now + chrono::Duration::hours(1),
        ..NewJob::now("later", json!({}), now)
    };
    tx.jobs().enqueue(later).await.unwrap();
    tx.commit().await.unwrap();

    // Zwei Worker beanspruchen disjunkte Mengen (SKIP LOCKED).
    let mut a = store.begin_system().await.unwrap();
    let mut b = store.begin_system().await.unwrap();
    let got_a = a.jobs().claim("w1", 10, now).await.unwrap();
    let got_b = b.jobs().claim("w2", 10, now).await.unwrap();
    assert_eq!(got_a.len(), 1, "only the due job");
    assert!(got_b.is_empty(), "locked row is skipped");
    assert_eq!(got_a[0].attempts, 1);
    a.commit().await.unwrap();
    drop(b);

    let id = got_a[0].id;
    let mut tx = store.begin_system().await.unwrap();
    tx.jobs().fail(id, "boom", Some(now)).await.unwrap();
    tx.commit().await.unwrap();

    let mut tx = store.begin_system().await.unwrap();
    let again = tx.jobs().claim("w1", 10, now).await.unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].attempts, 2);
    tx.jobs().complete(again[0].id, now).await.unwrap();
    tx.commit().await.unwrap();

    let mut tx = store.begin_system().await.unwrap();
    assert!(tx.jobs().claim("w1", 10, now).await.unwrap().is_empty());
}
