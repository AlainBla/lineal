#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Paket 7: SLA wird bei einer Prioritätsänderung sofort neu berechnet (B7.11).

use lineal_app::ports::Store;
use lineal_app::usecases::issue::{CreateIssueInput, UpdateIssueInput};
use lineal_app::usecases::team_extra::SetTeamSlaInput;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

/// B7.10: Liegen zwei Nachbarn in der manuellen Sortierung enger als 1e-6, nummeriert der Job das
/// Team mit Abstand 1 neu (Reihenfolge bleibt, `updated_at` bleibt, Sync-Event je Issue).
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sort_order_is_renumbered_when_gaps_collapse(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    let alice = ctx_for(SeedUser::Alice);
    let mut ids = Vec::new();
    for t in ["First", "Second", "Third"] {
        let i = app
            .create_issue(
                &alice,
                CreateIssueInput {
                    team: "ENG".into(),
                    title: Some(format!("Sort {t}")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        ids.push(i.issue.id.to_string());
    }
    // Die ersten beiden fast identisch (Abstand 5e-10), das dritte weit weg.
    for (id, v) in ids.iter().zip([-2.0_f64, -1.9999999995, 7.0]) {
        sqlx::query("UPDATE issue SET sort_order = $2 WHERE id = $1::uuid")
            .bind(id)
            .bind(v)
            .execute(&pool)
            .await
            .unwrap();
    }
    let updated_before: Vec<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT updated_at FROM issue WHERE title LIKE 'Sort %' ORDER BY title",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let stats = app.run_sort_renumber(chrono::Utc::now()).await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    let rows: Vec<(String, f64)> = sqlx::query_as(
        "SELECT title, sort_order FROM issue WHERE title LIKE 'Sort %' ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let titles: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(titles, ["Sort First", "Sort Second", "Sort Third"]);
    assert!(
        rows.windows(2).all(|w| w[1].1 - w[0].1 >= 1.0 - 1e-9),
        "gaps of at least 1: {rows:?}"
    );
    let updated_after: Vec<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT updated_at FROM issue WHERE title LIKE 'Sort %' ORDER BY title",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(updated_before, updated_after, "no activity signal");
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_event WHERE payload->>'type' = 'IssueUpdated' \
         AND payload::text LIKE '%sortOrder%' AND origin = 'system'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(events >= 1, "the new order is distributed via sync");
    // Zweiter Lauf: nichts mehr zu tun.
    assert_eq!(
        app.run_sort_renumber(chrono::Utc::now())
            .await
            .unwrap()
            .processed,
        0
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn priority_change_recomputes_the_sla_synchronously(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    let alice = ctx_for(SeedUser::Alice);
    app.set_team_sla(
        &alice,
        SetTeamSlaInput {
            team: "ENG".into(),
            enabled: Some(true),
            work_week: None,
        },
    )
    .await
    .unwrap();
    // Priorität "Medium" fällt unter die Regel "Others" (entfernt SLA): kein SLA.
    let issue = app
        .create_issue(
            &alice,
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("SLA follows priority".into()),
                priority: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = issue.issue.id.to_string();
    let breach = || async {
        sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
            "SELECT sla_breaches_at FROM issue WHERE id = $1::uuid",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert!(breach().await.is_none());
    let set_priority = |p: i64| {
        let (app, alice, id) = (app.clone(), alice.clone(), id.clone());
        async move {
            app.update_issue(
                &alice,
                UpdateIssueInput {
                    id,
                    priority: Some(p),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
    };
    // Priorität -> Urgent: SLA (24 h) steht sofort, ohne dass der Job lief.
    set_priority(1).await;
    let urgent = breach().await.expect("SLA is set synchronously");
    // Priorität -> High: andere Regel (1 Woche), Frist verschiebt sich sofort.
    set_priority(2).await;
    let high = breach().await.expect("SLA stays set");
    assert!(high > urgent, "{high} should be after {urgent}");
    // Priorität -> Medium: "Others" entfernt das SLA sofort.
    set_priority(3).await;
    assert!(breach().await.is_none());
}
