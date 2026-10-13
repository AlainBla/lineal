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

/// Quelle, die beim n-ten Issue-Seitenabruf eine Aktion ausführt und danach lange wartet
/// (wie eine Rate-Limit-Wartezeit des Linear-Adapters).
struct SlowOnPage<'a> {
    inner: &'a lineal_import::FakeSource,
    env: &'a Env,
    run: uuid::Uuid,
    at_call: usize,
    pause: bool,
    wait: std::time::Duration,
    calls: std::sync::atomic::AtomicUsize,
    /// Lease am Ende der Wartezeit.
    lease_after_wait: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
}

#[async_trait::async_trait]
impl lineal_import::ImportSource for SlowOnPage<'_> {
    fn kind(&self) -> &'static str {
        "linear"
    }
    async fn workspace(
        &self,
    ) -> lineal_import::port::SourceResult<lineal_import::port::SourceWorkspace> {
        self.inner.workspace().await
    }
    async fn page(
        &self,
        q: &lineal_import::port::PageQuery,
    ) -> lineal_import::port::SourceResult<lineal_import::port::SourcePage> {
        if q.entity == SourceEntity::Issues {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if n == self.at_call {
                if self.pause {
                    self.env
                        .app
                        .import_pause(&self.env.alice_ctx(), self.run)
                        .await
                        .unwrap();
                } else {
                    // Die Uhr läuft weiter; ohne Heartbeat wäre die Lease längst abgelaufen.
                    self.env.clock.set(t0() + chrono::Duration::minutes(10));
                }
                tokio::time::sleep(self.wait).await;
                let lease: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
                    "SELECT lease_until FROM import_run WHERE id = $1",
                )
                .bind(self.run)
                .fetch_one(&self.env.pool)
                .await
                .unwrap();
                *self.lease_after_wait.lock().unwrap() = lease;
            }
        }
        self.inner.page(q).await
    }
    async fn download(
        &self,
        url: &str,
    ) -> lineal_import::port::SourceResult<lineal_import::port::Downloaded> {
        self.inner.download(url).await
    }
}

fn fast_control() -> EngineConfig {
    EngineConfig {
        page_size: 40,
        heartbeat: std::time::Duration::from_millis(50),
        control_poll: std::time::Duration::from_millis(50),
        ..Default::default()
    }
}

/// B7.04: Während einer langen Wartezeit der Quelle wird die Lease verlängert.
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn lease_is_extended_while_the_source_waits(pool: PgPool) {
    let e = env(pool).await;
    let fake_src = fake();
    let planned = e.scan(&fake_src).await;
    e.start(planned.id, ImportStartInput::default()).await;
    let src = SlowOnPage {
        inner: &fake_src,
        env: &e,
        run: planned.id,
        at_call: 2,
        pause: false,
        wait: std::time::Duration::from_millis(600),
        calls: Default::default(),
        lease_after_wait: Default::default(),
    };
    let out = e
        .engine(&src, fast_control())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    let lease = *src.lease_after_wait.lock().unwrap();
    // Ohne Heartbeat stünde die Lease bei t0 + 2 min; mit ihm bei (Uhr = t0 + 10 min) + 2 min.
    assert!(
        lease.is_some_and(|l| l > t0() + chrono::Duration::minutes(10)),
        "lease during the wait: {lease:?}"
    );
}

/// B7.04: Pause wirkt zeitnah, auch wenn die Quelle gerade lange wartet.
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn pause_takes_effect_during_a_long_source_wait(pool: PgPool) {
    let e = env(pool).await;
    let fake_src = fake();
    let planned = e.scan(&fake_src).await;
    e.start(planned.id, ImportStartInput::default()).await;
    let src = SlowOnPage {
        inner: &fake_src,
        env: &e,
        run: planned.id,
        at_call: 2,
        pause: true,
        wait: std::time::Duration::from_secs(120),
        calls: Default::default(),
        lease_after_wait: Default::default(),
    };
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        e.engine(&src, fast_control()).run(e.org, planned.id),
    )
    .await
    .expect("the pause must not wait for the source")
    .unwrap();
    assert_eq!(out, RunOutcome::Paused);
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

/// B7.08: Ein zweiter echter Lauf ins selbe Team wird abgelehnt, solange der erste läuft; ein Lauf
/// mit Objektfehlern endet nicht kommentarlos als `completed`.
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn second_run_for_the_same_teams_is_rejected_and_errors_are_flagged(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    source.patch(SourceEntity::Issues, "i-ENG-30", &json!({ "team": null }));
    let first = e.scan(&source).await;
    let second = e.scan(&source).await;
    e.start(first.id, ImportStartInput::default()).await;
    let err = e
        .app
        .import_start(
            &e.alice_ctx(),
            ImportStartInput {
                run_id: second.id,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, lineal_app::error::ErrorKind::Conflict, "{err:?}");
    // Der erste Lauf läuft zu Ende; ein Issue ohne Team ist ein Objektfehler.
    let out = e
        .engine(&source, Default::default())
        .run(e.org, first.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    let flagged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM import_issue WHERE import_run_id = $1 \
         AND message LIKE 'the run completed, but % could not be imported%'",
    )
    .bind(first.id)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(flagged, 1);
    // Danach ist das Team frei.
    e.app
        .import_start(
            &e.alice_ctx(),
            ImportStartInput {
                run_id: second.id,
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

/// B7.07: Der CSV-Import meldet jede stille Datenänderung (Duplikat, Kürzung, ungültige Werte).
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn csv_import_reports_every_data_change(pool: PgPool) {
    let e = env(pool).await;
    let long = "t".repeat(300);
    let csv = format!(
        "ID,Team,Title,Status,Priority,Created\n\
ENG-1,Engineering,First,Todo,High,2025-01-06T09:00:00.000Z\n\
ENG-1,Engineering,Duplicate,Todo,High,2025-01-06T09:00:00.000Z\n\
ENG-2,Engineering,{long},Odd Status,Extreme,yesterday\n"
    );
    let source = lineal_import::CsvSource::from_csv(csv.as_bytes(), Some("ENG")).unwrap();
    let ctx = e.alice_ctx();
    let run = e
        .app
        .import_register_scanned(&ctx, "linearCsv", json!({ "teams": [{ "id": "csv-team-ENG", "key": "ENG", "name": "Engineering" }] }))
        .await
        .unwrap();
    e.app
        .import_start(
            &ctx,
            ImportStartInput {
                run_id: run.id,
                teams: vec!["ENG".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let out = e
        .engine(&source, Default::default())
        .run(e.org, run.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 2);
    let messages: Vec<String> = sqlx::query_scalar(
        "SELECT message FROM import_issue WHERE import_run_id = $1 AND severity = 'warning'",
    )
    .bind(run.id)
    .fetch_all(&e.pool)
    .await
    .unwrap();
    let all = messages.join("\n");
    for needle in [
        "duplicate ID ENG-1",
        "longer than 255",
        "status `Odd Status`",
        "unknown priority `Extreme`",
        "created date `yesterday`",
    ] {
        assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
    }
}
