//! Contract-Tests für die Gruppe `inbox` gegen echte Datenbank (Seed `acme`).

use crate::dispatch::dispatch;
use chrono::Duration;
use lineal_app::ports::Store;
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::ids::IssueId;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct Env {
    app: Arc<App>,
    pool: PgPool,
    d: SeedData,
}

async fn setup(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    Env {
        app: test_app(Arc::new(store)),
        pool,
        d: SeedData::acme(),
    }
}

async fn call(
    env: &Env,
    who: SeedUser,
    tool: &str,
    args: Value,
) -> Result<Value, lineal_app::AppError> {
    dispatch(&env.app, &ctx_for(who), tool, args)
        .await
        .expect("known tool")
}

impl Env {
    async fn issue(&self, team: lineal_domain::ids::TeamId, slot: u128, number: i32) -> IssueId {
        let id = IssueId::new_v7();
        sqlx::query(
            "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) \
             VALUES ($1,$2,$3,$4,'Login schlägt bei SSO-Usern fehl',$5,$6)",
        )
        .bind(id.0)
        .bind(self.d.org.0)
        .bind(team.0)
        .bind(number)
        .bind(seed_workflow_state_ids(slot)[1].0)
        .bind(self.d.alice.0)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    /// Notification `age_minutes` alt (je älter, desto kleiner `created_at`).
    async fn notify(
        &self,
        user: lineal_domain::ids::UserId,
        issue: IssueId,
        kind: &str,
        age_minutes: i64,
    ) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO notification (id, organization_id, user_id, type, category, target_type, actor_id, \
             issue_id, dedup_key, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,'assignments','issue',$5,$6,$7, now() - ($8 || ' minutes')::interval, \
                     now() - ($8 || ' minutes')::interval)",
        )
        .bind(id)
        .bind(self.d.org.0)
        .bind(user.0)
        .bind(kind)
        .bind(self.d.carol.0)
        .bind(issue.0)
        .bind(id.to_string())
        .bind(age_minutes.to_string())
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn get_notifications_shape_order_paging_and_filters(pool: PgPool) {
    let env = setup(pool).await;
    let issue = env.issue(env.d.eng, 1, 124).await;
    let oldest = env.notify(env.d.bob, issue, "issueAssignedToYou", 30).await;
    let middle = env.notify(env.d.bob, issue, "issueNewComment", 20).await;
    let newest = env.notify(env.d.bob, issue, "issueMention", 10).await;
    env.notify(env.d.alice, issue, "issueMention", 5).await; // fremd

    let out = call(&env, SeedUser::Bob, "get_notifications", json!({}))
        .await
        .unwrap();
    let ids: Vec<&str> = out["notifications"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![newest.to_string(), middle.to_string(), oldest.to_string()]
    );
    assert_eq!(out["hasNextPage"], false);
    assert!(out.get("cursor").is_none());
    let first = &out["notifications"][0];
    assert_eq!(first["type"], "issueMention");
    assert_eq!(first["category"], "assignments");
    assert_eq!(first["title"], "Login schlägt bei SSO-Usern fehl");
    assert_eq!(first["subtitle"], "Carol Example mentioned you in ENG-124");
    assert!(
        first["url"]
            .as_str()
            .unwrap()
            .ends_with("/acme/issue/ENG-124")
    );
    assert_eq!(first["issue"]["id"], "ENG-124");
    assert_eq!(first["issue"]["uuid"], issue.to_string());
    assert_eq!(first["actor"]["id"], env.d.carol.to_string());
    assert_eq!(first["actor"]["name"], "Carol Example");
    assert!(
        first["project"].is_null() && first["initiative"].is_null() && first["document"].is_null()
    );
    assert_eq!(first["isPriority"], false);
    assert!(
        first["readAt"].is_null()
            && first["archivedAt"].is_null()
            && first["snoozedUntilAt"].is_null()
    );
    assert!(first["createdAt"].as_str().unwrap().ends_with('Z'));

    // Paginierung per Cursor.
    let p1 = call(
        &env,
        SeedUser::Bob,
        "get_notifications",
        json!({ "limit": 2 }),
    )
    .await
    .unwrap();
    assert_eq!(p1["notifications"].as_array().unwrap().len(), 2);
    assert_eq!(p1["hasNextPage"], true);
    let cursor = p1["cursor"].as_str().unwrap().to_owned();
    let p2 = call(
        &env,
        SeedUser::Bob,
        "get_notifications",
        json!({ "limit": 2, "cursor": cursor }),
    )
    .await
    .unwrap();
    assert_eq!(p2["notifications"].as_array().unwrap().len(), 1);
    assert_eq!(p2["notifications"][0]["id"], oldest.to_string());
    assert_eq!(p2["hasNextPage"], false);
    // Cursor eines anderen Filters wird abgelehnt.
    let err = call(
        &env,
        SeedUser::Bob,
        "get_notifications",
        json!({ "limit": 2, "cursor": cursor, "unreadOnly": true }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);

    // unreadOnly, archiviert und gesnoozt.
    sqlx::query("UPDATE notification SET read_at = now() WHERE id = $1")
        .bind(middle)
        .execute(&env.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE notification SET archived_at = now() WHERE id = $1")
        .bind(oldest)
        .execute(&env.pool)
        .await
        .unwrap();
    let all = call(&env, SeedUser::Bob, "get_notifications", json!({}))
        .await
        .unwrap();
    assert_eq!(
        all["notifications"].as_array().unwrap().len(),
        2,
        "archivierte nie"
    );
    let unread = call(
        &env,
        SeedUser::Bob,
        "get_notifications",
        json!({ "unreadOnly": true }),
    )
    .await
    .unwrap();
    assert_eq!(unread["notifications"].as_array().unwrap().len(), 1);
    sqlx::query("UPDATE notification SET snoozed_until_at = timestamptz '2026-10-10 07:15:00+00' + interval '1 hour' WHERE id = $1")
        .bind(newest)
        .execute(&env.pool)
        .await
        .unwrap();
    let snoozed = call(&env, SeedUser::Bob, "get_notifications", json!({}))
        .await
        .unwrap();
    assert_eq!(
        snoozed["notifications"].as_array().unwrap().len(),
        1,
        "gesnoozte erst nach Ablauf"
    );
    sqlx::query("UPDATE notification SET snoozed_until_at = timestamptz '2026-10-10 07:15:00+00' - interval '1 minute' WHERE id = $1")
        .bind(newest)
        .execute(&env.pool)
        .await
        .unwrap();
    let resurfaced = call(&env, SeedUser::Bob, "get_notifications", json!({}))
        .await
        .unwrap();
    assert_eq!(resurfaced["notifications"].as_array().unwrap().len(), 2);

    // Schema-Validierung greift vor dem Handler.
    let err = call(
        &env,
        SeedUser::Bob,
        "get_notifications",
        json!({ "limit": 1000 }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn notifications_of_private_team_issues_are_hidden_from_non_members(pool: PgPool) {
    let env = setup(pool).await;
    let issue = env.issue(env.d.des, 3, 7).await;
    env.notify(env.d.alice, issue, "issueMention", 1).await; // Alice sieht DES nicht
    env.notify(env.d.bob, issue, "issueMention", 1).await; // Bob ist Mitglied
    let alice = call(&env, SeedUser::Alice, "get_notifications", json!({}))
        .await
        .unwrap();
    assert!(alice["notifications"].as_array().unwrap().is_empty());
    let bob = call(&env, SeedUser::Bob, "get_notifications", json!({}))
        .await
        .unwrap();
    assert_eq!(bob["notifications"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn mark_notification_read_snooze_and_errors(pool: PgPool) {
    let env = setup(pool).await;
    let issue = env.issue(env.d.eng, 1, 124).await;
    let id = env
        .notify(env.d.bob, issue, "issueAssignedToYou", 5)
        .await
        .to_string();

    let read = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": id, "read": true }),
    )
    .await
    .unwrap();
    assert_eq!(read["id"], id);
    assert!(read["readAt"].is_string());
    let first_read = read["readAt"].clone();
    let again = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": id, "read": true }),
    )
    .await
    .unwrap();
    assert_eq!(again["readAt"], first_read, "bereits gelesen → unverändert");
    let unread = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": id, "read": false }),
    )
    .await
    .unwrap();
    assert!(unread["readAt"].is_null());

    // Snooze: danach nicht mehr in get_notifications; null beendet es.
    let until = (env.app.clock.now() + Duration::days(2))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let snoozed = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": id, "snoozedUntilAt": until }),
    )
    .await
    .unwrap();
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(snoozed["snoozedUntilAt"].as_str().unwrap()).unwrap(),
        chrono::DateTime::parse_from_rfc3339(&until).unwrap()
    );
    let list = call(&env, SeedUser::Bob, "get_notifications", json!({}))
        .await
        .unwrap();
    assert!(list["notifications"].as_array().unwrap().is_empty());
    let woke = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": id, "snoozedUntilAt": null }),
    )
    .await
    .unwrap();
    assert!(woke["snoozedUntilAt"].is_null());
    let list = call(&env, SeedUser::Bob, "get_notifications", json!({}))
        .await
        .unwrap();
    assert_eq!(list["notifications"].as_array().unwrap().len(), 1);

    // Fehler: Vergangenheit, kein Parameter, fremde und unbekannte IDs.
    let past = (env.app.clock.now() - Duration::hours(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let err = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": id, "snoozedUntilAt": past }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    let err = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": id }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    let err = call(
        &env,
        SeedUser::Alice,
        "mark_notification",
        json!({ "id": id, "read": true }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    let err = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": Uuid::now_v7().to_string(), "read": true }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    let err = call(
        &env,
        SeedUser::Bob,
        "mark_notification",
        json!({ "id": "n-1", "read": true }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}
