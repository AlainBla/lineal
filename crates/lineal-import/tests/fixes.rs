#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Fehlerbehebungen nach dem ersten echten Lauf: Platzhalter-Text bleibt erhalten, Dateien gehen
//! direkt in den Blob-Store, Labels werden zugeordnet, Dokumente ohne Elternobjekt hängen am Team,
//! Antworten vor ihrem Elternkommentar werden nachträglich verknüpft (Fake-Quelle, kein Linear).

mod common;

use async_trait::async_trait;
use common::*;
use lineal_app::AppResult;
use lineal_app::ports::BlobStore;
use lineal_app::usecases::import::ImportStartInput;
use lineal_import::engine::RunOutcome;
use lineal_import::{AppBlobSink, FakeSource, ImportSource, SourceEntity};
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PLACEHOLDERS: &str = "Setze <token> und <uuid> in <datei> auf <host-ipv4>, Mail <e-mail>, \
                            Schema <https>://x und <script>alert(1)</script>.";

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn placeholder_text_survives_in_issues_comments_and_documents(pool: PgPool) {
    let source: FakeSource = fake();
    source.patch(
        SourceEntity::Issues,
        "i-ENG-1",
        &json!({ "description": format!("Beschreibung: {PLACEHOLDERS}\n\n`<code-tag>`") }),
    );
    source.patch(
        SourceEntity::Comments,
        "c-ENG-1",
        &json!({ "body": format!("Kommentar: {PLACEHOLDERS}") }),
    );
    source.patch(
        SourceEntity::Documents,
        "d-3",
        &json!({ "content": format!("# Notiz\n\n{PLACEHOLDERS}") }),
    );
    let e = env(pool.clone()).await;
    let (run, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);

    let texts: Vec<String> = vec![
        sqlx::query_scalar(
            "SELECT i.description FROM issue i JOIN team t ON t.id = i.team_id WHERE t.key = 'ENG' AND i.number = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        sqlx::query_scalar("SELECT body FROM comment c JOIN issue i ON i.id = c.issue_id WHERE i.number = 1 AND c.parent_id IS NULL AND c.body LIKE 'Kommentar:%' LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT content FROM document WHERE title = 'Notes'")
            .fetch_one(&pool)
            .await
            .unwrap(),
    ];
    for t in &texts {
        for p in [
            "token",
            "uuid",
            "datei",
            "host-ipv4",
            "e-mail",
            "https",
            "script",
        ] {
            assert!(t.contains(&format!("\\<{p}>")), "{p} lost in {t:?}");
        }
        assert!(
            !t.replace("\\<", "").contains("<token>") && !t.replace("\\<", "").contains("<script>"),
            "no raw tags: {t:?}"
        );
        // Editor-Roundtrip ohne Verlust.
        let back = lineal_app::collab::pm::to_markdown(&lineal_app::collab::pm::parse_markdown(t));
        for p in [
            "token",
            "uuid",
            "datei",
            "host-ipv4",
            "e-mail",
            "https",
            "script",
        ] {
            assert!(
                back.contains(&format!("\\<{p}>")),
                "{p} lost in roundtrip {back:?}"
            );
        }
    }
    assert!(
        texts[0].contains("`<code-tag>`"),
        "inline code is untouched"
    );
    // Bericht: nichts „removed“, sondern erhalten; aktive Elemente als Warnung.
    let msgs: Vec<(String, String)> = sqlx::query_as(
        "SELECT severity, message FROM import_issue WHERE import_run_id = $1 AND message LIKE '%markup element%'",
    )
    .bind(run)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(!msgs.is_empty());
    assert!(msgs.iter().all(|(_, m)| !m.contains("was removed")));
    assert!(
        msgs.iter().any(|(sev, m)| m.contains("<script>")
            && sev == "warning"
            && m.contains("literal text"))
    );
    assert!(msgs.iter().any(|(sev, m)| m.contains("<token>")
        && sev == "info"
        && m.contains("kept as literal text")));
}

/// Blob-Store, der nur direkte Schreibzugriffe kennt (kein HTTP: die signierte URL ist unbrauchbar).
#[derive(Default)]
struct DirectBlobs {
    files: Mutex<HashMap<String, (String, usize)>>,
    reject_png: bool,
}

#[async_trait]
impl BlobStore for DirectBlobs {
    async fn put_bytes(&self, key: &str, content_type: &str, bytes: Vec<u8>) -> AppResult<()> {
        if self.reject_png && content_type == "image/png" {
            return Err(lineal_app::AppError::invalid_request(
                "The file content does not match the declared content type.",
            ));
        }
        self.files
            .lock()
            .unwrap()
            .insert(key.to_owned(), (content_type.to_owned(), bytes.len()));
        Ok(())
    }
    async fn signed_put_url(&self, _: &str, _: &str, _: Duration) -> AppResult<String> {
        Ok("http://unreachable.invalid:1/uploads/x".into())
    }
    async fn signed_get_url(&self, key: &str, _: Duration) -> AppResult<String> {
        Ok(format!("http://unreachable.invalid:1/files/{key}"))
    }
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn files_are_written_directly_through_the_blob_port(pool: PgPool) {
    let blobs = Arc::new(DirectBlobs::default());
    let e = env_with_blobs(pool.clone(), blobs.clone()).await;
    let source = fake();
    let sink = AppBlobSink { app: &e.app };
    let planned = e.scan(&source).await;
    e.start(planned.id, ImportStartInput::default()).await;
    let out = lineal_import::engine::ImportEngine::new(&e.app, &source, &sink)
        .with_config(lineal_import::engine::EngineConfig {
            page_size: 40,
            ..Default::default()
        })
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        blobs.files.lock().unwrap().len(),
        3,
        "two embedded images and the attached PDF"
    );
    let failed: i64 = e
        .count(&format!(
            "SELECT count(*) FROM import_issue WHERE import_run_id = '{}' AND message LIKE '%blob store failed%'",
            planned.id
        ))
        .await;
    assert_eq!(failed, 0);
    assert_eq!(
        e.count(
            "SELECT count(*) FROM attachment WHERE url LIKE '%/assets/%' OR storage_key IS NOT NULL"
        )
        .await
        .min(1),
        1
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn a_rejected_file_is_reported_not_fatal(pool: PgPool) {
    let blobs = Arc::new(DirectBlobs {
        reject_png: true,
        ..Default::default()
    });
    let e = env_with_blobs(pool.clone(), blobs.clone()).await;
    let source = fake();
    let sink = AppBlobSink { app: &e.app };
    let planned = e.scan(&source).await;
    e.start(planned.id, ImportStartInput::default()).await;
    let out = lineal_import::engine::ImportEngine::new(&e.app, &source, &sink)
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(blobs.files.lock().unwrap().len(), 1, "only the PDF");
    assert!(
        e.count(&format!(
            "SELECT count(*) FROM import_issue WHERE import_run_id = '{}' AND severity <> 'info' AND message LIKE '%content type%'",
            planned.id
        ))
        .await
            > 0
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn labels_with_the_same_name_are_mapped_not_duplicated(pool: PgPool) {
    let e = env(pool.clone()).await;
    // Vorhandene Workspace-Labels (andere Schreibweise) vor dem Import.
    let existing: Vec<(uuid::Uuid, &str)> = vec![
        (uuid::Uuid::now_v7(), "feature"),
        (uuid::Uuid::now_v7(), "BUG"),
    ];
    for (id, name) in &existing {
        sqlx::query(
            "INSERT INTO issue_label (id, organization_id, name, color) VALUES ($1, $2, $3, '#95A2B3')",
        )
        .bind(id)
        .bind(e.org.0)
        .bind(name)
        .execute(&pool)
        .await
        .unwrap();
    }
    let source = fake();
    let (run, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE name LIKE '% (Linear)'")
            .await,
        0
    );
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE lower(name) IN ('feature', 'bug')")
            .await,
        2,
        "no duplicates"
    );
    for (id, src) in [(existing[0].0, "l-feature"), (existing[1].0, "l-bug")] {
        let target: uuid::Uuid = sqlx::query_scalar(
            "SELECT target_id FROM import_mapping WHERE source_type = 'IssueLabel' AND source_id = $1",
        )
        .bind(src)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(target, id, "{src} maps to the existing label");
    }
    // Issues mit Linear-Label „Bug“ tragen jetzt das vorhandene Label.
    assert!(
        e.count(&format!(
            "SELECT count(*) FROM issue_to_label WHERE label_id = '{}'",
            existing[1].0
        ))
        .await
            > 0
    );
    assert!(
        e.count(&format!(
            "SELECT count(*) FROM import_issue WHERE import_run_id = '{run}' AND message LIKE 'mapped to the existing label%'"
        ))
        .await
            >= 2
    );
}

fn orphan_docs(source: &FakeSource) {
    let at = "2026-01-01T00:00:00Z";
    let mut docs = source.entity(SourceEntity::Documents);
    for (id, team, project) in [
        ("d-team-plt", Some("t-plt"), None),
        ("d-team-des", Some("t-des"), None),
        ("d-noteam", None, None),
        // Elternprojekt gibt es in der Quelle, es wird aber nicht importiert (unbekannt).
        ("d-gone-parent", Some("t-plt"), Some("p-private-unknown")),
    ] {
        docs.push(json!({ "id": id, "title": id, "content": "Orphan", "team": team.map(|t| json!({"id": t})),
            "project": project.map(|p| json!({"id": p})), "creator": { "id": "u-alice" }, "createdAt": at, "updatedAt": at }));
    }
    source.set_entity(SourceEntity::Documents, docs);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn orphaned_documents_only_go_to_their_own_imported_team(pool: PgPool) {
    let source = fake();
    orphan_docs(&source);
    let e = env(pool.clone()).await;
    let (run, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    let team_of = |title: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT t.key FROM document d LEFT JOIN team t ON t.id = d.team_id WHERE d.title = $1",
            )
            .bind(title)
            .fetch_optional(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(team_of("d-team-plt").await, Some(Some("PLT".into())));
    assert_eq!(team_of("d-team-des").await, Some(Some("DES".into())));
    // Kein eigenes Team, oder Elternobjekt nicht importiert: bleibt draußen (nie Workspace/anderes Team).
    assert_eq!(team_of("d-noteam").await, None);
    assert_eq!(team_of("d-gone-parent").await, None);
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM import_issue WHERE import_run_id = '{run}' AND severity = 'warning' AND source_type = 'Document'"
        ))
        .await,
        2
    );
    // Das Dokument des privaten Teams DES ist für Bob (kein Mitglied) unsichtbar.
    let mut tx = e.app.store.begin(e.org).await.unwrap();
    let snap = lineal_app::policy::PolicySnapshot::load(&mut *tx, &e.bob_ctx())
        .await
        .unwrap();
    let des: uuid::Uuid = sqlx::query_scalar("SELECT id FROM team WHERE key = 'DES'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!snap.visible_team_ids().iter().any(|t| t.0 == des));
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn orphaned_document_of_a_team_that_is_not_imported_stays_out(pool: PgPool) {
    let source = fake();
    orphan_docs(&source);
    let e = env(pool.clone()).await;
    let planned = e.scan(&source).await;
    e.start(
        planned.id,
        ImportStartInput {
            teams: vec!["ENG".into()],
            ..Default::default()
        },
    )
    .await;
    let out = e
        .engine(&source, Default::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        e.count("SELECT count(*) FROM document WHERE title LIKE 'd-%'")
            .await,
        0,
        "no orphan of a team outside the selection is imported"
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn replies_before_their_parent_are_linked_afterwards(pool: PgPool) {
    let source = fake();
    let mut comments = source.entity(SourceEntity::Comments);
    comments.reverse(); // Antworten zuerst, wie bei „neueste zuerst“
    let first = comments[0]["id"].as_str().unwrap().to_owned();
    assert!(
        first.ends_with("-r"),
        "fixture: replies come first: {first}"
    );
    source.set_entity(SourceEntity::Comments, comments);
    // Eine Antwort auf einen Kommentar, den es nicht gibt (gelöscht): bleibt oberste Ebene.
    source.patch(
        SourceEntity::Comments,
        "c-ENG-2-r",
        &json!({ "parent": { "id": "c-deleted" } }),
    );
    let e = env(pool.clone()).await;
    let (run, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    let replies = e
        .count("SELECT count(*) FROM comment WHERE parent_id IS NOT NULL")
        .await;
    let expected = source
        .entity(SourceEntity::Comments)
        .iter()
        .filter(|c| c["parent"]["id"].as_str().is_some_and(|p| p != "c-deleted"))
        .count() as i64;
    assert!(expected > 5);
    assert_eq!(
        replies, expected,
        "all replies with a known parent are linked"
    );
    let warnings: Vec<String> = sqlx::query_scalar(
        "SELECT message FROM import_issue WHERE import_run_id = $1 AND source_type = 'Comment' AND severity = 'warning'",
    )
    .bind(run)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("not part of the import"));
    let _: &dyn ImportSource = &source;
}
