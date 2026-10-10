#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP17: `issue_history` verrät nichts über Objekte, die der Aufrufer nicht sehen darf
//! (Relationen und Verweise auf das private Team DES; Seed `acme`).

use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::issue::{CreateIssueInput, RelationChanges, UpdateIssueInput};
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

struct TestClock(Mutex<DateTime<Utc>>);

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

fn create(team: &str, title: &str) -> CreateIssueInput {
    CreateIssueInput {
        team: team.into(),
        title: Some(title.into()),
        ..Default::default()
    }
}

fn update(id: &str) -> UpdateIssueInput {
    UpdateIssueInput {
        id: id.into(),
        ..Default::default()
    }
}

async fn setup(pool: PgPool) -> (Arc<App>, Arc<TestClock>, SeedData) {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let clock = Arc::new(TestClock(Mutex::new(
        Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap(),
    )));
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = clock.clone();
    (Arc::new(app), clock, data)
}

fn tick(clock: &TestClock) {
    *clock.0.lock().unwrap() += Duration::hours(1);
}

async fn history_json(app: &App, who: &RequestCtx, issue: &str) -> String {
    serde_json::to_string(
        &app.issue_history(who, issue)
            .await
            .unwrap()
            .iter()
            .map(|e| (&e.changes, &e.relation_changes))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn history_hides_relations_and_references_to_invisible_objects(pool: PgPool) {
    let (app, clock, data) = setup(pool).await;
    let (alice, bob) = (ctx_for(SeedUser::Alice), ctx_for(SeedUser::Bob));

    // Ein Issue im privaten Team DES (für Alice unsichtbar) und ein ENG-Issue, das darauf zeigt.
    let des = app
        .create_issue(&bob, create("DES", "Intern"))
        .await
        .unwrap();
    let eng = app
        .create_issue(&bob, create("ENG", "Öffentlich"))
        .await
        .unwrap();
    let eng_ident = eng.issue.identifier.clone();
    tick(&clock);
    app.update_issue(
        &bob,
        UpdateIssueInput {
            parent: Some(Some(des.issue.id.to_string())),
            ..update(&eng_ident)
        },
    )
    .await
    .unwrap();
    tick(&clock);
    app.update_issue(
        &bob,
        UpdateIssueInput {
            relations: RelationChanges {
                blocks: vec![des.issue.identifier.clone()],
                ..Default::default()
            },
            ..update(&eng_ident)
        },
    )
    .await
    .unwrap();
    // Ein Issue wandert aus DES nach ENG: `teamId.from` ist das private Team.
    let moved = app
        .create_issue(&bob, create("DES", "Wird verschoben"))
        .await
        .unwrap();
    tick(&clock);
    app.update_issue(
        &bob,
        UpdateIssueInput {
            team: Some("ENG".into()),
            ..update(&moved.issue.id.to_string())
        },
    )
    .await
    .unwrap();

    // Bob (Mitglied) sieht alles ...
    let bob_view = history_json(&app, &bob, &eng_ident).await;
    assert!(bob_view.contains(&des.issue.identifier), "{bob_view}");
    assert!(bob_view.contains(&des.issue.id.to_string()), "{bob_view}");

    // ... Alice (kein DES-Zugriff) sieht weder Kennung noch UUID des DES-Issues.
    let alice_view = history_json(&app, &alice, &eng_ident).await;
    assert!(
        !alice_view.contains(&des.issue.identifier) && !alice_view.contains("DES-"),
        "{alice_view}"
    );
    assert!(
        !alice_view.contains(&des.issue.id.to_string()),
        "{alice_view}"
    );
    // Die Änderung selbst bleibt sichtbar, nur der Verweis ist `null`.
    assert!(alice_view.contains("parentId"), "{alice_view}");

    // Verschobenes Issue: das frühere (private) Team taucht für Alice nicht auf.
    let moved_new = app
        .get_issue(&alice, &moved.issue.id.to_string())
        .await
        .unwrap()
        .view
        .issue
        .identifier;
    let alice_moved = history_json(&app, &alice, &moved_new).await;
    assert!(
        !alice_moved.contains(&data.des.0.to_string()),
        "{alice_moved}"
    );
    let bob_moved = history_json(&app, &bob, &moved_new).await;
    assert!(bob_moved.contains(&data.des.0.to_string()), "{bob_moved}");
}
