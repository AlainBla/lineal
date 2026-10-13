#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Der Linear-Pfad (`LinearSource` über HTTP) einmal komplett gegen den lokalen Mock-Server.
//! Kein Test kontaktiert das echte Linear: Endpunkt und Download-Policy zeigen nur auf 127.0.0.1.

mod common;

use common::mock_linear::{Fault, MockLinear};
use common::*;
use lineal_app::usecases::import::{ImportStartInput, Secret};
use lineal_import::engine::RunOutcome;
use lineal_import::{LinearConfig, LinearSource, UrlPolicy};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

fn linear_source(endpoint: &str) -> LinearSource {
    let mut policy = UrlPolicy::for_hosts(vec!["127.0.0.1".into()]);
    policy.allow_local_for_tests = true;
    let cfg = LinearConfig {
        endpoint: endpoint.to_owned(),
        policy,
        max_retries: 5,
        backoff_base: Duration::from_millis(10),
        max_wait: Duration::from_secs(2),
        ..Default::default()
    };
    LinearSource::with_config(&Secret::new(TOKEN).unwrap(), cfg).unwrap()
}

/// Kompletter Lauf über HTTP: gleiche Tabellenstände wie mit der Fake-Quelle, Token nur als
/// `Authorization` an den Mock, keine Mutation.
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn full_import_against_the_mock_server_matches_the_fake_source(pool: PgPool) {
    let data = Arc::new(fake());
    let (mock, endpoint) = MockLinear::start(data.clone()).await;
    let http = linear_source(&endpoint);

    let via_http = env_n(pool.clone(), 1).await;
    let (_, out) = via_http
        .import_all(&http, ImportStartInput::default())
        .await;
    assert_eq!(out, RunOutcome::Completed);

    let direct = env_n(pool, 2).await;
    let (_, out) = direct
        .import_all(&*data, ImportStartInput::default())
        .await;
    assert_eq!(out, RunOutcome::Completed);

    let strip = |s: Vec<(String, i64)>| -> Vec<(String, i64)> {
        s.into_iter().filter(|(t, _)| t != "attachment").collect()
    };
    let (a, b) = (via_http.snapshot().await, direct.snapshot().await);
    assert_eq!(
        strip(a.clone()),
        strip(b.clone()),
        "HTTP path and fake source import the same rows"
    );
    // Die beiden hochgeladenen Dateien liegen auf uploads.linear.app: außerhalb der Policy
    // (nur 127.0.0.1) werden sie abgelehnt, nie abgerufen; die Fake-Quelle liefert sie.
    let attachments = |s: &[(String, i64)]| s.iter().find(|(t, _)| t == "attachment").unwrap().1;
    assert_eq!(attachments(&a) + 2, attachments(&b));
    assert!(mock.calls_for("issues") > 5, "issues were paged over HTTP");
    assert!(mock.calls_for("viewer") >= 1);
    assert!(
        mock.calls
            .lock()
            .unwrap()
            .iter()
            .all(|(_, auth)| auth.as_deref() == Some(TOKEN)),
        "the personal key is sent raw (no Bearer) to the configured endpoint only"
    );
}

/// Rate-Limit und Serverfehler mitten im Lauf werden abgewartet bzw. wiederholt.
#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn import_survives_rate_limits_and_server_errors(pool: PgPool) {
    let data = Arc::new(fake());
    let (mock, endpoint) = MockLinear::start(data).await;
    let http = linear_source(&endpoint);
    let e = env(pool).await;
    let planned = e.scan(&http).await;
    e.start(planned.id, ImportStartInput::default()).await;
    {
        let mut f = mock.faults.lock().unwrap();
        f.push_back(Fault::RateLimited(150));
        f.push_back(Fault::ServerError);
        f.push_back(Fault::RateLimited(150));
    }
    let out = e
        .engine(&http, Default::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert!(mock.faults.lock().unwrap().is_empty(), "all faults were hit");
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 200);
}
