#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Sicherheit F-05 (Rechteprüfung vor der Zeilensperre, TOCTOU) und F-08 (Team-Purge öffnet
//! private Initiativen). Das Rennen wird deterministisch nachgestellt: eine rohe Transaktion
//! hält die Zeile gesperrt, der Akteur (Bob, nur ENG) prüft sein Recht auf dem alten Stand und
//! blockiert an der Sperre; danach wird das Objekt in ein privates Team verschoben (Commit).

use chrono::{Duration, Utc};
use lineal_app::ports::Store;
use lineal_app::usecases::document::{ParentRefs, SaveDocumentInput};
use lineal_app::usecases::initiative::{ListInitiativesInput, SaveInitiativeInput};
use lineal_app::usecases::issue::{CreateIssueInput, UpdateIssueInput};
use lineal_app::usecases::team::CreateTeamInput;
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::TeamVisibility;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::future::Future;
use std::sync::Arc;
use uuid::Uuid;

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}
fn bob() -> RequestCtx {
    ctx_for(SeedUser::Bob)
}

async fn setup(pool: &PgPool) -> Arc<App> {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    test_app(Arc::new(store))
}

async fn private_team(app: &App, key: &str) -> Uuid {
    app.create_team(
        &alice(),
        CreateTeamInput {
            name: format!("Private {key}"),
            key: Some(key.into()),
            visibility: Some(TeamVisibility::Private),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .team
    .id
    .0
}

async fn new_issue(app: &App) -> (Uuid, String) {
    let v = app
        .create_issue(
            &alice(),
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Original".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    (v.issue.id.0, v.issue.identifier.clone())
}

/// Hält `lock_sql` (bindet `$1`) in einer rohen Transaktion offen, startet `op` (blockiert an der
/// Sperre), führt dann `move_sql` (bindet `$1`, `$2` = Zielteam) aus und committet.
async fn race<T, Fut>(
    pool: &PgPool,
    lock_sql: &str,
    move_sql: &str,
    id: Uuid,
    target: Uuid,
    op: impl FnOnce() -> Fut,
) -> T
where
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let mut raw = pool.begin().await.unwrap();
    sqlx::query(lock_sql)
        .bind(id)
        .execute(&mut *raw)
        .await
        .unwrap();
    let handle = tokio::spawn(op());
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    sqlx::query(move_sql)
        .bind(id)
        .bind(target)
        .execute(&mut *raw)
        .await
        .unwrap();
    raw.commit().await.unwrap();
    handle.await.unwrap()
}

const LOCK_ISSUE: &str = "SELECT id FROM issue WHERE id = $1 FOR UPDATE";
// Zielstatus: erster Status des privaten Teams (Trigger setzt Kennung und Statustyp).
const MOVE_ISSUE: &str = "UPDATE issue SET team_id = $2, number = 100, \
    state_id = (SELECT id FROM workflow_state WHERE team_id = $2 ORDER BY position LIMIT 1) \
    WHERE id = $1";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn update_issue_rechecks_rights_after_lock(pool: PgPool) {
    let app = setup(&pool).await;
    let prv = private_team(&app, "PRV").await;
    let (id, ident) = new_issue(&app).await;
    let a = app.clone();
    let res = race(&pool, LOCK_ISSUE, MOVE_ISSUE, id, prv, move || async move {
        a.update_issue(
            &bob(),
            UpdateIssueInput {
                id: ident,
                title: Some("Gekapert".into()),
                ..Default::default()
            },
        )
        .await
    })
    .await;
    assert_eq!(res.err().map(|e| e.kind), Some(ErrorKind::NotFound));
    let title: String = sqlx::query_scalar("SELECT title FROM issue WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(title, "Original");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn archive_issue_rechecks_rights_after_lock(pool: PgPool) {
    let app = setup(&pool).await;
    let prv = private_team(&app, "PRV").await;
    let (id, ident) = new_issue(&app).await;
    let a = app.clone();
    let res = race(&pool, LOCK_ISSUE, MOVE_ISSUE, id, prv, move || async move {
        a.archive_issue(&bob(), &ident).await
    })
    .await;
    assert_eq!(res.err().map(|e| e.kind), Some(ErrorKind::NotFound));
    let archived: bool =
        sqlx::query_scalar("SELECT archived_at IS NOT NULL FROM issue WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!archived);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn delete_issue_rechecks_rights_after_lock(pool: PgPool) {
    let app = setup(&pool).await;
    let prv = private_team(&app, "PRV").await;
    let (id, ident) = new_issue(&app).await;
    let a = app.clone();
    let res = race(&pool, LOCK_ISSUE, MOVE_ISSUE, id, prv, move || async move {
        a.delete_issue(&bob(), &ident, false).await
    })
    .await;
    assert_eq!(res.err().map(|e| e.kind), Some(ErrorKind::NotFound));
    let trashed: bool = sqlx::query_scalar("SELECT trashed FROM issue WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!trashed);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn update_document_rechecks_rights_after_lock(pool: PgPool) {
    let app = setup(&pool).await;
    let prv = private_team(&app, "PRV").await;
    let doc = app
        .save_document(
            &alice(),
            SaveDocumentInput {
                title: Some("Wiki".into()),
                content: Some("geheim".into()),
                parent: ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = doc.doc.id.0;
    let a = app.clone();
    let res = race(
        &pool,
        "SELECT id FROM document_content WHERE owner_id = $1 FOR UPDATE",
        "UPDATE document SET team_id = $2 WHERE id = $1",
        id,
        prv,
        move || async move {
            a.save_document(
                &bob(),
                SaveDocumentInput {
                    id: Some(id.to_string()),
                    title: Some("Gekapert".into()),
                    ..Default::default()
                },
            )
            .await
        },
    )
    .await;
    assert_eq!(res.err().map(|e| e.kind), Some(ErrorKind::NotFound));
    let title: String = sqlx::query_scalar("SELECT title FROM document WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(title, "Wiki");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_initiative_stays_invisible_after_team_purge(pool: PgPool) {
    let app = setup(&pool).await;
    private_team(&app, "INI").await;
    let init = app
        .save_initiative(
            &alice(),
            SaveInitiativeInput {
                name: Some("SecretInit".into()),
                description: Some("INIT-SECRET-DESC".into()),
                lead_team: Some(Some("INI".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let init_id = init.initiative.id.0.to_string();
    // Vorher: für Bob unsichtbar.
    let err = app.get_initiative(&bob(), &init_id).await.err().unwrap();
    assert_eq!(err.kind, ErrorKind::NotFound);

    app.delete_team(&alice(), "INI", None).await.unwrap();
    let stats = app
        .run_team_purge(Utc::now() + Duration::days(40))
        .await
        .unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(stats.processed, 1);

    // Nachher: weiterhin nicht sichtbar (die Initiative wird mit dem Team entfernt, nicht geöffnet).
    let err = app.get_initiative(&bob(), &init_id).await.err().unwrap();
    assert_eq!(err.kind, ErrorKind::NotFound);
    let (_, views) = app
        .list_initiatives(&bob(), ListInitiativesInput::default())
        .await
        .unwrap();
    assert!(views.iter().all(|v| v.initiative.name != "SecretInit"));
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM initiative WHERE name = 'SecretInit'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}
