#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Paket 7: Importintegrität (Zyklen, Zähler, Lease, Parallelität), Fake-Quelle.

mod common;

use common::*;
use lineal_app::usecases::import::ImportStartInput;
use lineal_import::SourceEntity;
use lineal_import::engine::RunOutcome;
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
