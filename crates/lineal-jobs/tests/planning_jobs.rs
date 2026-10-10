#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP07: Jobs `cycles` und `update_reminders` gegen echte Datenbank (Seed `acme`).

use chrono::{DateTime, TimeZone, Utc};
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_jobs::PeriodicJob;
use lineal_jobs::periodic::cycles::CyclesJob;
use lineal_jobs::periodic::update_reminders::UpdateRemindersJob;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, mi, 0).single().unwrap()
}

async fn setup(pool: &PgPool) -> Arc<lineal_app::App> {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    test_app(Arc::new(store))
}

async fn prompts(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notification WHERE type = 'projectUpdatePrompt'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn reminders_are_sent_once_per_slot_and_stage(pool: PgPool) {
    let app = setup(&pool).await;
    // Seed-Projekt `started`, Lead Alice (UTC); Workspace-Default: wöchentlich Freitag 14:00.
    let job = UpdateRemindersJob;
    job.run(&app, utc(2026, 10, 9, 14, 5)).await.unwrap();
    assert_eq!(prompts(&pool).await, 1);
    job.run(&app, utc(2026, 10, 9, 15, 0)).await.unwrap();
    assert_eq!(prompts(&pool).await, 1, "Stufe 0 nur einmal");
    // Nachfass nach 1 Arbeitstag (Montag 14:00).
    job.run(&app, utc(2026, 10, 12, 14, 5)).await.unwrap();
    assert_eq!(prompts(&pool).await, 2);
    job.run(&app, utc(2026, 10, 12, 16, 0)).await.unwrap();
    assert_eq!(prompts(&pool).await, 2);
    // Ein Update seit dem Termin beendet die Erinnerungen.
    let alice = ctx_for(SeedUser::Alice);
    app.post_project_update(&alice, "P-ENG-1", "Stand.", None, false)
        .await
        .unwrap();
    // Die Test-Uhr steht auf 2026-10-10; das Update ist neuer als der Termin vom 09.10. (der Slot 02.10. ist alt).
    job.run(&app, utc(2026, 10, 14, 14, 5)).await.unwrap();
    assert_eq!(prompts(&pool).await, 2);
    // Der Initiative-Modus (Workspace-Default: keine Frequenz) erzeugt nichts.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notification WHERE type = 'initiativeUpdatePrompt'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn paused_and_never_modes_suppress_reminders(pool: PgPool) {
    let app = setup(&pool).await;
    sqlx::query("UPDATE project SET update_reminder_mode = 'never'")
        .execute(&pool)
        .await
        .unwrap();
    UpdateRemindersJob.run(&app, utc(2026, 10, 9, 14, 5)).await.unwrap();
    assert_eq!(prompts(&pool).await, 0);
    sqlx::query("UPDATE project SET update_reminder_mode = 'followWorkspace', update_reminders_paused_until_at = $1")
        .bind(utc(2026, 11, 1, 0, 0))
        .execute(&pool)
        .await
        .unwrap();
    UpdateRemindersJob.run(&app, utc(2026, 10, 9, 14, 5)).await.unwrap();
    assert_eq!(prompts(&pool).await, 0);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn cycles_job_generates_and_rolls_over(pool: PgPool) {
    let app = setup(&pool).await;
    // Uhr 2026-10-10: Seed-ENG hat #2 aktiv, #3 und #4 künftig → nichts zu tun, Snapshot für heute.
    CyclesJob.run(&app, utc(2026, 10, 10, 7, 15)).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cycle WHERE team_id = $1")
        .bind(SeedData::acme().eng.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 4);
    // Später (2026-11-02 00:00 Berlin = 2026-11-01T23:00Z): #3 läuft, #2 ist abgeschlossen, ein fünfter Cycle entsteht.
    CyclesJob.run(&app, utc(2026, 11, 1, 23, 5)).await.unwrap();
    let (completed2, started3): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT (SELECT completed_at FROM cycle WHERE number = 2 AND team_id = $1), \
                (SELECT started_processed_at FROM cycle WHERE number = 3 AND team_id = $1)",
    )
    .bind(SeedData::acme().eng.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(completed2.is_some() && started3.is_some());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM cycle WHERE team_id = $1 AND starts_at > $2")
        .bind(SeedData::acme().eng.0)
        .bind(utc(2026, 11, 1, 23, 5))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 2, "upcomingCycleCount = 2 wird wieder aufgefüllt");
}
