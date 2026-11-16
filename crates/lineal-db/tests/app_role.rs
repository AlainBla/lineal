//! LIN-61 / S-09: die de-privilegierte Rolle `lineal_app` kann den Normalbetrieb (DML), aber
//! kein DDL. Beweist, dass `lineal_app` NICHT kann, was der Eigentuemer kann (CREATE/DROP TABLE),
//! und dass der uebliche Ablauf (Outbox, Job-Queue) als `lineal_app` weiter funktioniert.

use lineal_app::ports::Store;
use lineal_app::ports::repos::NewJob;
use lineal_db::PgStore;
use lineal_domain::ids::OrgId;
use serde_json::json;
use sqlx::PgPool;

const TEST_PW: &str = "lin61-test-app-pw-0123456789";

#[sqlx::test(migrations = "./migrations")]
async fn app_role_can_dml_but_not_ddl(pool: PgPool) {
    // `migrate` laeuft als Eigentuemer; hier provisioniert der Test die Rolle wie `lineal migrate`.
    lineal_db::ensure_app_role(&pool, TEST_PW)
        .await
        .expect("provision lineal_app");
    // Idempotent: ein zweiter Lauf (z. B. erneutes Deploy) darf nicht scheitern.
    lineal_db::ensure_app_role(&pool, TEST_PW)
        .await
        .expect("ensure_app_role is idempotent");

    // Attribute: Login ja, aber keinerlei erhoehte Rechte (kein Superuser/CREATEROLE/CREATEDB/
    // BYPASSRLS – Letzteres bindet die spaetere RLS-Folgearbeit an diese Rolle).
    let (can_login, is_super, createrole, createdb, bypassrls): (bool, bool, bool, bool, bool) =
        sqlx::query_as(
            "SELECT rolcanlogin, rolsuper, rolcreaterole, rolcreatedb, rolbypassrls \
             FROM pg_roles WHERE rolname = 'lineal_app'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(can_login, "lineal_app muss sich anmelden koennen");
    assert!(!is_super, "lineal_app darf kein Superuser sein");
    assert!(
        !createrole && !createdb && !bypassrls,
        "lineal_app darf keine erhoehten Rechte haben"
    );

    // Echte Anmeldung als lineal_app (ueber pg_hba/scram) – genau wie `serve`/`worker` verbinden.
    let app_opts = lineal_db::app_role_options((*pool.connect_options()).clone(), TEST_PW);
    let app_pool = lineal_db::pool_options(2)
        .connect_with(app_opts)
        .await
        .expect("login as lineal_app");

    // Normalbetrieb als lineal_app: Outbox zieht nextval('sync_seq') (USAGE auf Sequenz noetig),
    // Job-Queue nutzt INSERT/UPDATE und FOR UPDATE SKIP LOCKED.
    let store = PgStore::new(app_pool.clone());
    let org = OrgId::new_v7();
    let mut tx = store.begin(org).await.expect("begin as lineal_app");
    let sync = tx.last_sync_id().await.expect("nextval('sync_seq')");
    assert!(sync >= 1);
    tx.commit().await.unwrap();

    let mut tx = store.begin_system().await.unwrap();
    tx.jobs()
        .enqueue(NewJob::now("lin61", json!({}), chrono::Utc::now()))
        .await
        .expect("INSERT into job as lineal_app");
    tx.commit().await.unwrap();

    let mut tx = store.begin_system().await.unwrap();
    let claimed = tx
        .jobs()
        .claim("w", 10, chrono::Utc::now())
        .await
        .expect("claim (UPDATE) as lineal_app");
    assert_eq!(claimed.len(), 1, "der eingereihte Job wird beansprucht");
    tx.commit().await.unwrap();

    // DDL ist verboten: CREATE TABLE scheitert mit insufficient_privilege (42501).
    let create = sqlx::query("CREATE TABLE lin61_evil (x int)")
        .execute(&app_pool)
        .await;
    assert_eq!(
        create
            .expect_err("CREATE TABLE muss scheitern")
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("42501"),
        "CREATE TABLE muss mit insufficient_privilege (42501) scheitern"
    );

    // DROP einer bestehenden Tabelle scheitert mangels Ownership.
    let drop = sqlx::query("DROP TABLE job").execute(&app_pool).await;
    assert!(
        drop.is_err(),
        "DROP TABLE job muss als lineal_app scheitern (keine Ownership)"
    );

    app_pool.close().await;
}
