#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Paket 7: Importintegrität (Zyklen, Zähler, Lease, Parallelität), Fake-Quelle.

mod common;

use common::*;
use lineal_app::usecases::import::ImportStartInput;
use lineal_import::SourceEntity;
use lineal_import::engine::{EngineConfig, RunOutcome};
use serde_json::json;
use sqlx::PgPool;

/// B7.01: Parent-Zyklen aus der Quelle werden verworfen, Bericht enthält Warnungen.
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn parent_cycles_are_dropped_with_report(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    let p = |id: &str, parent: &str| {
        source.patch(
            SourceEntity::Issues,
            id,
            &json!({ "parent": { "id": parent } }),
        )
    };
    // Zwei-Zyklus und Drei-Zyklus.
    assert!(p("i-ENG-20", "i-ENG-22") && p("i-ENG-22", "i-ENG-20"));
    assert!(p("i-ENG-23", "i-ENG-24") && p("i-ENG-24", "i-ENG-25") && p("i-ENG-25", "i-ENG-23"));
    let (run, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    // Kein Issue ist sein eigener Vorfahr.
    let cyclic = e
        .count(
            "WITH RECURSIVE w(start, cur, d) AS ( \
               SELECT id, parent_id, 1 FROM issue WHERE parent_id IS NOT NULL \
               UNION ALL SELECT w.start, i.parent_id, w.d + 1 FROM issue i JOIN w ON i.id = w.cur \
               WHERE i.parent_id IS NOT NULL AND w.d < 50) \
             SELECT count(*) FROM w WHERE cur = start",
        )
        .await;
    assert_eq!(cyclic, 0);
    // Genau eine Kante je Zyklus wurde verworfen (Warnung im Bericht).
    let warnings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM import_issue WHERE import_run_id = $1 AND message LIKE '%would create a cycle%'",
    )
    .bind(run)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(warnings, 2);
}

const BEHIND_SQL: &str = "SELECT count(*) FROM team t WHERE last_issue_number < \
     COALESCE((SELECT max(number) FROM issue i WHERE i.team_id = t.id), 0)";

/// B7.03: Nach einem unterbrochenen Lauf stehen die Team-Zähler auf dem Maximum der importierten
/// Nummern; `importCancel` führt sie ebenfalls nach und hebt die Sperren auf.
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn interrupted_import_leaves_counters_usable(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    let planned = e.scan(&source).await;
    e.start(planned.id, ImportStartInput::default()).await;
    source.fail_after(
        SourceEntity::Issues,
        source.page_calls(SourceEntity::Issues) + 2,
        lineal_import::SourceError::Transport("boom".into()),
    );
    let out = e
        .engine(
            &source,
            EngineConfig {
                page_size: 40,
                ..Default::default()
            },
        )
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert!(matches!(out, RunOutcome::Failed(_)), "{out:?}");
    assert!(e.count("SELECT count(*) FROM issue").await > 0, "some issues were imported");
    assert_eq!(e.count(BEHIND_SQL).await, 0, "counters follow the issues");
    assert!(e.count("SELECT max(last_issue_number) FROM team").await > 0);
    // Zähler zurücksetzen (harter Abbruch) und abbrechen: der Abbruch stellt sie wieder her.
    sqlx::query("UPDATE team SET last_issue_number = 0")
        .execute(&e.pool)
        .await
        .unwrap();
    e.app
        .import_cancel(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    assert_eq!(e.count(BEHIND_SQL).await, 0, "cancel repairs the counters");
    assert_eq!(
        e.count("SELECT count(*) FROM team WHERE import_locked_until IS NOT NULL")
            .await,
        0
    );
}

/// B7.03: Von der CLI gestartete Läufe, deren Prozess hart beendet wurde, werden vom Runner als
/// `failed` (fortsetzbar) markiert statt ewig `running` zu bleiben.
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn zombie_cli_runs_are_failed_by_the_runner(pool: PgPool) {
    let e = env(pool).await;
    let ids = [uuid::Uuid::now_v7(), uuid::Uuid::now_v7()];
    for (id, lease) in ids.iter().zip(["-2 hours", "1 minute"]) {
        sqlx::query(
            "INSERT INTO import_run (id, organization_id, source, status, lease_until) \
             VALUES ($1, $2, 'linearCsv', 'running', now() + $3::interval)",
        )
        .bind(id)
        .bind(e.org.0)
        .bind(lease)
        .execute(&e.pool)
        .await
        .unwrap();
    }
    lineal_import::runner::tick(&e.app, chrono::Utc::now())
        .await
        .unwrap();
    let status = |id: uuid::Uuid| {
        let pool = e.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT status FROM import_run WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(status(ids[0]).await, "failed");
    assert_eq!(status(ids[1]).await, "running", "a live run is left alone");
}
