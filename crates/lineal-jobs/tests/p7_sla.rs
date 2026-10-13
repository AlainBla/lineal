#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Paket 7: SLA wird bei einer Prioritätsänderung sofort neu berechnet (B7.11).

use lineal_app::ports::Store;
use lineal_app::usecases::issue::{CreateIssueInput, UpdateIssueInput};
use lineal_app::usecases::team_extra::SetTeamSlaInput;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

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
