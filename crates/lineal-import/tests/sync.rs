#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Regelmäßiger Abgleich Linear -> Lineal: Einrichten, Rechte, Delta mit Konfliktregel, Archivieren
//! gelöschter Objekte, „Inhalte neu laden“, Label-Aufräumen, Fehlerbenachrichtigung, Planer
//! (Fake-Quelle; nie das echte Linear).

mod common;

use chrono::Duration;
use common::*;
use lineal_app::ErrorKind;
use lineal_app::usecases::import::ImportStartInput;
use lineal_app::usecases::import_sync::{ImportSyncInput, SyncRunOptions};
use lineal_import::engine::{EngineConfig, RunOutcome};
use lineal_import::{FakeSource, SourceEntity, SourceError};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const T_LATER: &str = "2026-10-10T08:40:00Z";
const T_LOCAL: &str = "2026-10-10T08:20:00Z";

async fn imported(pool: PgPool, source: &FakeSource) -> Env {
    let e = env(pool).await;
    let (_, out) = e.import_all(source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    e
}

fn input() -> ImportSyncInput {
    ImportSyncInput {
        interval: "hourly".into(),
        token: Some(TOKEN.into()),
        ..Default::default()
    }
}

async fn run_sync(
    e: &Env,
    source: &FakeSource,
    opts: SyncRunOptions,
) -> (lineal_app::ports::repos::import::ImportRun, RunOutcome) {
    e.clock.set(t0() + Duration::hours(1));
    let run = e
        .app
        .import_sync_run_now(&e.alice_ctx(), opts)
        .await
        .unwrap();
    assert_eq!(run.status, "running");
    let out = e
        .engine(
            source,
            EngineConfig {
                page_size: 40,
                ..Default::default()
            },
        )
        .run(e.org, run.id)
        .await
        .unwrap();
    (run, out)
}

async fn title_of(e: &Env, ident: (&str, i32)) -> String {
    sqlx::query_scalar("SELECT i.title FROM issue i JOIN team t ON t.id = i.team_id WHERE t.key = $1 AND i.number = $2")
        .bind(ident.0)
        .bind(ident.1)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn only_owner_or_admin_can_manage_the_sync_and_the_key_never_leaks(pool: PgPool) {
    let source = fake();
    let e = imported(pool.clone(), &source).await;
    let bob = e.bob_ctx();
    assert_eq!(
        e.app.import_sync_get(&bob).await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        e.app
            .import_sync_save(&bob, input())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert!(
        e.app
            .import_sync_run_now(&bob, SyncRunOptions::default())
            .await
            .is_err()
    );
    assert!(e.app.import_sync_set_paused(&bob, true).await.is_err());
    assert!(e.app.import_sync_delete(&bob).await.is_err());
    // Ohne Schlüssel kein Einrichten.
    let err = e
        .app
        .import_sync_save(
            &e.alice_ctx(),
            ImportSyncInput {
                token: None,
                ..input()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    let row = e
        .app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    assert_eq!(row.interval, "hourly");
    assert!(row.next_run_at.is_some());
    // Schlüssel: nur verschlüsselt in der DB, nie in Debug-Ausgaben, Audit-Log und Berichten.
    let blob: Vec<u8> = sqlx::query_scalar("SELECT encrypted_token FROM import_sync_credential")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!blob.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()));
    assert!(!format!("{row:?}").contains(TOKEN));
    let audit: Vec<String> =
        sqlx::query_scalar("SELECT details::text FROM audit_log WHERE action LIKE 'import.sync.%'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!audit.is_empty() && audit.iter().all(|d| !d.contains("FAKETOKEN")));
    // Ersetzen des Schlüssels ist ein eigener, protokollierter Vorgang.
    e.app
        .import_sync_save(
            &e.alice_ctx(),
            ImportSyncInput {
                token: Some("lin_api_SECONDKEYabcdef0123456789".into()),
                ..input()
            },
        )
        .await
        .unwrap();
    let audit: Vec<String> = sqlx::query_scalar(
        "SELECT action || details::text FROM audit_log WHERE action LIKE 'import.sync.%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        audit
            .iter()
            .any(|d| d.starts_with("import.sync.updated") && d.contains("\"tokenReplaced\": true"))
    );
    assert!(audit.iter().all(|d| !d.contains("SECONDKEY")));
    // Pause/Fortsetzen/Löschen mit Audit.
    let paused = e
        .app
        .import_sync_set_paused(&e.alice_ctx(), true)
        .await
        .unwrap();
    assert!(paused.paused && paused.next_run_at.is_none());
    let resumed = e
        .app
        .import_sync_set_paused(&e.alice_ctx(), false)
        .await
        .unwrap();
    assert!(!resumed.paused && resumed.next_run_at.is_some());
    e.app.import_sync_delete(&e.alice_ctx()).await.unwrap();
    assert!(
        e.app
            .import_sync_get(&e.alice_ctx())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        e.count("SELECT count(*) FROM import_sync_credential").await,
        0,
        "the stored key is deleted with the sync"
    );
    for action in [
        "import.sync.created",
        "import.sync.updated",
        "import.sync.paused",
        "import.sync.resumed",
        "import.sync.deleted",
    ] {
        assert_eq!(
            e.count(&format!(
                "SELECT count(*) FROM audit_log WHERE action = '{action}'"
            ))
            .await,
            1,
            "{action}"
        );
    }
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn sync_needs_a_completed_import(pool: PgPool) {
    let e = env(pool).await;
    let err = e
        .app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn delta_creates_updates_and_keeps_locally_changed_objects(pool: PgPool) {
    let source = fake();
    let e = imported(pool.clone(), &source).await;
    e.app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    let issues_before = e.count("SELECT count(*) FROM issue").await;
    // Linear: ENG-1 und ENG-2 geändert, ein neues Issue ENG-200.
    source.patch(
        SourceEntity::Issues,
        "i-ENG-1",
        &json!({ "title": "Linear new title 1", "updatedAt": T_LATER }),
    );
    source.patch(
        SourceEntity::Issues,
        "i-ENG-2",
        &json!({ "title": "Linear new title 2", "updatedAt": T_LATER }),
    );
    let mut issues = source.entity(SourceEntity::Issues);
    let mut fresh = issues
        .iter()
        .find(|i| i["id"] == "i-ENG-4")
        .cloned()
        .unwrap();
    fresh["id"] = json!("i-ENG-200");
    fresh["number"] = json!(200);
    fresh["identifier"] = json!("ENG-200");
    fresh["title"] = json!("Brand new in Linear");
    fresh["updatedAt"] = json!(T_LATER);
    fresh["createdAt"] = json!(T_LATER);
    fresh["parent"] = serde_json::Value::Null;
    fresh["assignee"] = serde_json::Value::Null;
    issues.push(fresh);
    source.set_entity(SourceEntity::Issues, issues);
    // Lineal: ENG-2 wurde nach dem Import lokal geändert.
    sqlx::query("UPDATE issue SET title = 'Local edit', updated_at = $1::timestamptz WHERE number = 2 AND team_id = (SELECT id FROM team WHERE key = 'ENG')")
        .bind(T_LOCAL)
        .execute(&pool)
        .await
        .unwrap();

    let (run, out) = run_sync(&e, &source, SyncRunOptions::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        title_of(&e, ("ENG", 1)).await,
        "Linear new title 1",
        "unchanged locally: Linear wins"
    );
    assert_eq!(
        title_of(&e, ("ENG", 2)).await,
        "Local edit",
        "changed locally: the local value stays"
    );
    assert_eq!(title_of(&e, ("ENG", 200)).await, "Brand new in Linear");
    assert_eq!(
        e.count("SELECT count(*) FROM issue").await,
        issues_before + 1
    );
    // Konflikt im Bericht, Zähler stimmen.
    let conflicts: i64 = e
        .count(&format!(
            "SELECT count(*) FROM import_issue WHERE import_run_id = '{}' AND severity = 'warning' AND message LIKE '%changed in Lineal since the import%'",
            run.id
        ))
        .await;
    assert!(conflicts >= 1);
    let stored = e.app.import_get_run(&e.alice_ctx(), run.id).await.unwrap();
    assert_eq!(stored.stats["entities"]["Issue"]["created"], 1);
    assert_eq!(stored.stats["entities"]["Issue"]["updated"], 1);
    assert_eq!(stored.stats["entities"]["Issue"]["conflicts"], 1);
    // Sync-Status: letzter Lauf erfolgreich.
    let sync = e
        .app
        .import_sync_get(&e.alice_ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sync.last_status.as_deref(), Some("completed"));
    assert_eq!(sync.last_run_id, Some(run.id));
    assert!(sync.last_success_at.is_some());
    // Ein weiterer Lauf ohne Änderungen ändert nichts.
    let (run2, out) = run_sync(&e, &source, SyncRunOptions::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    let again = e.app.import_get_run(&e.alice_ctx(), run2.id).await.unwrap();
    let n = |k: &str| again.stats["entities"]["Issue"][k].as_u64().unwrap_or(0);
    assert_eq!((n("updated"), n("created")), (0, 0));
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn objects_removed_in_linear_are_archived_unless_changed_locally(pool: PgPool) {
    let source = fake();
    let e = imported(pool.clone(), &source).await;
    e.app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    let mut issues = source.entity(SourceEntity::Issues);
    issues.retain(|i| i["id"] != "i-ENG-3" && i["id"] != "i-ENG-4");
    source.set_entity(SourceEntity::Issues, issues);
    let mut docs = source.entity(SourceEntity::Documents);
    docs.retain(|d| d["id"] != "d-2");
    source.set_entity(SourceEntity::Documents, docs);
    sqlx::query("UPDATE issue SET title = 'Local keep', updated_at = $1::timestamptz WHERE number = 4 AND team_id = (SELECT id FROM team WHERE key = 'ENG')")
        .bind(T_LOCAL)
        .execute(&pool)
        .await
        .unwrap();
    let (run, out) = run_sync(&e, &source, SyncRunOptions::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    let archived = |n: i32| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>("SELECT i.archived_at IS NOT NULL FROM issue i JOIN team t ON t.id = i.team_id WHERE t.key = 'ENG' AND i.number = $1")
                .bind(n)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert!(
        archived(3).await,
        "removed in Linear: archived, not deleted"
    );
    assert!(!archived(4).await, "locally changed: kept");
    assert!(!archived(5).await, "still in Linear");
    assert_eq!(
        e.count("SELECT count(*) FROM document WHERE title = 'Initiative brief' AND archived_at IS NOT NULL")
            .await,
        1
    );
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM import_issue WHERE import_run_id = '{}' AND message LIKE 'archived in Lineal%'",
            run.id
        ))
        .await,
        2
    );
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM import_issue WHERE import_run_id = '{}' AND message LIKE 'removed or archived in Linear, but changed%'",
            run.id
        ))
        .await,
        1
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn removal_check_is_skipped_when_the_source_reports_partial_errors(pool: PgPool) {
    let source = fake();
    let e = imported(pool.clone(), &source).await;
    e.app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    let mut issues = source.entity(SourceEntity::Issues);
    issues.retain(|i| i["id"] != "i-ENG-3");
    source.set_entity(SourceEntity::Issues, issues);
    // Mitten im Lauf bricht die Quelle ab: nichts darf archiviert werden.
    let archived_before = e
        .count("SELECT count(*) FROM issue WHERE archived_at IS NOT NULL")
        .await;
    source.fail_after(
        SourceEntity::Issues,
        1,
        SourceError::Transport("boom".into()),
    );
    let (_, out) = run_sync(&e, &source, SyncRunOptions::default()).await;
    assert!(matches!(out, RunOutcome::Failed(_)), "{out:?}");
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE archived_at IS NOT NULL")
            .await,
        archived_before
    );
}

fn strip_tags_like_before(text: &str) -> String {
    text.replace("<token>", "").replace("<uuid>", "")
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn reload_content_restores_lost_text_but_never_overwrites_local_or_foreign_objects(
    pool: PgPool,
) {
    let source = fake();
    let original = "Setze <token> und <uuid> ein.";
    source.patch(
        SourceEntity::Issues,
        "i-ENG-6",
        &json!({ "description": original }),
    );
    source.patch(
        SourceEntity::Issues,
        "i-ENG-11",
        &json!({ "description": original }),
    );
    source.patch(
        SourceEntity::Issues,
        "i-ENG-12",
        &json!({ "description": original }),
    );
    let e = imported(pool.clone(), &source).await;
    // Zweiter Workspace mit eigenem Issue (gleiche Quell-IDs sind dort unerheblich).
    let other = env_n(pool.clone(), 2).await;
    let foreign_team = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO team (id, organization_id, key, name) VALUES ($1, $2, 'FOR', 'Foreign')",
    )
    .bind(foreign_team)
    .bind(other.org.0)
    .execute(&pool)
    .await
    .unwrap();
    let foreign_state = Uuid::now_v7();
    sqlx::query("INSERT INTO workflow_state (id, organization_id, team_id, name, type, color, position) VALUES ($1, $2, $3, 'Todo', 'unstarted', '#95A2B3', 1)")
        .bind(foreign_state)
        .bind(other.org.0)
        .bind(foreign_team)
        .execute(&pool)
        .await
        .unwrap();
    let foreign_issue = Uuid::now_v7();
    sqlx::query("INSERT INTO issue (id, organization_id, team_id, number, title, description, state_id) VALUES ($1, $2, $3, 1, 'Foreign', 'foreign text', $4)")
        .bind(foreign_issue)
        .bind(other.org.0)
        .bind(foreign_team)
        .bind(foreign_state)
        .execute(&pool)
        .await
        .unwrap();
    // Zustand „wie nach dem fehlerhaften Lauf“: Platzhalter gelöscht.
    let lost = strip_tags_like_before(original);
    for n in [6, 11, 12] {
        sqlx::query("UPDATE issue SET description = $1 WHERE number = $2 AND team_id = (SELECT id FROM team WHERE key = 'ENG' AND organization_id = $3)")
            .bind(&lost)
            .bind(n)
            .bind(e.org.0)
            .execute(&pool)
            .await
            .unwrap();
    }
    // ENG-11: lokal geändert; ENG-12: Mapping zeigt (manipuliert) auf ein Objekt eines anderen Workspace.
    sqlx::query("UPDATE issue SET description = 'my own text', updated_at = $1::timestamptz WHERE number = 11 AND team_id = (SELECT id FROM team WHERE key = 'ENG' AND organization_id = $2)")
        .bind(T_LOCAL)
        .bind(e.org.0)
        .execute(&pool)
        .await
        .unwrap();
    let foreign_text_before: Option<String> =
        sqlx::query_scalar("SELECT description FROM issue WHERE id = $1")
            .bind(foreign_issue)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE import_mapping SET target_id = $1 WHERE organization_id = $2 AND source_type = 'Issue' AND source_id = 'i-ENG-12'")
        .bind(foreign_issue)
        .bind(e.org.0)
        .execute(&pool)
        .await
        .unwrap();
    e.app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    let (_, out) = run_sync(
        &e,
        &source,
        SyncRunOptions {
            reload_content: true,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(out, RunOutcome::Completed);
    let desc = |n: i32| {
        let pool = pool.clone();
        let org = e.org.0;
        async move {
            sqlx::query_scalar::<_, Option<String>>("SELECT i.description FROM issue i JOIN team t ON t.id = i.team_id WHERE t.key = 'ENG' AND t.organization_id = $2 AND i.number = $1")
                .bind(n)
                .bind(org)
                .fetch_one(&pool)
                .await
                .unwrap()
                .unwrap_or_default()
        }
    };
    let restored = desc(6).await;
    assert!(
        restored.contains("\\<token>") && restored.contains("\\<uuid>"),
        "{restored}"
    );
    assert_eq!(desc(11).await, "my own text", "locally changed: kept");
    {
        let after: Option<String> =
            sqlx::query_scalar("SELECT description FROM issue WHERE id = $1")
                .bind(foreign_issue)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            after, foreign_text_before,
            "objects of other workspaces are never touched"
        );
    }
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM issue WHERE organization_id = '{}' AND description LIKE '%\\<token>%'",
            other.org.0
        ))
        .await,
        0
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn runs_never_overlap_for_the_same_teams(pool: PgPool) {
    let source = fake();
    let e = imported(pool.clone(), &source).await;
    e.app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    let first = e
        .app
        .import_sync_run_now(&e.alice_ctx(), SyncRunOptions::default())
        .await
        .unwrap();
    assert_eq!(first.status, "running");
    let err = e
        .app
        .import_sync_run_now(&e.alice_ctx(), SyncRunOptions::default())
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Conflict);
    // Der Planer überspringt, solange der Lauf aktiv ist.
    e.clock.set(t0() + Duration::hours(3));
    assert_eq!(e.app.import_sync_tick(e.clock_now()).await.unwrap(), 0);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn scheduler_starts_due_syncs_with_the_stored_key_and_respects_pause(pool: PgPool) {
    let source = fake();
    let e = imported(pool.clone(), &source).await;
    e.clock.set(t0() + Duration::minutes(10));
    e.app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    // Noch nicht fällig.
    e.clock.set(t0() + Duration::minutes(30));
    assert_eq!(e.app.import_sync_tick(e.clock_now()).await.unwrap(), 0);
    // Pausiert: nie.
    e.app
        .import_sync_set_paused(&e.alice_ctx(), true)
        .await
        .unwrap();
    e.clock.set(t0() + Duration::hours(5));
    assert_eq!(e.app.import_sync_tick(e.clock_now()).await.unwrap(), 0);
    e.app
        .import_sync_set_paused(&e.alice_ctx(), false)
        .await
        .unwrap();
    // Fällig: ein Lauf mit Delta, Einrichtende als Starter, Schlüssel für den Lauf neu verschlüsselt.
    e.clock
        .set(t0() + Duration::hours(5) + Duration::minutes(30));
    assert_eq!(
        e.app.import_sync_tick(e.clock_now()).await.unwrap(),
        0,
        "next slot is one hour after resuming"
    );
    e.clock.set(t0() + Duration::hours(7));
    assert_eq!(e.app.import_sync_tick(e.clock_now()).await.unwrap(), 1);
    let runs = e.app.import_list_runs(&e.alice_ctx(), 10).await.unwrap();
    let run = runs
        .iter()
        .find(|r| r.options["syncId"].is_string())
        .expect("sync run");
    assert_eq!(run.status, "running");
    assert_eq!(run.options["delta"], true);
    assert_eq!(run.started_by, Some(e.alice.0));
    let token = e
        .app
        .import_load_token((e.org, run.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(token.expose(), TOKEN);
    // Direkt danach: aktiver Lauf, nächster Termin liegt in der Zukunft.
    assert_eq!(e.app.import_sync_tick(e.clock_now()).await.unwrap(), 0);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn failed_run_is_recorded_and_notifies_the_person_who_set_it_up(pool: PgPool) {
    let source = fake();
    let e = imported(pool.clone(), &source).await;
    e.app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    source.fail_with(Some(SourceError::Auth));
    let (run, out) = run_sync(&e, &source, SyncRunOptions::default()).await;
    assert!(matches!(out, RunOutcome::Failed(_)), "{out:?}");
    let sync = e
        .app
        .import_sync_get(&e.alice_ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sync.last_status.as_deref(), Some("failed"));
    assert!(sync.last_error.is_some());
    assert!(!sync.last_error.unwrap().contains("FAKETOKEN"));
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM notification WHERE user_id = '{}' AND type = 'importSyncFailed'",
            e.alice.0
        ))
        .await,
        1
    );
    let _ = run;
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn duplicate_linear_labels_are_merged_only_with_the_explicit_option(pool: PgPool) {
    let source = fake();
    let e = imported(pool.clone(), &source).await;
    // Zustand nach dem früheren Lauf: das importierte Label heißt „Feature (Linear)“, daneben
    // gibt es ein eigenes Workspace-Label „Feature“.
    let imported_label: Uuid = sqlx::query_scalar(
        "SELECT target_id FROM import_mapping WHERE source_type = 'IssueLabel' AND source_id = 'l-feature'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE issue_label SET name = 'Feature (Linear)' WHERE id = $1")
        .bind(imported_label)
        .execute(&pool)
        .await
        .unwrap();
    let own = Uuid::now_v7();
    sqlx::query("INSERT INTO issue_label (id, organization_id, name, color) VALUES ($1, $2, 'Feature', '#95A2B3')")
        .bind(own)
        .bind(e.org.0)
        .execute(&pool)
        .await
        .unwrap();
    let carriers = e
        .count(&format!(
            "SELECT count(*) FROM issue_to_label WHERE label_id = '{imported_label}'"
        ))
        .await;
    assert!(carriers > 0);
    e.app
        .import_sync_save(&e.alice_ctx(), input())
        .await
        .unwrap();
    // Ohne Option bleibt alles.
    let (_, out) = run_sync(&e, &source, SyncRunOptions::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE name = 'Feature (Linear)'")
            .await,
        1
    );
    // Mit Option: zusammengeführt, Mapping zeigt auf das gleichnamige Label.
    let (_, out) = run_sync(
        &e,
        &source,
        SyncRunOptions {
            merge_linear_labels: true,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE name = 'Feature (Linear)'")
            .await,
        0
    );
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM issue_to_label WHERE label_id = '{own}'"
        ))
        .await,
        carriers
    );
    let mapped: Uuid = sqlx::query_scalar(
        "SELECT target_id FROM import_mapping WHERE source_type = 'IssueLabel' AND source_id = 'l-feature'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(mapped, own);
}
