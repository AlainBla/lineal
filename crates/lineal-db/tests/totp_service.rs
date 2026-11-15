#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-70 T8: Enrollment, Zweitschritt (Code + Recovery), Replay-Schutz,
//! Abschalten und Reset – gegen die echte Datenbank.

use chrono::{DateTime, Utc};
use lineal_app::App;
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::totp::core;
use lineal_app::usecases::totp::service::TotpSecondFactor;
use lineal_db::PgStore;
use lineal_domain::ids::UserId;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

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
    (app, d)
}

fn code(secret: &[u8]) -> String {
    core::code_now(secret, Utc::now().timestamp())
}

/// Richtet TOTP für bob ein und gibt (Secret, Recovery-Codes) zurück.
async fn enroll(app: &App) -> (Vec<u8>, Vec<String>) {
    let ctx = ctx_for(SeedUser::Bob);
    let enrollment = app.totp_begin_enrollment(&ctx).await.unwrap();
    let secret = core::base32_decode(&enrollment.secret_base32).unwrap();
    let recovery = app
        .totp_confirm_enrollment(&ctx, &code(&secret))
        .await
        .unwrap();
    (secret, recovery)
}

#[sqlx::test(migrations = "./migrations")]
async fn enrollment_activates_and_yields_recovery_codes(pool: PgPool) {
    let (app, _d) = setup(pool, false).await;
    let ctx = ctx_for(SeedUser::Bob);
    assert!(!app.totp_status(&ctx).await.unwrap().confirmed);

    let (_secret, recovery) = enroll(&app).await;
    assert_eq!(recovery.len(), 10);

    let status = app.totp_status(&ctx).await.unwrap();
    assert!(status.confirmed);
    assert_eq!(status.recovery_remaining, 10);
}

#[sqlx::test(migrations = "./migrations")]
async fn wrong_code_does_not_activate(pool: PgPool) {
    let (app, _d) = setup(pool, false).await;
    let ctx = ctx_for(SeedUser::Bob);
    app.totp_begin_enrollment(&ctx).await.unwrap();
    let err = app.totp_confirm_enrollment(&ctx, "000000").await;
    assert!(err.is_err());
    assert!(!app.totp_status(&ctx).await.unwrap().confirmed);
}

#[sqlx::test(migrations = "./migrations")]
async fn second_factor_logs_in_and_code_cannot_replay(pool: PgPool) {
    let (app, d) = setup(pool, false).await;
    let (secret, _recovery) = enroll(&app).await;

    let c = code(&secret);
    let login = app
        .totp_complete_login(
            d.bob,
            d.org,
            &TotpSecondFactor::Code(c.clone()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(app.principal_for_token(&login.token).await.is_ok());

    // Derselbe Code darf nicht erneut gelten (Replay-Schutz).
    let replay = app
        .totp_complete_login(d.bob, d.org, &TotpSecondFactor::Code(c), None, None, None)
        .await;
    assert!(replay.is_err(), "benutzter Code darf nicht erneut gelten");
}

#[sqlx::test(migrations = "./migrations")]
async fn recovery_code_logs_in_once(pool: PgPool) {
    let (app, d) = setup(pool, false).await;
    let (_secret, recovery) = enroll(&app).await;
    let one = recovery[0].clone();

    let login = app
        .totp_complete_login(
            d.bob,
            d.org,
            &TotpSecondFactor::Recovery(one.clone()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(app.principal_for_token(&login.token).await.is_ok());

    // Verbraucht: zweiter Versuch mit demselben Recovery-Code scheitert.
    let again = app
        .totp_complete_login(
            d.bob,
            d.org,
            &TotpSecondFactor::Recovery(one),
            None,
            None,
            None,
        )
        .await;
    assert!(again.is_err());

    assert_eq!(
        app.totp_status(&ctx_for(SeedUser::Bob))
            .await
            .unwrap()
            .recovery_remaining,
        9
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn disable_blocked_for_enforced_owner_but_allowed_for_member(pool: PgPool) {
    let (app, _d) = setup(pool, true).await;
    // Owner alice: Enrollment, dann Disable verboten.
    let actx = ctx_for(SeedUser::Alice);
    let enrollment = app.totp_begin_enrollment(&actx).await.unwrap();
    let secret = core::base32_decode(&enrollment.secret_base32).unwrap();
    app.totp_confirm_enrollment(&actx, &code(&secret))
        .await
        .unwrap();
    assert!(
        app.totp_disable(&actx).await.is_err(),
        "Owner darf nicht abschalten"
    );

    // Mitglied bob: Disable erlaubt.
    let (_s, _r) = enroll(&app).await;
    let bctx = ctx_for(SeedUser::Bob);
    app.totp_disable(&bctx).await.unwrap();
    assert!(!app.totp_status(&bctx).await.unwrap().enabled);
}

#[sqlx::test(migrations = "./migrations")]
async fn begin_is_rejected_when_totp_already_active(pool: PgPool) {
    let (app, _d) = setup(pool, false).await;
    enroll(&app).await; // bob ist jetzt aktiv
    // Ein neues Enrollment darf das aktive TOTP nicht ueberschreiben/deaktivieren.
    assert!(
        app.totp_begin_enrollment(&ctx_for(SeedUser::Bob))
            .await
            .is_err()
    );
    assert!(
        app.totp_status(&ctx_for(SeedUser::Bob))
            .await
            .unwrap()
            .confirmed
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn reset_of_unknown_user_is_rejected(pool: PgPool) {
    let (app, _d) = setup(pool, false).await;
    // Fremde/erfundene UUID (anderer Mandant) darf nicht zuruecksetzbar sein.
    let stranger = UserId(Uuid::new_v4());
    assert!(
        app.totp_reset_for_user(&ctx_for(SeedUser::Alice), stranger)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn owner_can_reset_member_but_member_cannot(pool: PgPool) {
    let (app, d) = setup(pool, false).await;
    let (_s, _r) = enroll(&app).await; // bob enrolled
    assert!(
        app.totp_status(&ctx_for(SeedUser::Bob))
            .await
            .unwrap()
            .confirmed
    );

    // Mitglied darf nicht zurücksetzen.
    assert!(
        app.totp_reset_for_user(&ctx_for(SeedUser::Bob), d.bob)
            .await
            .is_err()
    );

    // Owner setzt bob zurück.
    app.totp_reset_for_user(&ctx_for(SeedUser::Alice), d.bob)
        .await
        .unwrap();
    assert!(
        !app.totp_status(&ctx_for(SeedUser::Bob))
            .await
            .unwrap()
            .enabled
    );
}
