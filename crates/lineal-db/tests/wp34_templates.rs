#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP34: Default-Templates (`useDefaultTemplate`) und Form-Templates (docs/06 §2.8) gegen echte
//! Datenbank (Seed `acme`).

use lineal_app::ports::Store;
use lineal_app::ports::repos::template::DefaultSlot;
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::usecases::template::{FormAnswer, SaveTemplateInput};
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::enums::TemplateType;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;

async fn setup(pool: PgPool) -> App {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap()
}

async fn template(app: &App, name: &str, data: Value) -> String {
    app.save_template(
        &ctx_for(SeedUser::Alice),
        SaveTemplateInput {
            template_type: Some(TemplateType::Issue),
            name: Some(name.into()),
            team: Some("ENG".into()),
            template_data: Some(data),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .template
    .id
    .to_string()
}

fn form() -> Value {
    json!({
        "priority": 3,
        "formFields": [
            { "id": "t", "kind": "title", "label": "Title", "required": true },
            { "id": "p", "kind": "priority", "label": "Priority" },
            { "id": "d", "kind": "longText", "label": "Details", "required": true },
            { "id": "k", "kind": "dropdown", "label": "Kind", "options": ["bug", "idea"] }
        ]
    })
}

fn answer(id: &str, v: Value) -> FormAnswer {
    FormAnswer {
        field_id: id.into(),
        value: v,
    }
}

fn input() -> CreateIssueInput {
    CreateIssueInput {
        team: "ENG".into(),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn default_template_depends_on_membership_and_slot(pool: PgPool) {
    let app = setup(pool).await;
    let members = template(&app, "Für Mitglieder", json!({ "title": "Aus Vorlage" })).await;
    let non_members = template(&app, "Für Externe", form()).await;
    let alice = ctx_for(SeedUser::Alice);
    // Ohne Default: nichts.
    assert_eq!(
        app.default_template_for_issue(&alice, "ENG").await.unwrap(),
        None
    );
    app.set_team_default_template(&alice, "ENG", DefaultSlot::Members, Some(&members))
        .await
        .unwrap();
    // Form-Templates sind nur für Nicht-Mitglieder als Default zulässig.
    let e = app
        .set_team_default_template(&alice, "ENG", DefaultSlot::Members, Some(&non_members))
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    app.set_team_default_template(&alice, "ENG", DefaultSlot::NonMembers, Some(&non_members))
        .await
        .unwrap();

    // Alice ist Mitglied von ENG, Bob ebenfalls (Seed); Carol (Gast) sieht ENG nicht -> not_found.
    assert_eq!(
        app.default_template_for_issue(&alice, "ENG").await.unwrap(),
        Some(members.clone())
    );
    // WEB (Sub-Team von ENG): Alice ist kein Mitglied -> Nicht-Mitglieder-Slot, Bob Mitglied -> Members-Slot.
    app.set_team_default_template(&alice, "WEB", DefaultSlot::NonMembers, Some(&non_members))
        .await
        .unwrap();
    assert_eq!(
        app.default_template_for_issue(&alice, "WEB").await.unwrap(),
        Some(non_members.clone())
    );
    assert_eq!(
        app.default_template_for_issue(&ctx_for(SeedUser::Bob), "WEB")
            .await
            .unwrap(),
        None
    );
    // Das Default wird angewendet (Titel aus der Vorlage).
    let out = app
        .create_issue_with_form(&alice, &members, input(), vec![], &[])
        .await
        .unwrap();
    assert_eq!(out.root.issue.title, "Aus Vorlage");
}

#[sqlx::test(migrations = "./migrations")]
async fn form_answers_fill_title_priority_and_description(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let id = template(&app, "Bug-Formular", form()).await;
    let out = app
        .create_issue_with_form(
            &alice,
            &id,
            input(),
            vec![
                answer("t", json!("Login schlägt fehl")),
                answer("p", json!("urgent")),
                answer("d", json!("Nach dem Klick passiert nichts.")),
                answer("k", json!("bug")),
            ],
            &[],
        )
        .await
        .unwrap();
    let i = &out.root.issue;
    assert_eq!(i.title, "Login schlägt fehl");
    assert_eq!(
        i.priority.value(),
        1,
        "Antwort überschreibt die Template-Priorität"
    );
    assert_eq!(
        i.description.as_deref(),
        Some("### Details\n\nNach dem Klick passiert nichts.\n\n### Kind\n\nbug")
    );

    // Ausdrücklich übergebene Felder haben Vorrang vor Antworten.
    let out = app
        .create_issue_with_form(
            &alice,
            &id,
            CreateIssueInput {
                title: Some("Eigener Titel".into()),
                ..input()
            },
            vec![answer("t", json!("Ignoriert")), answer("d", json!("x"))],
            &[],
        )
        .await
        .unwrap();
    assert_eq!(out.root.issue.title, "Eigener Titel");
}

#[sqlx::test(migrations = "./migrations")]
async fn form_validation_errors_list_the_fields(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let id = template(&app, "Bug-Formular", form()).await;
    let e = app
        .create_issue_with_form(&alice, &id, input(), vec![answer("p", json!(2))], &[])
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.details["fields"], json!(["t", "d"]));
    // Es entsteht kein Issue.
    // Antworten ohne Formular-Template sind ein Fehler.
    let plain = template(&app, "Normal", json!({ "title": "x" })).await;
    let e = app
        .create_issue_with_form(&alice, &plain, input(), vec![answer("t", json!("x"))], &[])
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
}
