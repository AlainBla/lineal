#![allow(clippy::unwrap_used, clippy::expect_used)]
//! B1.12 / W3D-12: Postgres-Notice "there is already a transaction in progress".
//!
//! Ursache (sqlx 0.8.6): `Transaction::begin` ist nicht abbruchsicher. Wird die Future abgebrochen
//! (Client trennt, Request-Timeout), nachdem `BEGIN` gesendet wurde, aber bevor die Antwort
//! verarbeitet ist, bleibt `transaction_depth` bei 0 und es wird kein `ROLLBACK` eingereiht. Der
//! Server steht dann auf "idle in transaction", die Verbindung geht in den Pool zurück, und der
//! nächste `BEGIN` auf ihr erzeugt die Notice; Schreibzugriffe zwischendurch (außerhalb einer
//! Transaktion) landen in der fremden, nie committeten Transaktion.
//!
//! `PgStore::begin` führt `BEGIN` deshalb in einer eigenen Task aus, die nicht mit dem Aufrufer
//! abgebrochen wird.

use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_domain::ids::OrgId;
use sqlx::PgPool;
use std::future::Future;
use std::task::{Context, Waker};
use std::time::Duration;

/// Zählt Verbindungen im Zustand "idle in transaction" über eine **eigene** Verbindung (nicht aus
/// dem Pool: eine verwaiste Pool-Verbindung würde die Abfrage sonst selbst in ihrer offenen
/// Transaktion ausführen und sich dadurch verstecken).
async fn idle_in_transaction(pool: &PgPool) -> i64 {
    use sqlx::Connection;
    let mut conn = sqlx::PgConnection::connect_with(&pool.connect_options())
        .await
        .unwrap();
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND state LIKE 'idle in transaction%' \
           AND pid <> pg_backend_pid()",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap()
}

/// Die Ursache, deterministisch und direkt auf sqlx: `BEGIN` wird gesendet, die Future danach
/// (vor der Antwort) verworfen; die zurückgegebene Verbindung steht weiter in einer Transaktion.
/// Schlägt dieser Test eines Tages fehl, ist sqlx behoben und der Umweg in `PgStore` entbehrlich.
#[sqlx::test(migrations = "./migrations")]
async fn sqlx_begin_dropped_after_sending_begin_leaves_the_server_in_a_transaction(pool: PgPool) {
    use sqlx::Connection;
    let mut conn = pool.acquire().await.unwrap();
    {
        let fut = conn.begin();
        let mut fut = std::pin::pin!(fut);
        let mut cx = Context::from_waker(Waker::noop());
        // Erster Poll: BEGIN wird geschrieben, die Antwort fehlt noch.
        assert!(fut.as_mut().poll(&mut cx).is_pending());
    }
    drop(conn);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(idle_in_transaction(&pool).await, 1);
}

/// `PgStore::begin`, nach k Polls abgebrochen (zwischen den Polls bekommt der IO-Treiber Zeit):
/// irgendein k trifft das Fenster "BEGIN gesendet, Antwort offen". Danach darf keine Verbindung in
/// einer offenen Transaktion zurückbleiben.
#[sqlx::test(migrations = "./migrations")]
async fn cancelled_store_begin_never_leaves_an_open_transaction(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let org = OrgId::new_v7();
    for k in 1..=6 {
        // Eine warme, freie Verbindung: dann ist k = 2 genau "Ping fertig, BEGIN gesendet".
        drop(pool.acquire().await.unwrap());
        tokio::time::sleep(Duration::from_millis(20)).await;
        {
            let fut = store.begin(org);
            let mut fut = std::pin::pin!(fut);
            let mut cx = Context::from_waker(Waker::noop());
            for _ in 0..k {
                if let std::task::Poll::Ready(r) = fut.as_mut().poll(&mut cx) {
                    drop(r.unwrap());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        } // hier wird die Future abgebrochen (verworfen)
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            idle_in_transaction(&pool).await,
            0,
            "a begin cancelled after {k} polls left an open transaction"
        );
        // Eine verwaiste Verbindung läge jetzt noch im Pool: einmal benutzen und zurückgeben.
        let mut c = pool.acquire().await.unwrap();
        sqlx::query("SELECT 1").execute(&mut *c).await.unwrap();
    }
}

/// Normalfall bleibt: begin/commit funktionieren und hinterlassen nichts.
#[sqlx::test(migrations = "./migrations")]
async fn begin_and_commit_leave_no_open_transaction(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    for _ in 0..20 {
        let tx = store.begin(OrgId::new_v7()).await.unwrap();
        tx.commit().await.unwrap();
    }
    assert_eq!(idle_in_transaction(&pool).await, 0);
}
