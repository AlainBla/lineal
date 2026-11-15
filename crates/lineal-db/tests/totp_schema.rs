#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-70 T1: Schema-Smoke für die TOTP-Tabellen `user_totp`, `user_recovery_code`
//! und `user_totp_grace` — Migration 5200 legt sie an, und je eine Zeile pro Nutzer
//! lässt sich einfügen bzw. verletzt die erwarteten Constraints.

use chrono::Utc;
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, seed};
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn totp_tables_accept_one_row_per_user(pool: PgPool) {
    let d = SeedData::acme();
    let store = PgStore::new(pool.clone());
    {
        let mut tx = store.begin(d.org).await.unwrap();
        seed::run(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    let org = d.org.0;
    let alice = d.alice.0;
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO user_totp (user_id, organization_id, secret_enc, dek_wrapped, key_backend, key_version, confirmed_at, last_used_step) \
         VALUES ($1,$2,$3,$4,'app',1,$5,$6)",
    )
    .bind(alice)
    .bind(org)
    .bind(vec![1u8, 2, 3])
    .bind(vec![4u8, 5, 6])
    .bind(now)
    .bind(57_600_000_i64)
    .execute(&pool)
    .await
    .unwrap();

    // Primärschlüssel user_id: zweite Zeile für denselben Nutzer schlägt fehl.
    let dup = sqlx::query(
        "INSERT INTO user_totp (user_id, organization_id, secret_enc, dek_wrapped, key_backend, key_version) \
         VALUES ($1,$2,$3,$4,'app',1)",
    )
    .bind(alice)
    .bind(org)
    .bind(vec![9u8])
    .bind(vec![9u8])
    .execute(&pool)
    .await;
    assert!(dup.is_err(), "user_totp ist pro Nutzer eindeutig");

    for i in 0..3 {
        sqlx::query(
            "INSERT INTO user_recovery_code (user_id, organization_id, code_hash) VALUES ($1,$2,$3)",
        )
        .bind(alice)
        .bind(org)
        .bind(vec![i as u8; 32])
        .execute(&pool)
        .await
        .unwrap();
    }

    sqlx::query(
        "INSERT INTO user_totp_grace (user_id, organization_id, started_at, logins_since) VALUES ($1,$2,$3,1)",
    )
    .bind(alice)
    .bind(org)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let codes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_recovery_code WHERE user_id = $1")
            .bind(alice)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(codes, 3);
}
