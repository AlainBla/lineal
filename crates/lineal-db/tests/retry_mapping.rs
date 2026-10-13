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

/// B1.12: Eine Verbindung mit offener (dem Client unbekannter) Transaktion wird beim Entnehmen
/// aus dem Pool verworfen, statt „there is already a transaction in progress“ auszulösen.
#[sqlx::test(migrations = "./migrations")]
async fn pool_discards_connection_with_open_transaction(pool: PgPool) {
    let opts = (*pool.connect_options()).clone();
    let p = lineal_db::pool_options(1).connect_with(opts).await.unwrap();
    let pid_before: i32 = {
        let mut c = p.acquire().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *c)
            .await
            .unwrap();
        // Rohes BEGIN am sqlx-Zustand vorbei: wie eine abgebrochene `begin()`-Anfrage.
        sqlx::raw_sql("BEGIN").execute(&mut *c).await.unwrap();
        pid
    };
    let mut c = p.acquire().await.unwrap();
    let pid_after: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_ne!(
        pid_before, pid_after,
        "verschmutzte Verbindung wurde wiederverwendet"
    );
    assert!(in_clean_state(&mut c).await);
    drop(c);
    // Saubere Verbindungen werden weiterverwendet.
    let pid_a = {
        let mut c = p.acquire().await.unwrap();
        sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&mut *c)
            .await
            .unwrap()
    };
    let pid_b = {
        let mut c = p.acquire().await.unwrap();
        sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&mut *c)
            .await
            .unwrap()
    };
    assert_eq!(pid_a, pid_b);
}

/// Simples Protokoll: `now()` (Transaktionsbeginn) = `statement_timestamp()` nur ohne offene
/// Transaktion.
async fn in_clean_state(c: &mut sqlx::PgConnection) -> bool {
    use sqlx::Row;
    sqlx::raw_sql("SELECT now() = statement_timestamp()")
        .fetch_one(&mut *c)
        .await
        .unwrap()
        .get::<bool, _>(0)
}

/// B1.12: Wird `PgStore::begin` mitten im `BEGIN` abgebrochen (Client-Abbruch), bleibt keine
/// offene Server-Transaktion auf der Pool-Verbindung zurück.
#[sqlx::test(migrations = "./migrations")]
async fn cancelled_begin_leaves_no_open_transaction(pool: PgPool) {
    use lineal_app::ports::Store;
    let store = lineal_db::PgStore::new(pool.clone());
    // Verbindung vorwärmen (idle im Pool).
    drop(pool.acquire().await.unwrap());
    let org = lineal_domain::ids::OrgId::nil();
    for round in 0..30_u64 {
        // Anfrage nach unterschiedlich kurzer Zeit abbrechen (mitten in `BEGIN`).
        let _ = tokio::time::timeout(
            std::time::Duration::from_micros(round * 50),
            store.begin(org),
        )
        .await;
        // Dem Hintergrundtask (falls vorhanden) Zeit zum Aufräumen geben.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let mut c = pool.acquire().await.unwrap();
        assert!(
            in_clean_state(&mut c).await,
            "offene Transaktion auf Pool-Verbindung"
        );
    }
}
