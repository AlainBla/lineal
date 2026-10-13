#![allow(clippy::unwrap_used, clippy::expect_used)]
//! B1.13: SQLSTATE 40P01 / 23503 / 23505 werden zentral abgebildet (nie `internal`).

use lineal_app::ErrorKind;
use lineal_db::store::map_db_error;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn real_deadlock_maps_to_retryable_conflict(pool: PgPool) {
    sqlx::query("CREATE TABLE dl (id int PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO dl VALUES (1),(2)")
        .execute(&pool)
        .await
        .unwrap();
    let mut a = pool.begin().await.unwrap();
    let mut b = pool.begin().await.unwrap();
    sqlx::query("SELECT * FROM dl WHERE id=1 FOR UPDATE")
        .execute(&mut *a)
        .await
        .unwrap();
    sqlx::query("SELECT * FROM dl WHERE id=2 FOR UPDATE")
        .execute(&mut *b)
        .await
        .unwrap();
    let fa = async {
        let r = sqlx::query("SELECT * FROM dl WHERE id=2 FOR UPDATE")
            .execute(&mut *a)
            .await;
        (r, a)
    };
    let fb = async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let r = sqlx::query("SELECT * FROM dl WHERE id=1 FOR UPDATE")
            .execute(&mut *b)
            .await;
        (r, b)
    };
    let ((ra, _a), (rb, _b)) = tokio::join!(fa, fb);
    let errs: Vec<_> = [ra, rb].into_iter().filter_map(Result::err).collect();
    assert_eq!(errs.len(), 1, "genau ein Deadlock-Opfer erwartet");
    let e = map_db_error(&errs[0]);
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert!(e.is_retryable());
}

#[sqlx::test(migrations = "./migrations")]
async fn fk_and_unique_violations_map_to_domain_errors(pool: PgPool) {
    sqlx::query("CREATE TABLE par (id int PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE chi (id int PRIMARY KEY, p int REFERENCES par(id))")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO par VALUES (1)")
        .execute(&pool)
        .await
        .unwrap();
    // Einfügen mit fehlendem Eltern-Objekt: not_found.
    let e = sqlx::query("INSERT INTO chi VALUES (1, 99)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(map_db_error(&e).kind, ErrorKind::NotFound);
    // Löschen eines noch referenzierten Objekts: conflict.
    sqlx::query("INSERT INTO chi VALUES (1, 1)")
        .execute(&pool)
        .await
        .unwrap();
    let e = sqlx::query("DELETE FROM par WHERE id=1")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(map_db_error(&e).kind, ErrorKind::Conflict);
    // Unique: conflict.
    let e = sqlx::query("INSERT INTO chi VALUES (1, 1)")
        .execute(&pool)
        .await
        .unwrap_err();
    let m = map_db_error(&e);
    assert_eq!(m.kind, ErrorKind::Conflict);
    assert!(!m.is_retryable());
}
