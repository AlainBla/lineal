#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_arguments,
    dead_code,
    unused_imports
)]
//! Paket 5 (Testlauf-Bugs): Planung und Settings gegen echte Datenbank.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::ports::{Clock, IdGen, Store};
use lineal_app::usecases::cycle::{CycleKind, CycleSettingsInput};
use lineal_app::usecases::initiative::{ListInitiativesInput, SaveInitiativeInput};
use lineal_app::usecases::project::milestone::SaveMilestoneInput;
use lineal_app::usecases::project::{ListProjectsInput, SaveProjectInput, ViewOpts};
use lineal_app::usecases::team::CreateTeamInput;
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::{Health, TeamVisibility};
use lineal_testkit::seed::planning::{seed_cycle_id, seed_initiative_id, seed_project_id};
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct At(DateTime<Utc>);
impl Clock for At {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}
struct V7;
impl IdGen for V7 {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, mi, 0).single().unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn app_at(pool: &PgPool, t: DateTime<Utc>) -> App {
    let mut app = Arc::try_unwrap(test_app(Arc::new(PgStore::new(pool.clone()))))
        .ok()
        .unwrap();
    app.clock = Arc::new(At(t));
    app.ids = Arc::new(V7);
    app
}

/// Seed `acme`; Uhr = 2026-10-10T07:15Z (wie die Cycle-Fenster des Seeds).
async fn setup(pool: PgPool) -> (App, SeedData, PgPool) {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (app_at(&pool, utc(2026, 10, 10, 7, 15)), data, pool)
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn teams_input(teams: &[&str]) -> SaveProjectInput {
    SaveProjectInput {
        name: Some("Relaunch".into()),
        add_teams: teams.iter().map(|t| (*t).to_owned()).collect(),
        ..Default::default()
    }
}

async fn new_team(app: &App, ctx: &RequestCtx, name: &str, key: &str, tz: &str) -> Uuid {
    app.create_team(
        ctx,
        CreateTeamInput {
            name: name.into(),
            key: Some(key.into()),
            timezone: Some(tz.into()),
            visibility: Some(TeamVisibility::Public),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .team
    .id
    .0
}

async fn state_id(pool: &PgPool, team: Uuid, kind: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM workflow_state WHERE team_id = $1 AND type = $2 ORDER BY position LIMIT 1",
    )
    .bind(team)
    .bind(kind)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
async fn issue(
    pool: &PgPool,
    org: Uuid,
    team: Uuid,
    number: i32,
    state_kind: &str,
    cycle: Option<Uuid>,
    project: Option<Uuid>,
) -> Uuid {
    let state = state_id(pool, team, state_kind).await;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, cycle_id, project_id) \
         VALUES ($1, $2, $3, $4, 'Testissue', $5, $6, $7)",
    )
    .bind(id)
    .bind(org)
    .bind(team)
    .bind(number)
    .bind(state)
    .bind(cycle)
    .bind(project)
    .execute(pool)
    .await
    .unwrap();
    id
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn b5_01_shift_all_rejects_out_of_range_days_without_panic(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let c3 = seed_cycle_id(3).to_string();
    for days in [
        2_000_000_000_i64,
        -2_000_000_000,
        2_147_483_647,
        100_000,
        3661,
        -3661,
    ] {
        let e = app.shift_cycles(&alice, &c3, days).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidRequest, "days={days}");
        assert_eq!(e.details["field"], "daysToShift");
    }
    app.shift_cycles(&alice, &c3, 3660).await.unwrap();
}
