#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Kalender-Feed über den echten Endpunkt (Seed `acme`, echte Datenbank).

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use lineal_app::App;
use lineal_app::ports::Store;
use lineal_app::usecases::project::milestone::SaveMilestoneInput;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use std::sync::Arc;
use tower::ServiceExt;

const BASE: &str = "http://lineal.test";

async fn setup(pool: sqlx::PgPool) -> (Arc<App>, Router) {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.config.base_url = BASE.into();
    let app = Arc::new(app);
    let router = lineal_api::router::router(app.clone(), false);
    (app, router)
}

async fn get(router: &Router, url: &str) -> (StatusCode, String, Option<String>) {
    let path = url.strip_prefix(BASE).unwrap_or(url);
    let res = router
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let ct = res
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_owned());
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap(), ct)
}

async fn milestone(app: &App, name: &str) {
    app.save_milestone(
        &ctx_for(SeedUser::Alice),
        SaveMilestoneInput {
            project: "P-ENG-1".into(),
            name: Some(name.into()),
            description: Some("Vertraulicher Text".into()),
            target_date: Some(Some("2026-12-24".parse().unwrap())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn feed_lists_dated_milestones_without_their_text(pool: sqlx::PgPool) {
    let (app, router) = setup(pool).await;
    milestone(&app, "Weihnachten").await;
    let url = app
        .rotate_calendar_feed(&ctx_for(SeedUser::Alice))
        .await
        .unwrap();
    assert!(url.starts_with("http://lineal.test/calendar/lin_cal_"));
    assert!(url.ends_with(".ics"));

    let (status, ics, ct) = get(&router, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ct.as_deref(), Some("text/calendar; charset=utf-8"));
    assert!(ics.contains("DTSTART;VALUE=DATE:20261224\r\n"));
    let unfolded = ics.replace("\r\n ", "");
    assert!(unfolded.contains("SUMMARY:Weihnachten ("));
    assert!(unfolded.contains("/project/P-ENG-1/milestone/"));
    assert!(!unfolded.contains("Vertraulicher"));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn feed_follows_what_the_person_may_see_now(pool: sqlx::PgPool) {
    let (app, router) = setup(pool.clone()).await;
    milestone(&app, "Geheimprojekt").await;
    let url = app
        .rotate_calendar_feed(&ctx_for(SeedUser::Bob))
        .await
        .unwrap();
    assert!(
        get(&router, &url)
            .await
            .1
            .replace("\r\n ", "")
            .contains("Geheimprojekt")
    );

    // Verliert Bob den Zugang zum privaten Team, verschwindet der Termin aus demselben Link.
    sqlx::query("UPDATE team SET visibility = 'private' WHERE key = 'ENG'")
        .execute(&pool)
        .await
        .unwrap();
    let (bob, _) = SeedData::acme().user(SeedUser::Bob);
    sqlx::query("DELETE FROM team_membership WHERE user_id = $1")
        .bind(bob.0)
        .execute(&pool)
        .await
        .unwrap();
    let (status, ics, _) = get(&router, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!ics.replace("\r\n ", "").contains("Geheimprojekt"));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn old_revoked_and_unknown_links_are_not_found(pool: sqlx::PgPool) {
    let (app, router) = setup(pool.clone()).await;
    let ctx = ctx_for(SeedUser::Alice);
    assert!(!app.calendar_feed_enabled(&ctx).await.unwrap());
    let first = app.rotate_calendar_feed(&ctx).await.unwrap();
    let second = app.rotate_calendar_feed(&ctx).await.unwrap();
    assert!(app.calendar_feed_enabled(&ctx).await.unwrap());
    assert_eq!(get(&router, &first).await.0, StatusCode::NOT_FOUND);
    assert_eq!(get(&router, &second).await.0, StatusCode::OK);
    let no_suffix = second.strip_suffix(".ics").unwrap();
    assert_eq!(get(&router, no_suffix).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        get(&router, "/calendar/lin_cal_unknown.ics").await.0,
        StatusCode::NOT_FOUND
    );

    app.revoke_calendar_feed(&ctx).await.unwrap();
    assert!(!app.calendar_feed_enabled(&ctx).await.unwrap());
    assert_eq!(get(&router, &second).await.0, StatusCode::NOT_FOUND);

    let third = app.rotate_calendar_feed(&ctx).await.unwrap();
    let (id, _) = SeedData::acme().user(SeedUser::Alice);
    sqlx::query("UPDATE \"user\" SET active = false WHERE id = $1")
        .bind(id.0)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(get(&router, &third).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn suspension_removes_the_feed_so_unlock_does_not_revive_it(pool: sqlx::PgPool) {
    // L-4: Eine Sperrung soll den Kalender-Feed endgültig entwerten – eine spätere Entsperrung
    // darf den alten Link nicht wiederbeleben (docs/02 Z. 711 „eine Entsperrung belebt nichts wieder").
    let (app, router) = setup(pool.clone()).await;
    let url = app
        .rotate_calendar_feed(&ctx_for(SeedUser::Bob))
        .await
        .unwrap();
    assert_eq!(get(&router, &url).await.0, StatusCode::OK);

    // Owner sperrt Bob, dann entsperrt ihn wieder.
    let owner = ctx_for(SeedUser::Alice);
    app.set_user_active(&owner, "bob@acme.example", false)
        .await
        .unwrap();
    app.set_user_active(&owner, "bob@acme.example", true)
        .await
        .unwrap();

    // Der alte Link bleibt tot und die Feed-Zeile ist gelöscht (nicht nur über `active` verdeckt).
    assert_eq!(get(&router, &url).await.0, StatusCode::NOT_FOUND);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM calendar_feed")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    // Nach der Entsperrung ist der Feed wieder normal aktivierbar.
    assert!(!app.calendar_feed_enabled(&ctx_for(SeedUser::Bob)).await.unwrap());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn links_are_stored_hashed_and_only_managed_from_a_session(pool: sqlx::PgPool) {
    let (app, _) = setup(pool.clone()).await;
    let url = app
        .rotate_calendar_feed(&ctx_for(SeedUser::Alice))
        .await
        .unwrap();
    let token = url
        .rsplit('/')
        .next()
        .unwrap()
        .strip_suffix(".ics")
        .unwrap();
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM calendar_feed")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!stored.contains(token));

    let mut key = ctx_for(SeedUser::Alice);
    key.via_session = false;
    assert!(app.rotate_calendar_feed(&key).await.is_err());
    assert!(app.revoke_calendar_feed(&key).await.is_err());
}
