#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP34: Dokumente – Kaskade beim Löschen von Issue, Projekt und Team, endgültiges Löschen samt
//! Inhalt, Bündelung von Snapshots, Erwähnungen und Verlinkung von Issue-Kennungen.

use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::Store;
use lineal_app::usecases::document::{ParentRefs, SaveDocumentInput};
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_jobs::PeriodicJob;
use lineal_jobs::periodic::purge::PurgeJob;
use lineal_testkit::seed::planning::{seed_cycle_id, seed_project_id};
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap()
}

async fn setup(pool: &PgPool) -> Arc<App> {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    test_app(Arc::new(store))
}

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}

fn doc_in(parent: ParentRefs, title: &str, content: &str) -> SaveDocumentInput {
    SaveDocumentInput {
        title: Some(title.into()),
        content: Some(content.into()),
        parent,
        ..Default::default()
    }
}

async fn new_issue(app: &App, title: &str) -> String {
    app.create_issue(
        &alice(),
        CreateIssueInput {
            team: "ENG".into(),
            title: Some(title.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .issue
    .identifier
}

async fn flags(pool: &PgPool, id: Uuid) -> (bool, bool, bool) {
    sqlx::query_as(
        "SELECT trashed, trashed_with_parent, trashed_at IS NOT NULL FROM document WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issue_trash_cascades_to_documents_and_restore_keeps_manual_deletions(pool: PgPool) {
    let app = setup(&pool).await;
    let ident = new_issue(&app, "Mit Dokumenten").await;
    let parent = || ParentRefs {
        issue: Some(ident.clone()),
        ..Default::default()
    };
    let a = app
        .save_document(&alice(), doc_in(parent(), "A", "a"))
        .await
        .unwrap();
    let b = app
        .save_document(&alice(), doc_in(parent(), "B", "b"))
        .await
        .unwrap();
    // B wurde vorher einzeln gelöscht.
    app.delete_document(&alice(), &b.doc.id.to_string())
        .await
        .unwrap();

    app.delete_issue(&alice(), &ident, false).await.unwrap();
    assert_eq!(flags(&pool, a.doc.id.0).await, (true, true, true));
    assert_eq!(
        flags(&pool, b.doc.id.0).await,
        (true, false, true),
        "einzeln gelöscht bleibt einzeln"
    );
    let err = app
        .get_document(&alice(), &a.doc.id.to_string())
        .await
        .expect_err("trashed");
    assert_eq!(err.kind, ErrorKind::NotFound);

    app.restore_issue(&alice(), &ident).await.unwrap();
    assert_eq!(flags(&pool, a.doc.id.0).await, (false, false, false));
    assert!(flags(&pool, b.doc.id.0).await.0);
    app.get_document(&alice(), &a.doc.id.to_string())
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn project_trash_cascades_to_documents(pool: PgPool) {
    let app = setup(&pool).await;
    let project = seed_project_id().0.to_string();
    let d = app
        .save_document(
            &alice(),
            doc_in(
                ParentRefs {
                    project: Some(project.clone()),
                    ..Default::default()
                },
                "Projektnotiz",
                "x",
            ),
        )
        .await
        .unwrap();
    app.set_project_trashed(&alice(), &project, true)
        .await
        .unwrap();
    assert_eq!(flags(&pool, d.doc.id.0).await, (true, true, true));
    app.set_project_trashed(&alice(), &project, false)
        .await
        .unwrap();
    assert_eq!(flags(&pool, d.doc.id.0).await, (false, false, false));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn team_delete_cascades_to_team_and_cycle_documents(pool: PgPool) {
    let app = setup(&pool).await;
    let team_doc = app
        .save_document(
            &alice(),
            doc_in(
                ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                "Team-Wiki",
                "x",
            ),
        )
        .await
        .unwrap();
    let cycle_doc = app
        .save_document(
            &alice(),
            doc_in(
                ParentRefs {
                    cycle: Some(seed_cycle_id(3).0.to_string()),
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                "Cycle-Plan",
                "y",
            ),
        )
        .await
        .unwrap();
    app.delete_team(
        &alice(),
        "ENG",
        Some(lineal_app::usecases::team_lifecycle::SubTeamHandling::Unnest),
    )
    .await
    .unwrap();
    assert_eq!(flags(&pool, team_doc.doc.id.0).await, (true, true, true));
    assert_eq!(flags(&pool, cycle_doc.doc.id.0).await, (true, true, true));
    app.restore_deleted_team(&alice(), &SeedData::acme().eng.0.to_string())
        .await
        .unwrap();
    assert_eq!(flags(&pool, team_doc.doc.id.0).await, (false, false, false));
    assert_eq!(
        flags(&pool, cycle_doc.doc.id.0).await,
        (false, false, false)
    );
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn purge_removes_trashed_documents_with_content_history_and_orphans(pool: PgPool) {
    let app = setup(&pool).await;
    let d = app
        .save_document(
            &alice(),
            doc_in(
                ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                "Wegwerfdokument",
                "v1",
            ),
        )
        .await
        .unwrap();
    // Dokument am Issue: wird beim endgültigen Löschen des Issues per Fremdschlüssel entfernt.
    let ident = new_issue(&app, "Wird gelöscht").await;
    let on_issue = app
        .save_document(
            &alice(),
            doc_in(
                ParentRefs {
                    issue: Some(ident.clone()),
                    ..Default::default()
                },
                "Am Issue",
                "z",
            ),
        )
        .await
        .unwrap();
    app.delete_issue(&alice(), &ident, true).await.unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM document_content WHERE owner_type = 'document'"
        )
        .await,
        2,
        "der Inhalt des Issue-Dokuments ist verwaist"
    );

    app.delete_document(&alice(), &d.doc.id.to_string())
        .await
        .unwrap();
    // Vor Ablauf der Frist: nichts gelöscht, nur der verwaiste Inhalt wird abgeräumt.
    PurgeJob.run(&app, t0() + Duration::days(10)).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM document").await, 1);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM document_content WHERE owner_type = 'document'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            &format!(
                "SELECT count(*) FROM document_content WHERE owner_id = '{}'",
                on_issue.doc.id.0
            )
        )
        .await,
        0
    );
    // Nach 30 Tagen: Dokument samt Inhalt und Snapshots endgültig weg.
    PurgeJob.run(&app, t0() + Duration::days(31)).await.unwrap();
    assert_eq!(count(&pool, "SELECT count(*) FROM document").await, 0);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM document_content").await,
        0
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM document_content_history").await,
        0
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn purge_removes_trashed_projects_and_detaches_issues(pool: PgPool) {
    let app = setup(&pool).await;
    let project = seed_project_id().0;
    let issue = app
        .create_issue(
            &alice(),
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Im Projekt".into()),
                project: Some(project.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .issue
        .identifier;
    app.set_project_trashed(&alice(), &project.to_string(), true)
        .await
        .unwrap();
    // `trashed_at` setzt die Datenbank mit der echten Uhr; deshalb weit nach der Testuhr.
    let stats = app
        .run_trash_purge(t0() + Duration::days(400))
        .await
        .unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(
        count(
            &pool,
            &format!("SELECT count(*) FROM project WHERE id = '{project}'")
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            &format!("SELECT count(*) FROM issue WHERE project_id = '{project}'")
        )
        .await,
        0
    );
    // Das Issue des Projekts bleibt erhalten.
    assert_eq!(
        count(
            &pool,
            &format!("SELECT count(*) FROM issue WHERE identifier = '{issue}'")
        )
        .await,
        1
    );
}

async fn history(pool: &PgPool, doc: Uuid) -> Vec<(String, Vec<Uuid>)> {
    sqlx::query_as(
        "SELECT h.kind, h.actor_ids FROM document_content_history h \
         JOIN document_content c ON c.id = h.document_content_id \
         WHERE c.owner_type = 'document' AND c.owner_id = $1 ORDER BY h.content_snapshot_at, h.id",
    )
    .bind(doc)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn autosave_bundles_edits_into_one_snapshot_explicit_save_does_not(pool: PgPool) {
    let app = setup(&pool).await;
    let d = app
        .save_document(
            &alice(),
            doc_in(
                ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                "Entwurf",
                "eins",
            ),
        )
        .await
        .unwrap();
    let id = d.doc.id.to_string();
    let edit = |content: &str| SaveDocumentInput {
        id: Some(id.clone()),
        content: Some(content.into()),
        ..Default::default()
    };
    // Erste Änderung nach dem Anlegen: eigener `edited`-Snapshot; weitere werden gebündelt.
    app.save_document_autosave(&alice(), edit("zwei"))
        .await
        .unwrap();
    app.save_document_autosave(&alice(), edit("drei"))
        .await
        .unwrap();
    app.save_document_autosave(&alice(), edit("vier"))
        .await
        .unwrap();
    let h = history(&pool, d.doc.id.0).await;
    assert_eq!(
        h.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
        vec!["created", "edited"]
    );
    let latest: String = sqlx::query_scalar(
        "SELECT h.markdown FROM document_content_history h JOIN document_content c \
         ON c.id = h.document_content_id WHERE c.owner_id = $1 AND h.kind = 'edited'",
    )
    .bind(d.doc.id.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(latest, "vier");

    // Ausdrückliches Speichern: immer ein eigener Snapshot.
    app.save_document(&alice(), edit("fünf")).await.unwrap();
    app.save_document(&alice(), edit("sechs")).await.unwrap();
    assert_eq!(history(&pool, d.doc.id.0).await.len(), 4);
}

#[test]
fn bundle_windows() {
    use lineal_app::ports::repos::document::{HistoryEntry, HistoryKind};
    use lineal_app::usecases::document::snapshot_can_be_bundled;
    let entry = |kind, created| HistoryEntry {
        id: Uuid::nil(),
        document_content_id: Uuid::nil(),
        markdown: String::new(),
        title: None,
        actor_ids: vec![],
        content_snapshot_at: created,
        created_at: created,
        kind,
        restored_from_id: None,
    };
    let start = t0();
    let e = entry(HistoryKind::Edited, start);
    assert!(snapshot_can_be_bundled(
        &e,
        start + Duration::minutes(5),
        start + Duration::minutes(9)
    ));
    // 10 Minuten Ruhe: neuer Snapshot.
    assert!(!snapshot_can_be_bundled(
        &e,
        start + Duration::minutes(5),
        start + Duration::minutes(15)
    ));
    // 60 Minuten seit dem Snapshot: neuer Snapshot, auch bei laufender Bearbeitung.
    assert!(!snapshot_can_be_bundled(
        &e,
        start + Duration::minutes(59),
        start + Duration::minutes(61)
    ));
    // `created`/`restored` werden nie überschrieben.
    for kind in [HistoryKind::Created, HistoryKind::Restored] {
        assert!(!snapshot_can_be_bundled(
            &entry(kind, start),
            start,
            start + Duration::minutes(1)
        ));
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn mentions_notify_subscribe_and_skip_the_author(pool: PgPool) {
    let app = setup(&pool).await;
    let d = app
        .save_document(
            &alice(),
            doc_in(
                ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                "Plan",
                "Bitte @bob prüfen, danke @alice und @unbekannt.",
            ),
        )
        .await
        .unwrap();
    let bob_id = SeedData::acme().bob.0;
    let n: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT user_id, type, target_type FROM notification WHERE document_id = $1",
    )
    .bind(d.doc.id.0)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        n,
        vec![(bob_id, "documentMention".into(), "document".into())],
        "nur Bob; die Autorin selbst bekommt nichts"
    );
    let subs: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM document_subscription WHERE document_id = $1 AND reason = 'mention'",
    )
    .bind(d.doc.id.0)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(subs, vec![bob_id]);

    // Dieselbe Erwähnung erneut: keine weitere Benachrichtigung; eine neue Erwähnung (Carol: Gast,
    // sieht ENG nicht) erzeugt keine.
    app.save_document(
        &alice(),
        SaveDocumentInput {
            id: Some(d.doc.id.to_string()),
            content: Some("Bitte @bob prüfen, danke @alice. Auch @carol.".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM notification WHERE type = 'documentMention'"
        )
        .await,
        1
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn output_links_visible_issue_identifiers_but_storage_stays_plain(pool: PgPool) {
    let app = setup(&pool).await;
    let ident = new_issue(&app, "Verlinkt").await;
    let content = format!("Siehe {ident} und `{ident}` sowie ZZZ-99.");
    let d = app
        .save_document(
            &alice(),
            doc_in(
                ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                "Verweise",
                &content,
            ),
        )
        .await
        .unwrap();
    let link = format!("[{ident}](/acme/issue/{ident})");
    assert_eq!(
        d.doc.content,
        format!("Siehe {link} und `{ident}` sowie ZZZ-99."),
        "Klartext-Kennung verlinkt, Code und unbekannte Kennung nicht"
    );
    let stored: String = sqlx::query_scalar(
        "SELECT markdown FROM document_content WHERE owner_type = 'document' AND owner_id = $1",
    )
    .bind(d.doc.id.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, content);
    // Bob (Mitglied von ENG) sieht den Link; Carol (Gast ohne Team) sieht das Dokument nicht.
    let for_bob = app
        .get_document(&ctx_for(SeedUser::Bob), &d.doc.id.to_string())
        .await
        .unwrap();
    assert!(for_bob.doc.content.contains(&link));
}
