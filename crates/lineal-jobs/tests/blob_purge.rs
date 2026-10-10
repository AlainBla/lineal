#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP17 P1.4: Purge-Jobs löschen die zugehörigen Blobs (Issues, Teams, Upload-Reservierungen).

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::{BlobStore, Store};
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::{App, AppResult};
use lineal_db::PgStore;
use lineal_jobs::PeriodicJob;
use lineal_jobs::periodic::purge::PurgeJob;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

/// Merkt sich gelöschte Schlüssel.
#[derive(Default)]
struct RecordingBlobs(Mutex<Vec<String>>);

impl RecordingBlobs {
    fn deleted(&self) -> Vec<String> {
        let mut v = self.0.lock().unwrap().clone();
        v.sort();
        v
    }
}

#[async_trait]
impl BlobStore for RecordingBlobs {
    async fn delete(&self, key: &str) -> AppResult<()> {
        self.0.lock().unwrap().push(key.to_owned());
        Ok(())
    }
    async fn signed_put_url(
        &self,
        key: &str,
        _ct: &str,
        _ttl: std::time::Duration,
    ) -> AppResult<String> {
        Ok(format!("http://fake.invalid/uploads/{key}"))
    }
    async fn signed_get_url(&self, key: &str, _ttl: std::time::Duration) -> AppResult<String> {
        Ok(format!("http://fake.invalid/files/{key}"))
    }
}

struct Fx {
    app: Arc<App>,
    blobs: Arc<RecordingBlobs>,
    data: SeedData,
    pool: PgPool,
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap()
}

async fn setup(pool: PgPool) -> Fx {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let blobs = Arc::new(RecordingBlobs::default());
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.blobs = blobs.clone();
    Fx {
        app: Arc::new(app),
        blobs,
        data,
        pool,
    }
}

async fn issue(fx: &Fx, who: SeedUser, team: &str, title: &str) -> (uuid::Uuid, String) {
    let created = fx
        .app
        .create_issue(
            &ctx_for(who),
            CreateIssueInput {
                team: team.into(),
                title: Some(title.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    (created.issue.id.0, created.issue.identifier)
}

async fn attach(fx: &Fx, issue: uuid::Uuid, key: &str) {
    sqlx::query(
        "INSERT INTO attachment (organization_id, issue_id, url, title, storage_key) \
         VALUES ($1, $2, $3, 'Datei', $4)",
    )
    .bind(fx.data.org.0)
    .bind(issue)
    .bind(format!("https://lineal.example/assets/{key}"))
    .bind(key)
    .execute(&fx.pool)
    .await
    .unwrap();
}

async fn reserve(
    fx: &Fx,
    issue: Option<uuid::Uuid>,
    key: &str,
    expires: DateTime<Utc>,
    done: bool,
) {
    sqlx::query(
        "INSERT INTO upload_reservation (organization_id, issue_id, storage_key, filename, \
         content_type, size, expires_at, finalized_at) VALUES ($1, $2, $3, 'a.png', 'image/png', 10, $4, $5)",
    )
    .bind(fx.data.org.0)
    .bind(issue)
    .bind(key)
    .bind(expires)
    .bind(done.then_some(expires))
    .execute(&fx.pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issue_purge_deletes_blobs_but_keeps_shared_keys(pool: PgPool) {
    let fx = setup(pool).await;
    let (gone, gone_ident) = issue(&fx, SeedUser::Alice, "ENG", "Weg damit").await;
    let (kept, _) = issue(&fx, SeedUser::Alice, "ENG", "Bleibt").await;
    attach(&fx, gone, "acme/own.png").await;
    attach(&fx, gone, "acme/shared.png").await;
    attach(&fx, kept, "acme/shared.png").await; // zweiter Verweis auf dasselbe Objekt
    attach(&fx, kept, "acme/kept.png").await;
    reserve(&fx, Some(gone), "acme/reserved.png", t0(), true).await;

    fx.app
        .delete_issue(&ctx_for(SeedUser::Alice), &gone_ident, false)
        .await
        .unwrap();
    let trashed_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT trashed_at FROM issue WHERE id = $1")
            .bind(gone)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    // Innerhalb der Frist passiert nichts.
    PurgeJob
        .run(&fx.app, trashed_at + Duration::days(10))
        .await
        .unwrap();
    assert!(fx.blobs.deleted().is_empty());

    PurgeJob
        .run(&fx.app, trashed_at + Duration::days(31))
        .await
        .unwrap();
    assert_eq!(
        fx.blobs.deleted(),
        ["acme/own.png", "acme/reserved.png"],
        "geteilte und fremde Objekte bleiben"
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn expired_unfinalized_reservations_lose_their_blob(pool: PgPool) {
    let fx = setup(pool).await;
    let now = t0();
    reserve(
        &fx,
        None,
        "acme/old-open.png",
        now - Duration::days(3),
        false,
    )
    .await;
    reserve(
        &fx,
        None,
        "acme/fresh-open.png",
        now - Duration::hours(2),
        false,
    )
    .await;
    reserve(
        &fx,
        None,
        "acme/old-done.png",
        now - Duration::days(3),
        true,
    )
    .await;

    assert_eq!(fx.app.run_upload_purge(now).await.unwrap(), 1);
    assert_eq!(fx.blobs.deleted(), ["acme/old-open.png"]);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM upload_reservation")
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert_eq!(left, 2);
    // Idempotent.
    assert_eq!(fx.app.run_upload_purge(now).await.unwrap(), 0);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn team_purge_deletes_blobs_of_its_issues(pool: PgPool) {
    let fx = setup(pool).await;
    let (des_issue, _) = issue(&fx, SeedUser::Bob, "DES", "Design-Aufgabe").await;
    let (eng_issue, _) = issue(&fx, SeedUser::Alice, "ENG", "Engineering").await;
    attach(&fx, des_issue, "acme/des.png").await;
    attach(&fx, eng_issue, "acme/eng.png").await;
    sqlx::query("UPDATE team SET trashed = true, trashed_at = $2 WHERE id = $1")
        .bind(fx.data.des.0)
        .bind(t0() - Duration::days(40))
        .execute(&fx.pool)
        .await
        .unwrap();
    let stats = fx.app.run_team_purge(t0()).await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(stats.processed, 1);
    assert_eq!(fx.blobs.deleted(), ["acme/des.png"]);
}
