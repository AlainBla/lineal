//! Contract-Tests für die Gruppe `triage` gegen echte Datenbank (Seed `acme`).

use crate::dispatch::dispatch;
use chrono::{Duration, TimeZone, Utc};
use lineal_app::ports::Store;
use lineal_app::ports::repos::triage::TriageAction;
use lineal_app::usecases::team::UpdateTeamInput;
use lineal_app::usecases::triage::{ScheduleEntryInput, SetTriageResponsibilityInput};
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;

struct Env {
    app: Arc<App>,
    d: SeedData,
}

async fn setup(pool: PgPool) -> Env {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    Env {
        app: test_app(Arc::new(store)),
        d: SeedData::acme(),
    }
}

async fn call(env: &Env, who: SeedUser, args: Value) -> Result<Value, lineal_app::AppError> {
    dispatch(&env.app, &ctx_for(who), "get_triage_responsibility", args)
        .await
        .expect("known tool")
}

async fn enable_triage(env: &Env, who: SeedUser, team: &str) {
    env.app
        .update_team(
            &ctx_for(who),
            team,
            UpdateTeamInput {
                triage_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn disabled_triage_returns_all_keys_with_null_values(pool: PgPool) {
    let env = setup(pool).await;
    let v = call(&env, SeedUser::Bob, json!({ "team": "ENG" }))
        .await
        .unwrap();
    assert_eq!(v["triageEnabled"], false);
    assert_eq!(v["team"]["key"], "ENG");
    for key in ["action", "currentUser", "rotation"] {
        assert!(
            v.get(key).is_some_and(Value::is_null),
            "{key} muss null sein"
        );
    }
    assert_eq!(v["upcomingShifts"], json!([]));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn enabled_triage_without_responsibility_has_no_current_user(pool: PgPool) {
    let env = setup(pool).await;
    enable_triage(&env, SeedUser::Alice, "ENG").await;
    let v = call(&env, SeedUser::Bob, json!({ "team": "eng" }))
        .await
        .unwrap();
    assert_eq!(v["triageEnabled"], true);
    assert!(v["action"].is_null() && v["currentUser"].is_null() && v["rotation"].is_null());
    assert_eq!(v["upcomingShifts"], json!([]));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn rotation_lists_current_user_and_upcoming_shifts(pool: PgPool) {
    let env = setup(pool).await;
    enable_triage(&env, SeedUser::Alice, "ENG").await;
    let alice = ctx_for(SeedUser::Alice);
    let t0 = Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap();
    let week = Duration::days(7);
    let entry = |user: &str, from| ScheduleEntryInput {
        user: Some(user.into()),
        user_email: None,
        starts_at: from,
        ends_at: from + week,
        overridden_user: None,
    };
    let (schedule, _) = env
        .app
        .create_time_schedule(
            &alice,
            "ENG Triage",
            vec![
                entry("Bob Example", t0 - Duration::days(2)),
                entry("Alice Example", t0 + Duration::days(5)),
                entry("Bob Example", t0 + Duration::days(12)),
            ],
        )
        .await
        .unwrap();
    env.app
        .set_triage_responsibility(
            &alice,
            SetTriageResponsibilityInput {
                team: "ENG".into(),
                action: Some(TriageAction::Assign),
                users: vec![],
                schedule: Some(schedule.id.to_string()),
            },
        )
        .await
        .unwrap();
    let v = call(&env, SeedUser::Bob, json!({ "team": "ENG" }))
        .await
        .unwrap();
    assert_eq!(v["action"], "assign");
    assert_eq!(v["currentUser"]["id"], env.d.bob.to_string());
    assert_eq!(v["currentUser"]["name"], "Bob Example");
    assert_eq!(v["rotation"]["name"], "ENG Triage");
    assert_eq!(v["rotation"]["shiftLength"], "P1W");
    assert_eq!(v["upcomingShifts"].as_array().unwrap().len(), 3);
    assert_eq!(v["upcomingShifts"][1]["user"]["name"], "Alice Example");
    assert!(
        v["upcomingShifts"][0]["startsAt"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    // E-Mail-Adressen erscheinen nie
    assert!(!v.to_string().contains('@'));
    // `limit` und `user`
    let v = call(&env, SeedUser::Bob, json!({ "team": "ENG", "limit": 1 }))
        .await
        .unwrap();
    assert_eq!(v["upcomingShifts"].as_array().unwrap().len(), 1);
    let v = call(
        &env,
        SeedUser::Bob,
        json!({ "team": "ENG", "user": "Alice Example" }),
    )
    .await
    .unwrap();
    let shifts = v["upcomingShifts"].as_array().unwrap();
    assert_eq!(shifts.len(), 1);
    assert_eq!(shifts[0]["user"]["name"], "Alice Example");
    let v = call(&env, SeedUser::Bob, json!({ "team": "ENG", "user": "me" }))
        .await
        .unwrap();
    assert_eq!(v["upcomingShifts"].as_array().unwrap().len(), 2);
    // unbekannter User
    assert_eq!(
        call(
            &env,
            SeedUser::Bob,
            json!({ "team": "ENG", "user": "Niemand" })
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn unknown_and_private_teams_are_not_found(pool: PgPool) {
    let env = setup(pool).await;
    enable_triage(&env, SeedUser::Bob, "DES").await;
    // Carol (Gast) und Alice (Owner, kein Mitglied) sehen das private Team DES nicht.
    for who in [SeedUser::Carol, SeedUser::Alice] {
        assert_eq!(
            call(&env, who, json!({ "team": "DES" }))
                .await
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
    }
    assert_eq!(
        call(&env, SeedUser::Bob, json!({ "team": "DES" }))
            .await
            .unwrap()["triageEnabled"],
        true
    );
    assert_eq!(
        call(&env, SeedUser::Bob, json!({ "team": "NOPE" }))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}
