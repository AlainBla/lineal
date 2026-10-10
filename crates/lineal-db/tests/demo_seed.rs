#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP10: `seed::demo` legt den Demo-Workspace über die Use-Cases an.

use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn demo_seed_creates_a_realistic_workspace(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    seed::demo::run(&app).await.unwrap();

    assert!(count(&pool, "SELECT count(*) FROM issue").await >= 28);
    for key in ["ENG", "WEB", "DES"] {
        let n = count(
            &pool,
            &format!(
                "SELECT count(*) FROM issue i JOIN team t ON t.id = i.team_id WHERE t.key = '{key}'"
            ),
        )
        .await;
        assert!(n >= 7, "{key}: {n}");
    }
    // Nummern pro Team lückenlos.
    let gaps = count(
        &pool,
        "SELECT count(*) FROM team t WHERE t.last_issue_number <> (SELECT count(*) FROM issue i WHERE i.team_id = t.id)",
    )
    .await;
    assert_eq!(gaps, 0);
    assert!(
        count(
            &pool,
            "SELECT count(*) FROM issue WHERE parent_id IS NOT NULL"
        )
        .await
            >= 4
    );
    assert!(count(&pool, "SELECT count(*) FROM issue_to_label").await >= 15);
    assert!(count(&pool, "SELECT count(*) FROM comment").await >= 8);
    assert!(count(&pool, "SELECT count(*) FROM project").await >= 2);
    assert!(count(&pool, "SELECT count(*) FROM project_milestone").await >= 4);
    assert!(
        count(
            &pool,
            "SELECT count(*) FROM issue WHERE cycle_id IS NOT NULL"
        )
        .await
            >= 4
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM template").await, 2);
    assert_eq!(count(&pool, "SELECT count(*) FROM document").await, 2);
}
