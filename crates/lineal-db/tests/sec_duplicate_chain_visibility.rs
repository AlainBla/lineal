#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-63 N3: Die Duplikat-Kette (`canonical_end`) darf nicht ungeprueft in ein fuer den
//! Aufrufer unsichtbares Issue springen. Sonst haengen Relation, History und Abonnements an
//! einem Issue in einem privaten Team, das der Aufrufer nicht sehen darf. Seed `acme`,
//! privates Team DES (Bob Mitglied, Alice nicht).

use lineal_app::ports::Store;
use lineal_app::usecases::issue::{CreateIssueInput, RelationChanges, UpdateIssueInput};
use lineal_app::{App, ErrorKind, RequestCtx};
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

fn dup_of(id: &str, reference: &str) -> UpdateIssueInput {
    UpdateIssueInput {
        id: id.into(),
        relations: RelationChanges {
            duplicate_of: Some(Some(reference.into())),
            ..Default::default()
        },
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

/// Eine sichtbare Zwischenstation (ENG) ist Duplikat eines privaten DES-Issues. Markiert Alice
/// ihr ENG-Issue als Duplikat der Zwischenstation, darf die Kette nicht bis zum DES-Issue
/// springen: der Aufruf endet mit `not_found`, und das DES-Issue bleibt unberuehrt.
#[sqlx::test(migrations = "./migrations")]
async fn duplicate_chain_does_not_reach_invisible_canonical(pool: PgPool) {
    let (app, _data) = setup(pool).await;
    let (alice, bob): (RequestCtx, RequestCtx) = (ctx_for(SeedUser::Alice), ctx_for(SeedUser::Bob));

    // Privates kanonisches Issue in DES.
    let canonical = app
        .create_issue(&bob, create("DES", "Privates Original"))
        .await
        .unwrap();
    // Sichtbare Zwischenstation in ENG, von Bob als Duplikat des DES-Issues markiert.
    let middle = app
        .create_issue(&bob, create("ENG", "Sichtbares Duplikat"))
        .await
        .unwrap();
    app.update_issue(
        &bob,
        dup_of(
            &middle.issue.id.to_string(),
            &canonical.issue.id.to_string(),
        ),
    )
    .await
    .unwrap();

    // Alice sieht die Zwischenstation, nicht aber das DES-Issue dahinter.
    let x = app
        .create_issue(&alice, create("ENG", "Alices Issue"))
        .await
        .unwrap();
    let res = app
        .update_issue(
            &alice,
            dup_of(&x.issue.id.to_string(), &middle.issue.id.to_string()),
        )
        .await;
    assert_eq!(
        res.map(|_| ()).unwrap_err().kind,
        ErrorKind::NotFound,
        "die Kette darf nicht in das unsichtbare DES-Issue springen (N3)"
    );

    // Das private Issue blieb unberuehrt: keine neue Duplikat-Relation von Alices Issue.
    let canon_rels = app
        .issue_relations(&bob, &canonical.issue.id.to_string())
        .await
        .unwrap();
    assert!(
        !canon_rels
            .duplicates
            .iter()
            .any(|r| r.identifier == x.issue.identifier),
        "das unsichtbare Issue darf keine Relation zu Alices Issue erhalten (N3)"
    );
}
