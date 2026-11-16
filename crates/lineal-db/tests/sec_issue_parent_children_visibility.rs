#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-63 N1: Die Anzeigewerte (`extras`) eines Issues dürfen nichts über unsichtbare
//! Nachbar-Issues verraten – weder die Kennung eines Parents in einem unsichtbaren Team
//! noch die Zahl von Children in unsichtbaren Teams. Seed `acme`, privates Team DES
//! (Bob Mitglied, Alice nicht).

use lineal_app::ports::Store;
use lineal_app::usecases::issue::{CreateIssueInput, UpdateIssueInput};
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

fn create(team: &str, title: &str) -> CreateIssueInput {
    CreateIssueInput {
        team: team.into(),
        title: Some(title.into()),
        ..Default::default()
    }
}

async fn setup(pool: PgPool) -> (Arc<App>, SeedData) {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (test_app(Arc::new(store)), data)
}

/// Parent in DES (unsichtbar für Alice) → `parent_identifier` muss `None` sein; ein DES-Child
/// eines ENG-Issues darf `children_count` für Alice nicht erhöhen.
#[sqlx::test(migrations = "./migrations")]
async fn extras_hide_invisible_parent_and_children(pool: PgPool) {
    let (app, _data) = setup(pool).await;
    let (alice, bob): (RequestCtx, RequestCtx) = (ctx_for(SeedUser::Alice), ctx_for(SeedUser::Bob));

    // ENG-Parent (öffentlich) mit einem versteckten Child in DES.
    let eng_parent = app
        .create_issue(&bob, create("ENG", "Oeffentlicher Parent"))
        .await
        .unwrap();
    app.create_issue(
        &bob,
        CreateIssueInput {
            parent: Some(eng_parent.issue.id.to_string()),
            ..create("DES", "Verstecktes Child")
        },
    )
    .await
    .unwrap();

    // ENG-Child mit Parent in DES.
    let des_parent = app
        .create_issue(&bob, create("DES", "Versteckter Parent"))
        .await
        .unwrap();
    let eng_child = app
        .create_issue(&bob, create("ENG", "Oeffentliches Child"))
        .await
        .unwrap();
    app.update_issue(
        &bob,
        UpdateIssueInput {
            parent: Some(Some(des_parent.issue.id.to_string())),
            ..UpdateIssueInput {
                id: eng_child.issue.id.to_string(),
                ..Default::default()
            }
        },
    )
    .await
    .unwrap();

    // Bob (Mitglied DES) sieht alles.
    let bob_parent = app
        .get_issue(&bob, &eng_parent.issue.id.to_string())
        .await
        .unwrap();
    assert_eq!(bob_parent.view.extras.children_count, 1);
    let bob_child = app
        .get_issue(&bob, &eng_child.issue.id.to_string())
        .await
        .unwrap();
    assert_eq!(
        bob_child.view.extras.parent_identifier,
        Some(des_parent.issue.identifier.clone())
    );

    // Alice (kein DES-Zugriff) darf weder das DES-Child zählen noch die DES-Parent-Kennung sehen.
    let alice_parent = app
        .get_issue(&alice, &eng_parent.issue.id.to_string())
        .await
        .unwrap();
    assert_eq!(
        alice_parent.view.extras.children_count, 0,
        "childrenCount darf unsichtbare Children nicht zählen (N1)"
    );
    let alice_child = app
        .get_issue(&alice, &eng_child.issue.id.to_string())
        .await
        .unwrap();
    assert_eq!(
        alice_child.view.extras.parent_identifier, None,
        "parentId darf die Kennung eines unsichtbaren Parents nicht zeigen (N1)"
    );
}
