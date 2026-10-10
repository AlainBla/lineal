#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP09: Upload-Flow prepare → PUT (echter Endpunkt) → finalize → signierter Download,
//! mit abgelaufener und manipulierter URL (Seed `acme`, echte Datenbank).

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use chrono::{DateTime, Duration, Utc};
use http_body_util::BodyExt;
use lineal_api::blob::LocalBlobStore;
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::attachment::PrepareUploadInput;
use lineal_db::PgStore;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

struct TestClock(Mutex<DateTime<Utc>>);

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn upload_flow_through_the_real_put_endpoint(pool: sqlx::PgPool) {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    let dir = std::env::temp_dir().join(format!("lineal-wp09-{}", Uuid::now_v7()));
    let clock = Arc::new(TestClock(Mutex::new(
        "2026-10-10T08:00:00Z".parse().unwrap(),
    )));
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.config.blob_dir = dir.clone();
    app.config.base_url = "http://lineal.test".into();
    app.clock = clock.clone();
    app.blobs = Arc::new(LocalBlobStore::from_app(&app));
    let app = Arc::new(app);
    let router = lineal_api::router::router(app.clone(), false);

    let eng = Uuid::now_v7();
    sqlx::query(
        "WITH n AS (UPDATE team SET last_issue_number = last_issue_number + 1 \
         WHERE id = $1 RETURNING last_issue_number) \
         INSERT INTO issue (id, organization_id, team_id, number, title, state_id) \
         SELECT $2, $3, $1, last_issue_number, 'Login broken', $4 FROM n",
    )
    .bind(data.eng.0)
    .bind(eng)
    .bind(data.org.0)
    .bind(seed_workflow_state_ids(1)[0].0)
    .execute(&pool)
    .await
    .unwrap();

    let bob = ctx_for(SeedUser::Bob);
    let prep = |size: i64| PrepareUploadInput {
        issue: Some(eng.to_string()),
        filename: "diagram.png".into(),
        content_type: "image/png".into(),
        size,
        title: None,
        subtitle: None,
    };
    let put = |url: &str, ct: &str, body: Vec<u8>| {
        Request::builder()
            .method(Method::PUT)
            .uri(url.trim_start_matches("http://lineal.test"))
            .header("content-type", ct)
            .body(Body::from(body))
            .unwrap()
    };

    // prepare → (ohne PUT fehlt das Objekt) → PUT → finalize → signierter Download
    let p = app.prepare_attachment_upload(&bob, prep(4)).await.unwrap();
    let err = app
        .create_attachment_from_upload(&bob, &eng.to_string(), &p.asset_url, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.details["reason"], "upload_missing");
    let res = router
        .clone()
        .oneshot(put(&p.url, "image/png", vec![1, 2, 3, 4]))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let done = app
        .create_attachment_from_upload(&bob, &eng.to_string(), &p.asset_url, None, None)
        .await
        .unwrap();
    assert_eq!(done.view.attachment.size, Some(4));
    let get = router
        .clone()
        .oneshot(
            Request::get(done.view.url.trim_start_matches("http://lineal.test"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(
        &get.into_body().collect().await.unwrap().to_bytes()[..],
        &[1, 2, 3, 4]
    );

    // abgelaufene PUT-URL (60 s) → 403; finalize findet kein Objekt
    let p2 = app.prepare_attachment_upload(&bob, prep(4)).await.unwrap();
    *clock.0.lock().unwrap() += Duration::seconds(61);
    let res = router
        .clone()
        .oneshot(put(&p2.url, "image/png", vec![1, 2, 3, 4]))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let err = app
        .create_attachment_from_upload(&bob, &eng.to_string(), &p2.asset_url, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.details["reason"], "upload_missing");

    // manipuliertes Token → 403; abweichende Größe → upload_size_mismatch
    let p3 = app.prepare_attachment_upload(&bob, prep(10)).await.unwrap();
    let forged = format!("{}x", p3.url);
    let res = router
        .clone()
        .oneshot(put(&forged, "image/png", vec![0; 10]))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = router
        .clone()
        .oneshot(put(&p3.url, "image/png", vec![0; 3]))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let err = app
        .create_attachment_from_upload(&bob, &eng.to_string(), &p3.asset_url, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.details["reason"], "upload_size_mismatch");

    // BlobStore::delete entfernt Daten und Metadaten, ist idempotent und lässt andere Objekte stehen.
    let key: String = sqlx::query_scalar(
        "SELECT storage_key FROM upload_reservation WHERE id IS NOT NULL ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(app.blobs.head(&key).await.unwrap().is_some());
    app.blobs.delete(&key).await.unwrap();
    assert!(app.blobs.head(&key).await.unwrap().is_none());
    app.blobs.delete(&key).await.unwrap();
    let data_files = std::fs::read_dir(dir.join("data")).unwrap().count();
    assert_eq!(data_files, 1, "nur das erste, finalisierte Objekt bleibt");
    let _ = std::fs::remove_dir_all(dir);
}
