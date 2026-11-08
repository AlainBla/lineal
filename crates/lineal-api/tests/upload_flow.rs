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

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

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
    let p = app.prepare_attachment_upload(&bob, prep(12)).await.unwrap();
    let err = app
        .create_attachment_from_upload(&bob, &eng.to_string(), &p.asset_url, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.details["reason"], "upload_missing");
    let res = router
        .clone()
        .oneshot(put(&p.url, "image/png", PNG.to_vec()))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let done = app
        .create_attachment_from_upload(&bob, &eng.to_string(), &p.asset_url, None, None)
        .await
        .unwrap();
    assert_eq!(done.view.attachment.size, Some(12));
    // S-13/S-14: Nach dem Finalisieren bleibt keine finalisierte Reservierung stehen (sie blockierte
    // sonst das Aufräumen und ließ resolve_assets als Fallback signieren).
    let finalized: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM upload_reservation WHERE finalized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        finalized, 0,
        "finalisierte Reservierung wird beim Anlegen gelöscht"
    );
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
        PNG
    );

    // abgelaufene PUT-URL (60 s) → 403; finalize findet kein Objekt
    let p2 = app.prepare_attachment_upload(&bob, prep(12)).await.unwrap();
    *clock.0.lock().unwrap() += Duration::seconds(61);
    let res = router
        .clone()
        .oneshot(put(&p2.url, "image/png", PNG.to_vec()))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let err = app
        .create_attachment_from_upload(&bob, &eng.to_string(), &p2.asset_url, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.details["reason"], "upload_missing");

    // manipuliertes Token → 403; abweichende Größe → upload_size_mismatch
    let p3 = app.prepare_attachment_upload(&bob, prep(20)).await.unwrap();
    let forged = format!("{}x", p3.url);
    let res = router
        .clone()
        .oneshot(put(&forged, "image/png", PNG.to_vec()))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let res = router
        .clone()
        .oneshot(put(&p3.url, "image/png", PNG.to_vec()))
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

/// LIN-25: Datei am Meilenstein: Upload ohne Issue, signierter Download, Rechte folgen dem Projekt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn milestone_file_flow_follows_project_access(pool: sqlx::PgPool) {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let dir = std::env::temp_dir().join(format!("lineal-lin25-{}", Uuid::now_v7()));
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.config.blob_dir = dir.clone();
    app.config.base_url = "http://lineal.test".into();
    app.blobs = Arc::new(LocalBlobStore::from_app(&app));
    let app = Arc::new(app);
    let router = lineal_api::router::router(app.clone(), false);
    let bob = ctx_for(SeedUser::Bob);
    let alice = ctx_for(SeedUser::Alice);

    // Projekt nur im Team DES, das Alice nicht sieht.
    let (project, milestone): (String, String) = {
        let p = app
            .save_project(
                &bob,
                lineal_app::usecases::project::SaveProjectInput {
                    name: Some("Geheim".into()),
                    add_teams: vec!["DES".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let (_, m) = app
            .save_milestone(
                &bob,
                lineal_app::usecases::project::milestone::SaveMilestoneInput {
                    project: p.project.id.to_string(),
                    name: Some("Abnahme".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        (p.project.id.to_string(), m.milestone.id.to_string())
    };

    let upload = |user| {
        let app = app.clone();
        let router = router.clone();
        async move {
            let p = app
                .prepare_attachment_upload(
                    &user,
                    PrepareUploadInput {
                        issue: None,
                        filename: "plan.png".into(),
                        content_type: "image/png".into(),
                        size: PNG.len() as i64,
                        title: None,
                        subtitle: None,
                    },
                )
                .await
                .unwrap();
            let res = router
                .oneshot(
                    Request::builder()
                        .method(Method::PUT)
                        .uri(p.url.trim_start_matches("http://lineal.test"))
                        .header("content-type", "image/png")
                        .body(Body::from(PNG.to_vec()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            p.asset_url
        }
    };

    let asset = upload(bob.clone()).await;
    let id = app
        .add_resource_file(&bob, &project, Some(&milestone), &asset, None)
        .await
        .unwrap();
    let list = app
        .list_resources(&bob, &project, Some(&milestone))
        .await
        .unwrap();
    assert_eq!(list.files[0].file.title, "plan.png");
    assert_eq!(list.files[0].file.size, Some(PNG.len() as i64));
    let get = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(
                    list.files[0]
                        .download_url
                        .trim_start_matches("http://lineal.test"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);

    // Ein zweites Mal lässt sich derselbe Upload nicht anhängen. Die Reservierung wird beim
    // Finalisieren gelöscht (S-13/S-14), daher jetzt `upload_unknown` statt `upload_finalized`.
    let again = app
        .add_resource_file(&bob, &project, Some(&milestone), &asset, None)
        .await
        .unwrap_err();
    assert_eq!(again.details["reason"], "upload_unknown");

    // Wer das Projekt nicht sieht, bekommt weder Liste noch Download-URL.
    assert!(app.list_resources(&alice, &project, None).await.is_err());
    let urls = [asset.clone()];
    assert!(app.resolve_assets(&alice, &urls).await.unwrap().is_empty());
    assert_eq!(app.resolve_assets(&bob, &urls).await.unwrap().len(), 1);

    // Fremder Upload: nur der Ersteller darf ihn anhängen.
    let foreign = upload(alice.clone()).await;
    let err = app
        .add_resource_file(&bob, &project, Some(&milestone), &foreign, None)
        .await
        .unwrap_err();
    assert_eq!(err.details["reason"], "upload_not_owned");

    // Löschen entfernt Zeile und Objekt.
    app.remove_resource_file(&bob, &project, &id.to_string())
        .await
        .unwrap();
    let list = app
        .list_resources(&bob, &project, Some(&milestone))
        .await
        .unwrap();
    assert!(list.files.is_empty());
    assert!(app.resolve_assets(&bob, &urls).await.unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}
