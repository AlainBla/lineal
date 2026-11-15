#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-70 T6: Login-Split. Ein aktives TOTP erzwingt den zweiten Schritt
//! (`TotpRequired`); ein Owner ohne TOTP läuft bei aktivierter Pflicht in die
//! Schonfrist (3 Logins erlaubt, der 4. verlangt Enrollment). Ohne Pflicht und
//! für Nicht-Owner ändert sich nichts.

use chrono::{DateTime, Utc};
use lineal_app::App;
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::auth::{LoginInput, LoginOutcome};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

async fn setup(pool: PgPool, enforce: bool) -> (App, SeedData) {
    let d = SeedData::acme();
    let store = PgStore::new(pool);
    let mut tx = store.begin(d.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = Arc::new(SystemClock);
    app.config.totp_owner_enforcement = enforce;
    for user in [SeedUser::Alice, SeedUser::Bob] {
        app.set_user_password(&ctx_for(user), "me", "correct-horse")
            .await
            .unwrap();
    }
    (app, d)
}

fn login_input(email: &str) -> LoginInput {
    LoginInput {
        email: email.into(),
        password: "correct-horse".into(),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn active_totp_forces_second_step(pool: PgPool) {
    let (app, d) = setup(pool.clone(), false).await;
    // Aktives TOTP für alice (Owner) direkt setzen.
    sqlx::query(
        "INSERT INTO user_totp (user_id, organization_id, secret_enc, dek_wrapped, key_backend, key_version, confirmed_at) \
         VALUES ($1,$2,$3,$4,'app',1, now())",
    )
    .bind(d.alice.0)
    .bind(d.org.0)
    .bind(vec![1u8; 10])
    .bind(vec![2u8; 10])
    .execute(&pool)
    .await
    .unwrap();

    match app
        .login_outcome(login_input("alice@acme.example"))
        .await
        .unwrap()
    {
        LoginOutcome::TotpRequired { user_id, .. } => assert_eq!(user_id, d.alice),
        other => panic!("expected TotpRequired, got {other:?}"),
    }
    // Nutzer ohne TOTP bekommt weiter eine Session.
    assert!(matches!(
        app.login_outcome(login_input("bob@acme.example"))
            .await
            .unwrap(),
        LoginOutcome::Session(_)
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn owner_grace_allows_three_logins_then_blocks(pool: PgPool) {
    let (app, _d) = setup(pool, true).await;
    for n in 1..=3 {
        assert!(
            matches!(
                app.login_outcome(login_input("alice@acme.example"))
                    .await
                    .unwrap(),
                LoginOutcome::Session(_)
            ),
            "Login {n} muss innerhalb der Schonfrist durchgehen"
        );
    }
    match app
        .login_outcome(login_input("alice@acme.example"))
        .await
        .unwrap()
    {
        LoginOutcome::EnrollRequired { .. } => {}
        other => panic!("der 4. Login muss Enrollment erzwingen, war {other:?}"),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn enforcement_off_never_blocks_owner(pool: PgPool) {
    let (app, _d) = setup(pool, false).await;
    for _ in 0..5 {
        assert!(matches!(
            app.login_outcome(login_input("alice@acme.example"))
                .await
                .unwrap(),
            LoginOutcome::Session(_)
        ));
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn enforcement_job_revokes_expired_owner_sessions(pool: PgPool) {
    let (app, _d) = setup(pool, true).await;
    // 3 Logins innerhalb der Schonfrist; einen Token behalten.
    let mut token = None;
    for _ in 0..3 {
        if let LoginOutcome::Session(l) = app
            .login_outcome(login_input("alice@acme.example"))
            .await
            .unwrap()
        {
            token = Some(l.token);
        }
    }
    let token = token.unwrap();
    assert!(app.principal_for_token(&token).await.is_ok());

    // 4. Login: Schonfrist erschöpft.
    assert!(matches!(
        app.login_outcome(login_input("alice@acme.example"))
            .await
            .unwrap(),
        LoginOutcome::EnrollRequired { .. }
    ));

    // Der Job widerruft Alices Sessions.
    let revoked = app.enforce_totp_grace(app.clock.now()).await.unwrap();
    assert_eq!(revoked, 1);
    assert!(
        app.principal_for_token(&token).await.is_err(),
        "Session nach Enforcement ungueltig"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn member_is_never_enrolled(pool: PgPool) {
    let (app, _d) = setup(pool, true).await;
    for _ in 0..5 {
        assert!(matches!(
            app.login_outcome(login_input("bob@acme.example"))
                .await
                .unwrap(),
            LoginOutcome::Session(_)
        ));
    }
}
