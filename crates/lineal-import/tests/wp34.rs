#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP34: Custom Views, Favoriten, Issue-Reaktionen, Kommentare an Projekt-Updates und der
//! Dry-Run mit Download-Zählung (Fake-Quelle, kein Zugriff auf das echte Linear).

mod common;

use common::*;
use lineal_app::usecases::import::ImportStartInput;
use lineal_import::engine::{EngineConfig, RunOutcome};
use lineal_import::{FakeSource, ImportSource, SourceEntity};
use serde_json::{Value, json};
use sqlx::PgPool;

fn id(s: &str) -> Value {
    json!({ "id": s })
}

fn add_views(source: &FakeSource) {
    source.set_entity(
        SourceEntity::CustomViews,
        vec![
            json!({
                "id": "v-1", "name": "Open work", "modelName": "Issue", "shared": true,
                "owner": id("u-alice"), "creator": id("u-alice"), "team": id("t-eng"),
                "icon": "bug", "color": "#4F46E5", "description": "All open issues",
                "filterData": { "and": [
                    { "assignee": { "id": { "eq": "u-bob" } } },
                    { "state": { "type": { "in": ["started", "unstarted"] } } },
                    { "customerCount": { "gt": 1 } },
                    { "labels": { "some": { "id": { "in": ["l-gone"] } } } },
                ]},
                "createdAt": at(10), "updatedAt": at(11),
            }),
            json!({
                "id": "v-2", "name": "Bob's private", "modelName": "Issue", "shared": false,
                "owner": id("u-bob"), "creator": id("u-bob"),
                "filterData": { "priority": { "lte": 2 } },
                "createdAt": at(10), "updatedAt": at(11),
            }),
            json!({
                "id": "v-3", "name": "Ghost view", "modelName": "Issue", "shared": false,
                "owner": id("u-ghost"), "filterData": {}, "createdAt": at(10), "updatedAt": at(11),
            }),
            json!({
                "id": "v-4", "name": "Customers", "modelName": "Customer", "shared": true,
                "owner": id("u-alice"), "filterData": {}, "createdAt": at(10), "updatedAt": at(11),
            }),
            json!({
                "id": "v-5", "name": "Broken", "modelName": "Issue", "shared": true,
                "owner": id("u-alice"), "filterData": "not-an-object",
                "createdAt": at(10), "updatedAt": at(11),
            }),
        ],
    );
    source.set_entity(
        SourceEntity::Favorites,
        vec![
            json!({ "id": "f-issue", "type": "issue", "owner": id("u-alice"), "issue": id("i-ENG-1"),
                    "parent": id("f-folder"), "sortOrder": 2.0, "createdAt": at(20), "updatedAt": at(20) }),
            json!({ "id": "f-folder", "type": "folder", "folderName": "Pinned", "owner": id("u-alice"),
                    "sortOrder": 1.0, "createdAt": at(20), "updatedAt": at(20) }),
            json!({ "id": "f-view", "type": "customView", "owner": id("u-alice"), "customView": id("v-1"),
                    "sortOrder": 3.0, "createdAt": at(20), "updatedAt": at(20) }),
            json!({ "id": "f-gone", "type": "issue", "owner": id("u-alice"), "issue": id("i-NOPE"),
                    "sortOrder": 4.0, "createdAt": at(20), "updatedAt": at(20) }),
            json!({ "id": "f-pre", "type": "predefinedView", "owner": id("u-bob"), "predefinedViewType": "myIssues",
                    "sortOrder": 5.0, "createdAt": at(20), "updatedAt": at(20) }),
            json!({ "id": "f-odd", "type": "dashboard", "owner": id("u-alice"),
                    "sortOrder": 6.0, "createdAt": at(20), "updatedAt": at(20) }),
        ],
    );
}

fn at(h: i64) -> String {
    lineal_import::fixtures::at(h)
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn custom_views_and_favorites_are_translated_with_warnings(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    add_views(&source);
    let (run, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);

    // Views: v-1 und v-2 übernommen; Geister-Eigentümer, Fremdtyp und Unfug nicht.
    assert_eq!(e.count("SELECT count(*) FROM custom_view").await, 2);
    let (filter, shared, team, icon): (Value, bool, Option<uuid::Uuid>, Option<String>) =
        sqlx::query_as(
            "SELECT filter_data, shared, team_id, icon FROM custom_view WHERE name = 'Open work'",
        )
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert!(shared && team.is_some());
    assert_eq!(icon.as_deref(), Some("bug"));
    let text = filter.to_string();
    assert!(
        !text.contains("customerCount"),
        "unknown field dropped: {text}"
    );
    assert!(!text.contains("u-bob"), "source ids are replaced: {text}");
    assert!(!text.contains("l-gone"), "{text}");
    let bob: uuid::Uuid = e.bob.0;
    assert!(
        text.contains(&bob.to_string()),
        "Bob's id is mapped: {text}"
    );
    assert!(text.contains("started"), "{text}");
    let owner: uuid::Uuid =
        sqlx::query_scalar("SELECT owner_id FROM custom_view WHERE name = 'Bob''s private'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(owner, bob);

    // Favoriten: Ordner + Kind, View, vordefinierte Ansicht; unbekanntes Ziel/Typ entfallen.
    assert_eq!(e.count("SELECT count(*) FROM favorite").await, 4);
    assert_eq!(
        e.count(
            "SELECT count(*) FROM favorite c JOIN favorite p ON p.id = c.parent_id \
             WHERE p.type = 'folder' AND c.type = 'issue'"
        )
        .await,
        1
    );
    assert_eq!(
        e.count(
            "SELECT count(*) FROM favorite WHERE type = 'customView' AND custom_view_id IS NOT NULL"
        )
        .await,
        1
    );

    // Bericht: Warnung zum entfernten Feld, Hinweise zu den übersprungenen Objekten.
    let (entries, _) = e
        .app
        .import_report(&e.alice_ctx(), run, None, 1000, 0)
        .await
        .unwrap();
    let msgs: Vec<String> = entries.iter().map(|x| x.message.clone()).collect();
    assert!(msgs.iter().any(|m| m.contains("customerCount")), "{msgs:?}");
    assert!(
        msgs.iter().any(|m| m.contains("owner of the view")),
        "{msgs:?}"
    );

    // Zweiter Lauf ändert nichts (Mapping).
    let before = (
        e.count("SELECT count(*) FROM custom_view").await,
        e.count("SELECT count(*) FROM favorite").await,
    );
    let planned = e.scan(&source).await;
    e.start(planned.id, ImportStartInput::default()).await;
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        before,
        (
            e.count("SELECT count(*) FROM custom_view").await,
            e.count("SELECT count(*) FROM favorite").await
        )
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn issue_reactions_and_project_update_comments(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    source.patch(
        SourceEntity::Issues,
        "i-ENG-1",
        &json!({ "reactions": [
            { "id": "ir-1", "emoji": "🎉", "user": id("u-alice"), "createdAt": at(700) },
            { "id": "ir-2", "emoji": "🎉", "user": id("u-bob"), "createdAt": at(701) },
            { "id": "ir-3", "emoji": "🚀", "user": id("u-ghost"), "createdAt": at(701) },
        ]}),
    );
    let mut comments = source.entity(SourceEntity::Comments);
    comments.push(json!({
        "id": "c-pu-1", "body": "Thanks for the update", "user": id("u-bob"),
        "projectUpdate": id("pu-1"), "createdAt": at(910), "updatedAt": at(910),
        "reactions": [{ "id": "cr-1", "emoji": "👍", "user": id("u-alice"), "createdAt": at(911) }],
    }));
    comments.push(json!({
        "id": "c-pu-2", "body": "You are welcome", "user": id("u-alice"),
        "projectUpdate": id("pu-1"), "parent": id("c-pu-1"), "createdAt": at(912), "updatedAt": at(912),
    }));
    comments.push(json!({
        "id": "c-pu-lost", "body": "Orphan", "user": id("u-bob"),
        "projectUpdate": id("pu-unknown"), "createdAt": at(913), "updatedAt": at(913),
    }));
    source.set_entity(SourceEntity::Comments, comments);

    let (_, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);

    // Zwei gültige Reaktionen am Issue (Unbekannter Nutzer entfällt).
    assert_eq!(
        e.count("SELECT count(*) FROM reaction WHERE issue_id IS NOT NULL")
            .await,
        2
    );
    // Zwei Kommentare am Update, die Antwort hängt am Kommentar.
    assert_eq!(
        e.count("SELECT count(*) FROM comment WHERE project_update_id IS NOT NULL")
            .await,
        2
    );
    assert_eq!(
        e.count(
            "SELECT count(*) FROM comment r JOIN comment p ON p.id = r.parent_id \
             WHERE r.project_update_id IS NOT NULL AND p.project_update_id = r.project_update_id"
        )
        .await,
        1
    );
    assert_eq!(
        e.count(
            "SELECT count(*) FROM reaction r JOIN comment c ON c.id = r.comment_id \
             WHERE c.project_update_id IS NOT NULL"
        )
        .await,
        1
    );

    // Wiederholung erzeugt keine Dubletten.
    let before = e.snapshot().await;
    let planned = e.scan(&source).await;
    e.start(planned.id, ImportStartInput::default()).await;
    e.engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(before, e.snapshot().await);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn dry_run_counts_downloads_without_loading_them(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    let planned = e.scan(&source).await;
    e.start(
        planned.id,
        ImportStartInput {
            dry_run: true,
            ..Default::default()
        },
    )
    .await;
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::DryRunFinished);
    assert!(source.downloads().is_empty(), "nothing is downloaded");
    assert_eq!(e.sink.len(), 0);
    let run = e
        .app
        .import_get_run(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    let plan = &run.stats["plan"];
    // Anhänge spec.pdf und missing.png, eingebettet shot.png/diagram.png (je einmal gezählt).
    assert!(plan["downloadFiles"].as_u64().unwrap() >= 3, "plan: {plan}");
    assert!(plan["downloadBytes"].as_u64().unwrap() > 0, "plan: {plan}");
    assert!(
        plan["downloadSizeUnknown"].as_u64().unwrap() >= 1,
        "the missing file has no known size: {plan}"
    );
    let probes = source.probes();
    let mut unique = probes.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(probes.len(), unique.len(), "each file is probed once");
    let _ = source.kind();
}
