#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-63 N4: Nach einem Teamwechsel darf die Historie (und `stateHistory`) nichts ueber das
//! nun private Altteam verraten – weder Statusnamen/-IDs, noch die alte Kennung, noch Label-IDs
//! dieses Teams. Seed `acme`, privates Team DES (Bob Mitglied, Alice nicht).

use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::repos::label::{LabelKind, NewLabel};
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::issue::{CreateIssueInput, UpdateIssueInput};
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::ids::LabelId;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const DES_LABEL: Uuid = Uuid::from_u128(0x1AB5_0000_0000_7000_8000_0000_0000_00D5);

struct TestClock(Mutex<DateTime<Utc>>);
impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

fn update(id: &str) -> UpdateIssueInput {
    UpdateIssueInput {
        id: id.into(),
        ..Default::default()
    }
}

async fn setup(pool: PgPool) -> (Arc<App>, Arc<TestClock>) {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap();
    // Ein Issue-Label des privaten Teams DES.
    tx.labels()
        .insert(
            LabelKind::Issue,
            &NewLabel {
                id: LabelId(DES_LABEL),
                name: "DESGeheim".into(),
                color: "#112233".into(),
                description: None,
                team_id: Some(data.des),
                parent_id: None,
                is_group: false,
                group_type: None,
                creator_id: Some(data.bob),
                now,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let clock = Arc::new(TestClock(Mutex::new(now)));
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = clock.clone();
    (Arc::new(app), clock)
}

fn tick(clock: &TestClock) {
    *clock.0.lock().unwrap() += Duration::hours(1);
}

/// Alle Werte eines History-Eintrags (Änderungen, Relationen, Label-IDs) als ein Blob für
/// Substring-Prüfungen.
fn history_blob(entries: &[lineal_app::ports::repos::issue::HistoryEntry]) -> String {
    let mut s = String::new();
    for e in entries {
        s.push_str(&e.changes.to_string());
        if let Some(r) = &e.relation_changes {
            s.push_str(&r.to_string());
        }
        for l in e.added_label_ids.iter().flatten() {
            s.push_str(&l.0.to_string());
        }
        for l in e.removed_label_ids.iter().flatten() {
            s.push_str(&l.0.to_string());
        }
    }
    s
}

#[sqlx::test(migrations = "./migrations")]
async fn team_move_history_hides_old_private_team(pool: PgPool) {
    let (app, clock) = setup(pool).await;
    let (alice, bob): (RequestCtx, RequestCtx) = (ctx_for(SeedUser::Alice), ctx_for(SeedUser::Bob));
    let data = SeedData::acme();

    // Issue in DES, mit DES-Label und einem Status (Backlog).
    let issue = app
        .create_issue(
            &bob,
            CreateIssueInput {
                team: "DES".into(),
                title: Some("Wird verschoben".into()),
                labels: Some(vec!["DESGeheim".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = issue.issue.id.to_string();
    let des_state_backlog = issue.issue.state_id.0.to_string();
    let old_ident = issue.issue.identifier.clone();

    // Statuswechsel innerhalb von DES (Backlog -> Todo).
    tick(&clock);
    let changed = app
        .update_issue(
            &bob,
            UpdateIssueInput {
                state: Some("Todo".into()),
                ..update(&id)
            },
        )
        .await
        .unwrap();
    let des_state_todo = changed.issue.state_id.0.to_string();

    // Wechsel ins öffentliche Team ENG.
    tick(&clock);
    app.update_issue(
        &bob,
        UpdateIssueInput {
            team: Some("ENG".into()),
            ..update(&id)
        },
    )
    .await
    .unwrap();

    // Alice sieht jetzt das ENG-Issue über die neue Kennung.
    let eng_ident = app
        .get_issue(&alice, &id)
        .await
        .unwrap()
        .view
        .issue
        .identifier;

    // Bob (Mitglied DES) sieht die volle Historie.
    let bob_hist = history_blob(&app.issue_history(&bob, &eng_ident).await.unwrap());
    assert!(bob_hist.contains(&des_state_backlog), "{bob_hist}");
    assert!(bob_hist.contains(&DES_LABEL.to_string()), "{bob_hist}");

    // Alice darf nichts über das private Altteam DES erfahren.
    let alice_hist = history_blob(&app.issue_history(&alice, &eng_ident).await.unwrap());
    assert!(
        !alice_hist.contains(&des_state_backlog),
        "Status-ID des Altteams darf nicht in der Historie stehen (N4): {alice_hist}"
    );
    assert!(
        !alice_hist.contains(&des_state_todo),
        "Status-ID des Altteams darf nicht in der Historie stehen (N4): {alice_hist}"
    );
    assert!(
        !alice_hist.contains(&DES_LABEL.to_string()),
        "Label-ID des Altteams darf nicht in der Historie stehen (N4): {alice_hist}"
    );
    assert!(
        !alice_hist.contains(&old_ident) && !alice_hist.contains(&data.des.0.to_string()),
        "alte Kennung/Team-ID des Altteams darf nicht in der Historie stehen (N4): {alice_hist} (alt: {old_ident})"
    );

    // stateHistory darf keine Spanne (und damit keinen Namen/keine ID) des Altteams zeigen.
    let detail = app.get_issue(&alice, &eng_ident).await.unwrap();
    let span_blob: String = detail
        .state_history
        .iter()
        .map(|s| format!("{}|{}", s.state_id.0, s.name))
        .collect::<Vec<_>>()
        .join(";");
    assert!(
        !span_blob.contains(&des_state_backlog) && !span_blob.contains(&des_state_todo),
        "stateHistory darf keine Status des Altteams enthalten (N4): {span_blob}"
    );

    // Bob sieht in stateHistory sehr wohl die DES-Status.
    let bob_detail = app.get_issue(&bob, &eng_ident).await.unwrap();
    let bob_spans: String = bob_detail
        .state_history
        .iter()
        .map(|s| s.state_id.0.to_string())
        .collect::<Vec<_>>()
        .join(";");
    assert!(bob_spans.contains(&des_state_backlog), "{bob_spans}");

    // Residuum zu N4: previousIdentifiers darf die alte Kennung des privaten Altteams DES nicht
    // an Alice zeigen; Bob (DES-Mitglied) sieht sie sehr wohl.
    assert!(
        !detail.view.previous_identifiers.contains(&old_ident),
        "previousIdentifiers leakt die alte DES-Kennung an Alice: {:?}",
        detail.view.previous_identifiers
    );
    assert!(
        bob_detail.view.previous_identifiers.contains(&old_ident),
        "Bob (DES-Mitglied) muss die alte Kennung in previousIdentifiers sehen: {:?}",
        bob_detail.view.previous_identifiers
    );
}
