#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP18: Abnahmekriterien 1–6 aus docs/13 §9 gegen die Fake-Quelle (kein Zugriff auf das echte Linear).

mod common;

use common::*;
use lineal_app::ErrorKind;
use lineal_app::ports::repos::import::ImportRun;
use lineal_app::usecases::import::ImportStartInput;
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_import::engine::{EngineConfig, RunOutcome};
use lineal_import::port::SourceError;
use lineal_import::{FakeSource, SourceEntity};
use serde_json::json;
use sqlx::PgPool;
use std::collections::BTreeMap;

async fn completed_import(e: &Env) -> (FakeSource, uuid::Uuid) {
    let source = fake();
    let (run, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    (source, run)
}

async fn run_of(e: &Env, id: uuid::Uuid) -> ImportRun {
    e.app.import_get_run(&e.alice_ctx(), id).await.unwrap()
}

// ---------------------------------------------------------------------------------------------
// Kriterium 1: vollständige Übernahme, Zählungen stimmen
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn criterion_1_full_import_with_matching_counts(pool: PgPool) {
    let e = env(pool).await;
    let (source, run_id) = completed_import(&e).await;
    let run = run_of(&e, run_id).await;
    assert_eq!(run.status, "completed", "error: {:?}", run.error);
    assert!(run.finished_at.is_some());

    // Zählungen je Entität: Zeilen in Lineal == Objekte der Quelle (Ausnahmen unten).
    let n = |ent: SourceEntity| source.entity(ent).len() as i64;
    assert_eq!(
        e.count("SELECT count(*) FROM team").await,
        n(SourceEntity::Teams)
    );
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 200);
    assert_eq!(
        e.count("SELECT count(*) FROM issue").await,
        n(SourceEntity::Issues)
    );
    assert_eq!(
        e.count("SELECT count(*) FROM workflow_state").await,
        n(SourceEntity::WorkflowStates)
    );
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label").await,
        n(SourceEntity::IssueLabels)
    );
    assert_eq!(e.count("SELECT count(*) FROM cycle").await, 2);
    assert_eq!(e.count("SELECT count(*) FROM project").await, 3);
    assert_eq!(e.count("SELECT count(*) FROM project_milestone").await, 3);
    assert_eq!(e.count("SELECT count(*) FROM initiative").await, 2);
    assert_eq!(
        e.count("SELECT count(*) FROM initiative_parent").await,
        1,
        "DAG edge from parentInitiative"
    );
    assert_eq!(
        e.count("SELECT count(*) FROM initiative_to_project").await,
        3
    );
    assert_eq!(
        e.count("SELECT count(*) FROM comment").await,
        n(SourceEntity::Comments)
    );
    assert_eq!(
        e.count("SELECT count(*) FROM comment WHERE parent_id IS NOT NULL")
            .await,
        28,
        "replies keep their thread"
    );
    assert_eq!(e.count("SELECT count(*) FROM reaction").await, 28);
    assert_eq!(e.count("SELECT count(*) FROM document").await, 3);
    assert_eq!(e.count("SELECT count(*) FROM document_content").await, 3);
    assert_eq!(e.count("SELECT count(*) FROM project_update").await, 3);
    assert_eq!(e.count("SELECT count(*) FROM initiative_update").await, 1);
    assert_eq!(e.count("SELECT count(*) FROM template").await, 2);
    assert_eq!(
        e.count("SELECT count(*) FROM issue_subscription").await,
        n(SourceEntity::IssueSubscriptions)
    );
    // Relationen: das Gegenrichtungs-Paar r-4 wird übersprungen.
    assert_eq!(
        e.count("SELECT count(*) FROM issue_relation").await,
        n(SourceEntity::IssueRelations) - 1
    );
    // Historie: nur Einträge mit Statuswechsel/Assignee/Priorität (alle Fixtures).
    assert_eq!(
        e.count("SELECT count(*) FROM issue_history WHERE source = 'import'")
            .await,
        n(SourceEntity::IssueHistory)
    );
    // Anhänge: 14 Links + 2 Dateien + 2 eingebettete Uploads.
    assert_eq!(e.count("SELECT count(*) FROM attachment").await, 18);
    assert_eq!(
        e.count("SELECT count(*) FROM attachment WHERE storage_key IS NOT NULL")
            .await,
        3
    );
    assert_eq!(e.sink.len(), 3, "three files were stored in the blob store");

    // Teams: Sub-Team, privates Team, Zeitzone, Zyklen-Einstellungen.
    let web_parent: Option<String> = sqlx::query_scalar(
        "SELECT p.key FROM team w JOIN team p ON p.id = w.parent_id WHERE w.key = 'WEB'",
    )
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(web_parent.as_deref(), Some("ENG"));
    assert_eq!(
        e.count("SELECT count(*) FROM team WHERE key = 'DES' AND visibility = 'private'")
            .await,
        1
    );
    assert_eq!(
        e.count("SELECT count(*) FROM team WHERE key = 'ENG' AND cycles_enabled AND issue_estimation_type = 'fibonacci' AND triage_enabled AND timezone = 'Europe/Berlin'").await,
        1
    );
    // Default-/Triage-Status wurden aufgelöst (Zirkelreferenz Team/Status).
    assert_eq!(
        e.count("SELECT count(*) FROM team WHERE default_issue_state_id IS NULL")
            .await,
        0
    );
    assert_eq!(
        e.count(
            "SELECT count(*) FROM team WHERE key = 'ENG' AND triage_issue_state_id IS NOT NULL"
        )
        .await,
        1
    );
    // Duplicate-Status wurde dem Typ `duplicate` zugeordnet (nicht doppelt angelegt).
    assert_eq!(
        e.count("SELECT count(*) FROM workflow_state WHERE type = 'duplicate'")
            .await,
        4
    );

    // User: Alice/Bob per E-Mail (case-insensitiv) zugeordnet, Carol/Dave Platzhalter ohne Login.
    assert_eq!(
        e.count("SELECT count(*) FROM \"user\"").await,
        4,
        "no duplicate users for matched e-mails"
    );
    assert_eq!(
        e.count("SELECT count(*) FROM \"user\" WHERE status = 'imported' AND NOT active AND password_hash IS NULL AND email IN ('carol@acme.test', 'dave@acme.test')").await,
        2
    );
    assert_eq!(
        e.count("SELECT count(*) FROM \"user\" WHERE status = 'imported' AND role = 'guest'")
            .await,
        1
    );
    assert_eq!(e.count("SELECT count(*) FROM team_membership").await, 8);

    // Labels: Gruppen mit groupType, single-select-Zuordnung am Issue.
    assert_eq!(
        e.count("SELECT count(*) FROM issue_label WHERE is_group AND group_type = 'singleSelect'")
            .await,
        1
    );
    assert!(
        e.count("SELECT count(*) FROM issue_to_label WHERE single_select_group_id IS NOT NULL")
            .await
            > 0
    );

    // Projekte: Lead-Team, Teams, Nummern (P-ENG-1, P-ENG-2, P-DES-1).
    assert_eq!(e.count("SELECT count(*) FROM project_team").await, 5);
    let numbers: Vec<(String, i32)> = sqlx::query_as(
        "SELECT t.key, p.number FROM project p JOIN team t ON t.id = p.lead_team_id ORDER BY t.key, p.number",
    )
    .fetch_all(&e.pool)
    .await
    .unwrap();
    assert_eq!(
        numbers,
        vec![("DES".into(), 1), ("ENG".into(), 1), ("ENG".into(), 2)]
    );
    assert_eq!(
        e.count("SELECT count(*) FROM project WHERE slug_id = '0123456789ab'")
            .await,
        1
    );

    // Issues: Zeitstempel, Autor, State, Labels, Cycle, Projekt, Meilenstein bleiben erhalten.
    let (created, creator, state_type): (chrono::DateTime<chrono::Utc>, Option<uuid::Uuid>, String) = sqlx::query_as(
        "SELECT i.created_at, i.creator_id, i.state_type FROM issue i JOIN team t ON t.id = i.team_id WHERE t.key = 'ENG' AND i.number = 12",
    )
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(
        created,
        lineal_import::fixtures::at(50)
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap(),
        "createdAt is kept"
    );
    assert_eq!(creator, Some(e.alice.0));
    assert_eq!(state_type, "backlog");
    assert!(
        e.count("SELECT count(*) FROM issue WHERE cycle_id IS NOT NULL")
            .await
            >= 20
    );
    assert!(e.count("SELECT count(*) FROM issue WHERE project_id IS NOT NULL AND project_milestone_id IS NOT NULL").await > 5);
    assert_eq!(e.count("SELECT count(*) FROM issue WHERE trashed").await, 1);
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE archived_at IS NOT NULL")
            .await,
        1
    );
    // SLA-Felder werden übernommen.
    assert_eq!(
        e.count(
            "SELECT count(*) FROM issue WHERE sla_type = 'all' AND sla_breaches_at IS NOT NULL"
        )
        .await,
        1
    );

    // Sub-Issues inkl. Eltern mit höherer Nummer und teamübergreifend (Phase 4b).
    let parent_of = |key: &'static str, n: i32| {
        let pool = e.pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT p.identifier FROM issue i JOIN team t ON t.id = i.team_id JOIN issue p ON p.id = i.parent_id WHERE t.key = $1 AND i.number = $2",
            )
            .bind(key)
            .bind(n)
            .fetch_optional(&pool)
            .await
            .unwrap()
            .flatten()
        }
    };
    assert_eq!(
        parent_of("ENG", 3).await.as_deref(),
        Some("ENG-12"),
        "parent with a higher number"
    );
    assert_eq!(
        parent_of("WEB", 2).await.as_deref(),
        Some("ENG-10"),
        "cross-team parent"
    );
    assert_eq!(parent_of("PLT", 4).await.as_deref(), Some("PLT-3"));
    assert_eq!(
        e.count("SELECT count(*) FROM import_mapping WHERE source_type = 'IssueParentLink'")
            .await,
        0
    );

    // Markdown: Linear-Elemente umgeschrieben, aktive Inhalte entfernt, Erwähnungen aufgelöst.
    let desc = |key: &'static str, n: i32| {
        let pool = e.pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT i.description FROM issue i JOIN team t ON t.id = i.team_id WHERE t.key = $1 AND i.number = $2",
            )
            .bind(key)
            .bind(n)
            .fetch_one(&pool)
            .await
            .unwrap()
            .unwrap_or_default()
        }
    };
    let d10 = desc("ENG", 10).await;
    assert!(d10.contains("ENG-12") && !d10.contains("<issue"), "{d10}");
    assert!(
        d10.contains("http://localhost:8080/acme/issue/WEB-3"),
        "{d10}"
    );
    let d5 = desc("ENG", 5).await;
    // Kein ausführbares Markup, aber der Text bleibt sichtbar (`\<script>`).
    assert!(
        !d5.contains(" <script")
            && d5.contains("\\<script>alert(1)")
            && !d5.contains("](javascript:"),
        "{d5}"
    );
    assert!(d5.contains("\\<linear-image>"), "{d5}");
    assert!(
        desc("ENG", 9).await.contains("@bob"),
        "mention rewritten to the Lineal display name"
    );
    // Eingebettete Uploads: heruntergeladen, URL umgeschrieben, als Anhang geführt.
    let d8 = desc("ENG", 8).await;
    assert!(
        !d8.contains("uploads.linear.app") && d8.contains("/assets/"),
        "{d8}"
    );
    // Kommentare: Upload im Kommentar umgeschrieben, aktive Inhalte entfernt.
    let c1: String =
        sqlx::query_scalar("SELECT body FROM comment WHERE quoted_text = 'quoted part'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(
        c1.contains("\\<img src=x onerror=alert(1)>") && !c1.contains("uploads.linear.app"),
        "{c1}"
    );

    // Frühere Kennungen bleiben als Alias auflösbar.
    assert_eq!(
        e.count("SELECT count(*) FROM issue_identifier_alias a JOIN issue i ON i.id = a.issue_id WHERE a.identifier = 'OLD-4' AND i.identifier = 'ENG-123'").await,
        1
    );

    // Bericht und Statistik.
    let (entries, counts) = e
        .app
        .import_report(&e.alice_ctx(), run_id, None, 500, 0)
        .await
        .unwrap();
    assert!(!entries.is_empty());
    let by: BTreeMap<_, _> = counts.into_iter().collect();
    assert_eq!(
        by.get("error").copied().unwrap_or(0),
        0,
        "no errors expected: {entries:#?}"
    );
    assert!(
        by.get("warning").copied().unwrap_or(0) >= 1,
        "the failed download is reported"
    );
    assert!(
        entries.iter().any(|r| r.message.contains("linear-image")),
        "unknown elements are listed"
    );
    assert_eq!(run.stats["entities"]["Issue"]["created"], 200);
    assert_eq!(run.stats["verification"]["Issue"]["present"], 200);
    assert_eq!(run.stats["scan"]["counts"]["Issue"], 200);
}

// ---------------------------------------------------------------------------------------------
// Kriterium 2: Kennungen bleiben gleich, neues Issue = max + 1
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn criterion_2_identifiers_and_counters(pool: PgPool) {
    let e = env(pool).await;
    completed_import(&e).await;
    let (id, title): (String, String) =
        sqlx::query_as("SELECT identifier, title FROM issue WHERE identifier = 'ENG-123'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(
        (id.as_str(), title.as_str()),
        ("ENG-123", "The famous ENG-123")
    );
    // Lücken bleiben Lücken.
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE identifier = 'ENG-124'")
            .await,
        0
    );
    // Zähler = max(number); Projekte/Cycles/Initiativen ebenfalls.
    assert_eq!(
        e.count("SELECT last_issue_number FROM team WHERE key = 'ENG'")
            .await,
        130
    );
    assert_eq!(
        e.count("SELECT last_issue_number FROM team WHERE key = 'WEB'")
            .await,
        40
    );
    assert_eq!(
        e.count("SELECT last_cycle_number FROM team WHERE key = 'ENG'")
            .await,
        2
    );
    assert_eq!(
        e.count("SELECT last_project_number FROM team WHERE key = 'ENG'")
            .await,
        2
    );
    assert_eq!(
        e.count("SELECT last_initiative_number FROM organization")
            .await,
        2
    );

    // Ein neues Issue über den normalen Use-Case erhält max + 1 (Alice ist ENG-Mitglied).
    let ctx = e.alice_ctx();
    let created = e
        .app
        .create_issue(
            &ctx,
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Brand new".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(created.issue.identifier, "ENG-131");
    assert_eq!(created.issue.number, 131);
    // Die Aliase lösen alte Links auf.
    let by_alias = e.app.get_issue(&ctx, "OLD-4").await.unwrap();
    assert_eq!(by_alias.view.issue.identifier, "ENG-123");
    // Und der Lineal-Link in einem Dokument zeigt auf die gleiche Kennung.
    let doc: String =
        sqlx::query_scalar("SELECT content FROM document WHERE title = 'Checkout spec'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(doc.contains("ENG-10") && !doc.contains("<issue"));
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn key_conflicts_are_reported_before_the_start_and_key_mapping_resolves_them(pool: PgPool) {
    let e = env(pool).await;
    // In Lineal existiert bereits ein Team ENG.
    sqlx::query("INSERT INTO team (id, organization_id, name, key) VALUES (gen_random_uuid(), $1, 'Existing', 'ENG')")
        .bind(e.org.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let source = fake();
    let planned = e.scan(&source).await;
    assert_eq!(
        planned.stats["scan"]["conflicts"],
        json!([{ "key": "ENG", "reason": "key_exists" }])
    );
    let err = e
        .app
        .import_start(
            &e.alice_ctx(),
            ImportStartInput {
                run_id: planned.id,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Conflict);
    assert_eq!(err.details["conflicts"][0]["key"], "ENG");
    assert_eq!(
        e.count("SELECT count(*) FROM team").await,
        1,
        "nothing was written"
    );
    // Mit Ziel-Key geht es.
    let mut input = ImportStartInput {
        run_id: planned.id,
        ..Default::default()
    };
    input.key_mapping.insert("ENG".into(), "ENGX".into());
    e.app.import_start(&e.alice_ctx(), input).await.unwrap();
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE identifier = 'ENGX-123'")
            .await,
        1
    );
    assert_eq!(
        e.count("SELECT last_issue_number FROM team WHERE key = 'ENGX'")
            .await,
        130
    );
    // Links in Beschreibungen zeigen auf den neuen Key.
    let d: String =
        sqlx::query_scalar("SELECT description FROM issue WHERE identifier = 'ENGX-10'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(d.contains("ENGX-12"), "{d}");
}

// ---------------------------------------------------------------------------------------------
// Kriterium 3: Abbruch nach Phase 4 und Wiederaufnahme ohne Duplikate
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn criterion_3_stop_after_phase_4_and_resume_without_duplicates(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    let planned = e.scan(&source).await;
    e.start(planned.id, ImportStartInput::default()).await;
    let cfg = EngineConfig {
        page_size: 40,
        stop_after_phase: Some(4),
        ..Default::default()
    };
    let out = e.engine(&source, cfg).run(e.org, planned.id).await.unwrap();
    assert_eq!(out, RunOutcome::Stopped { after_phase: 4 });
    let mid = run_of(&e, planned.id).await;
    assert_eq!(mid.status, "running");
    assert_eq!(mid.checkpoint["phase"], 5, "checkpoint points to phase 5");
    assert_eq!(mid.checkpoint["step"], "Comments");
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 200);
    assert_eq!(
        e.count("SELECT count(*) FROM comment").await,
        0,
        "phase 5 has not started"
    );
    // Teams sind während des Laufs gesperrt (Auto-Close, SLA usw. überspringen sie).
    assert_eq!(
        e.count(
            "SELECT count(*) FROM team WHERE import_locked_until > now() - interval '10 years'"
        )
        .await,
        4
    );
    let issue_calls = source.page_calls(SourceEntity::Issues);

    // Wiederaufnahme mit einer frischen Engine.
    let out = e
        .engine(
            &source,
            EngineConfig {
                page_size: 40,
                ..Default::default()
            },
        )
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        source.page_calls(SourceEntity::Issues),
        issue_calls,
        "phase 4 was not read again"
    );
    let resumed = e.snapshot().await;
    assert_eq!(
        e.count("SELECT count(*) FROM team WHERE import_locked_until IS NOT NULL")
            .await,
        0,
        "locks are released"
    );

    // Referenz: ein ununterbrochener Lauf in einem zweiten Workspace ergibt dieselben Zählungen.
    let e2 = env_n(e.pool.clone(), 2).await;
    let (_, out) = e2.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(resumed, e2.snapshot().await);
    // Keine doppelten (team, number).
    assert_eq!(
        e.count("SELECT count(*) FROM (SELECT team_id, number FROM issue GROUP BY 1, 2 HAVING count(*) > 1) d").await,
        0
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn criterion_3b_crash_in_the_middle_of_a_page_sequence_resumes_without_duplicates(
    pool: PgPool,
) {
    let e = env(pool).await;
    let source = fake();
    let planned = e.scan(&source).await;
    e.start(planned.id, ImportStartInput::default()).await;
    // Die Quelle bricht beim dritten Issue-Seitenabruf weg (Netzwerkfehler).
    let base = source.page_calls(SourceEntity::Issues);
    source.fail_after(
        SourceEntity::Issues,
        base + 2,
        SourceError::Transport("connection reset".into()),
    );
    let cfg = EngineConfig {
        page_size: 40,
        ..Default::default()
    };
    let out = e
        .engine(&source, cfg.clone())
        .run(e.org, planned.id)
        .await
        .unwrap();
    let RunOutcome::Failed(msg) = out else {
        panic!("expected failure, got {out:?}")
    };
    assert!(msg.contains("connection reset"), "{msg}");
    let failed = run_of(&e, planned.id).await;
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.checkpoint["step"], "Issues");
    let partial = e.count("SELECT count(*) FROM issue").await;
    assert_eq!(partial, 80, "two committed pages of 40");
    // Wiederaufnahme: Quelle wieder gesund, Lauf fortsetzen.
    source.fail_after_clear();
    let resumed = e
        .app
        .import_resume(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    assert_eq!(resumed.status, "running");
    let out = e.engine(&source, cfg).run(e.org, planned.id).await.unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 200);
    assert_eq!(
        e.count("SELECT count(*) FROM (SELECT team_id, number FROM issue GROUP BY 1, 2 HAVING count(*) > 1) d").await,
        0
    );
    assert_eq!(
        e.count("SELECT count(*) FROM comment").await,
        source.entity(SourceEntity::Comments).len() as i64
    );
    assert_eq!(
        e.count("SELECT count(*) FROM import_mapping WHERE source_type = 'Issue'")
            .await,
        200
    );
}

// ---------------------------------------------------------------------------------------------
// Kriterium 4: Stille (keine Notification, kein Webhook-Aufruf)
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn criterion_4_import_is_silent(pool: PgPool) {
    use lineal_jobs::EventConsumer;
    use lineal_jobs::consumers::notifications::NotificationsConsumer;
    let e = env(pool).await;
    completed_import(&e).await;
    // Alle Events tragen origin = import (und es gibt welche).
    let total = e.count("SELECT count(*) FROM outbox_event").await;
    assert!(total > 300, "events were written: {total}");
    assert_eq!(
        e.count("SELECT count(*) FROM outbox_event WHERE origin <> 'import'")
            .await,
        0
    );
    assert_eq!(
        e.count("SELECT count(*) FROM outbox_event WHERE actor_id IS NOT NULL")
            .await,
        0
    );
    // Der Benachrichtigungs-Konsument überspringt sie: keine Notification, keine automatischen Abos,
    // keine Mails/Digests.
    let subs_before = e.count("SELECT count(*) FROM issue_subscription").await;
    let consumer = NotificationsConsumer;
    let mut handled = 0;
    loop {
        let mut tx = e.app.store.begin_system().await.unwrap();
        let batch = tx
            .outbox()
            .claim_undelivered("notifications", 500)
            .await
            .unwrap();
        if batch.is_empty() {
            break;
        }
        for ev in &batch {
            consumer.handle(&e.app, ev).await.unwrap();
            handled += 1;
        }
        let ids: Vec<_> = batch.iter().map(|b| b.id).collect();
        tx.outbox()
            .mark_delivered("notifications", &ids)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    assert_eq!(i64::from(handled), total);
    assert_eq!(
        e.count("SELECT count(*) FROM notification").await,
        0,
        "not a single notification"
    );
    assert_eq!(
        e.count("SELECT count(*) FROM issue_subscription").await,
        subs_before,
        "no implicit subscriptions"
    );
    // Webhooks: kein Zustellungsversuch (Webhook-Konsument gehört WP16; hier gibt es weder Zustellungen
    // noch Webhook-Endpunkte, und alle Events sind origin = import, die er überspringen muss).
    assert_eq!(e.count("SELECT count(*) FROM webhook").await, 0);
    // Importierte Issues landen nicht in Triage, auch wenn sie ein Nicht-Mitglied angelegt hätte.
    assert_eq!(
        e.count("SELECT count(*) FROM issue i JOIN workflow_state s ON s.id = i.state_id WHERE s.type = 'triage'").await,
        0
    );
}

// ---------------------------------------------------------------------------------------------
// Kriterium 5: zweiter Lauf ohne Änderungen ändert nichts; Delta mit Änderungen und Konflikt
// ---------------------------------------------------------------------------------------------

async fn fingerprint(e: &Env) -> (Vec<(String, i64)>, String, i64) {
    let versions: String = sqlx::query_scalar(
        "SELECT md5(string_agg(id::text || ':' || version || ':' || updated_at::text, ',' ORDER BY id)) FROM (\
           SELECT id, version, updated_at FROM issue UNION ALL SELECT id, version, updated_at FROM comment \
           UNION ALL SELECT id, version, updated_at FROM project UNION ALL SELECT id, version, updated_at FROM team \
           UNION ALL SELECT id, version, updated_at FROM \"user\") x",
    )
    .fetch_one(&e.pool)
    .await
    .unwrap();
    (
        e.snapshot().await,
        versions,
        e.count("SELECT count(*) FROM outbox_event").await,
    )
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn criterion_5_second_run_without_changes_changes_nothing(pool: PgPool) {
    let e = env(pool).await;
    let (source, first) = completed_import(&e).await;
    let before = fingerprint(&e).await;
    // Ein späterer, vollständiger Lauf (kein updatedAfter-Filter): alle Checksummen sind gleich.
    e.clock.set(t0() + chrono::Duration::days(1));
    let (second, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    assert_ne!(first, second);
    let after = fingerprint(&e).await;
    assert_eq!(before.0, after.0, "row counts");
    assert_eq!(before.1, after.1, "versions and updated_at of all objects");
    assert_eq!(before.2, after.2, "no new events");
    let run = run_of(&e, second).await;
    assert_eq!(run.stats["entities"]["Issue"]["unchanged"], 200);
    assert_eq!(run.stats["entities"]["Issue"]["created"], 0);
    assert_eq!(run.stats["entities"]["Issue"]["updated"], 0);
    // Auch der Delta-Modus (updatedAfter = Beginn des letzten Laufs) ändert nichts.
    e.clock.set(t0() + chrono::Duration::days(2));
    let planned = e.scan(&source).await;
    e.start(
        planned.id,
        ImportStartInput {
            delta: true,
            ..Default::default()
        },
    )
    .await;
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(fingerprint(&e).await, after);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn delta_updates_changed_objects_and_reports_conflicts(pool: PgPool) {
    let e = env(pool).await;
    let (source, _) = completed_import(&e).await;
    let issue_version = |ident: &'static str| {
        let pool = e.pool.clone();
        async move {
            sqlx::query_as::<_, (String, i32)>(
                "SELECT title, version FROM issue WHERE identifier = $1",
            )
            .bind(ident)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let later = t0() + chrono::Duration::days(3);
    e.clock.set(later);
    // Linear: ENG-1 und ENG-2 geändert, ENG-4 gelöscht (wird nicht gelöscht, nur gemeldet).
    let upd = (later - chrono::Duration::hours(1)).to_rfc3339();
    source.patch(
        SourceEntity::Issues,
        "i-ENG-1",
        &json!({ "title": "ENG-1 renamed in Linear", "updatedAt": upd }),
    );
    source.patch(
        SourceEntity::Issues,
        "i-ENG-2",
        &json!({ "title": "ENG-2 renamed in Linear", "updatedAt": upd }),
    );
    // Lineal: ENG-2 wurde nach dem Import lokal geändert -> Konflikt.
    sqlx::query("UPDATE issue SET title = 'ENG-2 edited in Lineal', updated_at = $1 WHERE identifier = 'ENG-2'")
        .bind(t0() + chrono::Duration::days(1))
        .execute(&e.pool)
        .await
        .unwrap();
    let events_before = e.count("SELECT count(*) FROM outbox_event").await;
    let planned = e.scan(&source).await;
    e.start(
        planned.id,
        ImportStartInput {
            delta: true,
            ..Default::default()
        },
    )
    .await;
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        issue_version("ENG-1").await,
        ("ENG-1 renamed in Linear".to_owned(), 2)
    );
    assert_eq!(
        issue_version("ENG-2").await.0,
        "ENG-2 edited in Lineal",
        "the Lineal state wins"
    );
    let run = run_of(&e, planned.id).await;
    assert_eq!(run.stats["entities"]["Issue"]["updated"], 1);
    assert_eq!(run.stats["entities"]["Issue"]["conflicts"], 1);
    let (entries, _) = e
        .app
        .import_report(&e.alice_ctx(), planned.id, Some("warning"), 100, 0)
        .await
        .unwrap();
    assert!(
        entries
            .iter()
            .any(|r| r.source_id.as_deref() == Some("i-ENG-2")
                && r.message.contains("changed in Lineal"))
    );
    // Das geänderte Objekt erzeugt ein (stilles) Update-Event, sonst nichts.
    assert_eq!(
        e.count("SELECT count(*) FROM outbox_event").await,
        events_before + 1
    );
    assert_eq!(
        e.count("SELECT count(*) FROM outbox_event WHERE origin <> 'import'")
            .await,
        0
    );
    // Löschungen in Linear werden nicht übernommen.
    let mut issues = source.entity(SourceEntity::Issues);
    issues.retain(|v| v["id"] != "i-ENG-4");
    source.set_entity(SourceEntity::Issues, issues);
    e.clock.set(later + chrono::Duration::days(1));
    let planned = e.scan(&source).await;
    e.start(
        planned.id,
        ImportStartInput {
            delta: false,
            ..Default::default()
        },
    )
    .await;
    e.engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(
        e.count("SELECT count(*) FROM issue WHERE identifier = 'ENG-4'")
            .await,
        1
    );
}

// ---------------------------------------------------------------------------------------------
// Kriterium 6: Token in keiner Log-Zeile, keinem Fehler, keinem Bericht
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
struct LogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
    type Writer = LogBuf;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn criterion_6_token_is_in_no_log_line_error_or_report(pool: PgPool) {
    let buf = LogBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let e = env(pool).await;
    let secret = lineal_app::usecases::import::Secret::new(TOKEN).unwrap();
    // Auch absichtliches Loggen des Credential-Objekts verrät nichts (Redaction).
    tracing::debug!(?secret, "credential for the import");
    tracing::info!(%secret, "credential for the import");
    let source = fake();
    let planned = e.scan(&source).await;
    // Ein fehlschlagender Authentifizierungsversuch erzeugt Fehlertexte.
    source.fail_with(Some(SourceError::Auth));
    e.start(planned.id, ImportStartInput::default()).await;
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    let RunOutcome::Failed(msg) = out else {
        panic!("expected failure: {out:?}")
    };
    assert!(!msg.contains(TOKEN));
    source.fail_with(None);
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await;
    // Der Lauf ist `failed`, nicht `running`: Fortsetzen geht nur über importResume.
    assert!(out.is_err());
    e.app
        .import_resume(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);

    let logs = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.is_empty(), "something was logged");
    assert!(
        logs.contains("[REDACTED]"),
        "the redaction marker is visible instead of the token"
    );
    assert!(!logs.contains(TOKEN), "token found in the logs");
    assert!(
        !logs.contains("FAKETOKEN"),
        "token fragment found in the logs"
    );
    // Weder in Läufen noch im Bericht noch in den Statistiken noch in anderen Tabellen im Klartext.
    let dump: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(r::text, ' '), '') FROM (\
           SELECT to_jsonb(import_run) AS r FROM import_run UNION ALL SELECT to_jsonb(import_issue) FROM import_issue \
           UNION ALL SELECT to_jsonb(import_mapping) FROM import_mapping) x",
    )
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert!(!dump.contains("FAKETOKEN"));
    // Das gespeicherte Token ist verschlüsselt und nach Abschluss gelöscht.
    assert_eq!(e.count("SELECT count(*) FROM import_credential WHERE import_run_id = '00000000-0000-0000-0000-000000000000'").await, 0);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn credential_is_encrypted_at_rest_and_deleted_on_completion_cancel_and_expiry(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    let planned = e.scan(&source).await;
    let blob: Vec<u8> = sqlx::query_scalar(
        "SELECT encrypted_token FROM import_credential WHERE import_run_id = $1",
    )
    .bind(planned.id)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert!(
        !blob.windows(9).any(|w| w == b"FAKETOKEN"),
        "token must not be stored in plain text"
    );
    assert!(blob.len() > TOKEN.len(), "nonce + ciphertext + tag");
    // Abbruch löscht das Token sofort.
    e.app
        .import_cancel(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    assert_eq!(e.count("SELECT count(*) FROM import_credential").await, 0);
    assert_eq!(run_of(&e, planned.id).await.status, "canceled");
    // Abgelaufene Zugangsdaten (7 Tage) werden vom Runner gelöscht.
    let planned = e.scan(&source).await;
    e.clock.set(t0() + chrono::Duration::days(8));
    lineal_import::runner::tick(&e.app, e.app.clock.now())
        .await
        .unwrap();
    assert_eq!(
        e.count("SELECT count(*) FROM import_credential").await,
        0,
        "expired after 7 days"
    );
    let _ = planned;
}

// ---------------------------------------------------------------------------------------------
// Rechte
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn only_admins_may_manage_imports(pool: PgPool) {
    use lineal_app::usecases::import::ImportScanInput;
    let e = env(pool).await;
    let source = fake();
    let planned = e.scan(&source).await;
    let bob = e.bob_ctx();
    let scan = ImportScanInput {
        source: "linear".into(),
        token: TOKEN.into(),
    };
    assert_eq!(
        e.app.import_scan(&bob, scan).await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        e.app
            .import_start(
                &bob,
                ImportStartInput {
                    run_id: planned.id,
                    ..Default::default()
                }
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        e.app.import_pause(&bob, planned.id).await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        e.app
            .import_resume(&bob, planned.id)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        e.app
            .import_cancel(&bob, planned.id)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        e.app
            .import_get_run(&bob, planned.id)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        e.app.import_list_runs(&bob, 10).await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        e.app
            .import_report(&bob, planned.id, None, 10, 0)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    // Auch ein Admin-Token ohne Scope `admin` darf nicht.
    let mut no_admin = e.alice_ctx();
    no_admin.scopes = lineal_app::Scopes::read_write();
    assert_eq!(
        e.app
            .import_get_run(&no_admin, planned.id)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    // Gast sowieso nicht.
    let guest = e.ctx(e.bob, lineal_domain::enums::UserRole::Guest);
    assert_eq!(
        e.app.import_list_runs(&guest, 10).await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    // Der Admin darf; fremde Läufe anderer Workspaces sind nicht sichtbar.
    assert_eq!(
        e.app
            .import_list_runs(&e.alice_ctx(), 10)
            .await
            .unwrap()
            .len(),
        1
    );
    let other = lineal_domain::ids::OrgId(uuid::Uuid::from_u128(77));
    sqlx::query("INSERT INTO organization (id, name, url_key) VALUES ($1, 'Other', 'other-org')")
        .bind(other.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let mut foreign = e.alice_ctx();
    foreign.org_id = other;
    assert_eq!(
        e.app
            .import_get_run(&foreign, planned.id)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}

// ---------------------------------------------------------------------------------------------
// Steuerung: Pause, Fortsetzen, Abbruch, Dry-Run
// ---------------------------------------------------------------------------------------------

/// Quelle, die beim n-ten Issue-Seitenabruf eine Steuerungsaktion ausführt.
struct ControlOnPage<'a> {
    inner: &'a FakeSource,
    env: &'a Env,
    run: uuid::Uuid,
    at_call: usize,
    cancel: bool,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl lineal_import::ImportSource for ControlOnPage<'_> {
    fn kind(&self) -> &'static str {
        "linear"
    }
    async fn workspace(
        &self,
    ) -> lineal_import::port::SourceResult<lineal_import::port::SourceWorkspace> {
        self.inner.workspace().await
    }
    async fn page(
        &self,
        q: &lineal_import::port::PageQuery,
    ) -> lineal_import::port::SourceResult<lineal_import::port::SourcePage> {
        if q.entity == SourceEntity::Issues {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if n == self.at_call {
                let ctx = self.env.alice_ctx();
                if self.cancel {
                    self.env.app.import_cancel(&ctx, self.run).await.unwrap();
                } else {
                    self.env.app.import_pause(&ctx, self.run).await.unwrap();
                }
            }
        }
        self.inner.page(q).await
    }
    async fn download(
        &self,
        url: &str,
    ) -> lineal_import::port::SourceResult<lineal_import::port::Downloaded> {
        self.inner.download(url).await
    }
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn pause_resume_and_cancel(pool: PgPool) {
    let e = env(pool).await;
    let fake_src = fake();
    let planned = e.scan(&fake_src).await;
    e.start(planned.id, ImportStartInput::default()).await;
    let src = ControlOnPage {
        inner: &fake_src,
        env: &e,
        run: planned.id,
        at_call: 2,
        cancel: false,
        calls: Default::default(),
    };
    let out = e
        .engine(
            &src,
            EngineConfig {
                page_size: 40,
                ..Default::default()
            },
        )
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Paused);
    assert_eq!(run_of(&e, planned.id).await.status, "paused");
    assert_eq!(
        e.count("SELECT count(*) FROM issue").await,
        40,
        "the page in flight was rolled back, the first page stayed"
    );
    e.app
        .import_resume(&e.alice_ctx(), planned.id)
        .await
        .unwrap();
    let out = e
        .engine(
            &fake_src,
            EngineConfig {
                page_size: 40,
                ..Default::default()
            },
        )
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 200);
    assert!(
        e.app
            .import_pause(&e.alice_ctx(), planned.id)
            .await
            .is_err(),
        "completed runs cannot be paused"
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn cancel_stops_the_run_deletes_the_token_and_releases_the_locks(pool: PgPool) {
    let e = env(pool).await;
    let fake_src = fake();
    let planned = e.scan(&fake_src).await;
    e.start(planned.id, ImportStartInput::default()).await;
    let src = ControlOnPage {
        inner: &fake_src,
        env: &e,
        run: planned.id,
        at_call: 2,
        cancel: true,
        calls: Default::default(),
    };
    let out = e
        .engine(
            &src,
            EngineConfig {
                page_size: 40,
                ..Default::default()
            },
        )
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Canceled);
    let run = run_of(&e, planned.id).await;
    assert_eq!(run.status, "canceled");
    assert_eq!(e.count("SELECT count(*) FROM import_credential").await, 0);
    assert_eq!(
        e.count("SELECT count(*) FROM team WHERE import_locked_until IS NOT NULL")
            .await,
        0
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn dry_run_validates_without_writing(pool: PgPool) {
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
        .engine(
            &source,
            EngineConfig {
                page_size: 40,
                ..Default::default()
            },
        )
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::DryRunFinished);
    for t in [
        "team",
        "issue",
        "comment",
        "project",
        "cycle",
        "document",
        "outbox_event",
        "import_mapping",
        "attachment",
    ] {
        assert_eq!(
            e.count(&format!("SELECT count(*) FROM {t}")).await,
            0,
            "{t} must stay empty"
        );
    }
    assert_eq!(
        e.count("SELECT count(*) FROM \"user\"").await,
        2,
        "no placeholder users either"
    );
    assert_eq!(e.sink.len(), 0, "no downloads in a dry run");
    let run = run_of(&e, planned.id).await;
    assert_eq!(run.status, "planned", "back to the plan");
    assert_eq!(
        run.stats["entities"]["Issue"]["created"], 200,
        "the report says what would happen"
    );
    let (entries, _) = e
        .app
        .import_report(&e.alice_ctx(), planned.id, None, 100, 0)
        .await
        .unwrap();
    assert!(!entries.is_empty());
    // Danach kann der echte Lauf folgen; der Bericht des Probelaufs wird ersetzt.
    e.start(planned.id, ImportStartInput::default()).await;
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 200);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn team_selection_user_mapping_and_untrusted_data(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    // Ungültige/feindselige Werte aus der Quelle: Bericht statt Abbruch.
    source.patch(
        SourceEntity::Issues,
        "i-WEB-1",
        &json!({
            "title": "x".repeat(400), "priority": 99, "estimate": -5, "dueDate": "not a date",
            "description": "y".repeat(150_000),
        }),
    );
    source.patch(
        SourceEntity::WorkflowStates,
        "s-WEB-todo",
        &json!({ "type": "bogus", "color": "red" }),
    );
    source.patch(
        SourceEntity::Initiatives,
        "i-2",
        &json!({ "status": "Exploding" }),
    );
    source.patch(
        SourceEntity::Teams,
        "t-web",
        &json!({ "timezone": "Mars/Olympus", "autoClosePeriod": 7 }),
    );
    let planned = e.scan(&source).await;
    let mut input = ImportStartInput {
        teams: vec!["ENG".into(), "WEB".into()],
        ..Default::default()
    };
    // Dave wird explizit Bob zugeordnet.
    input.user_mapping.insert("u-dave".into(), Some(e.bob.0));
    e.start(planned.id, input).await;
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, planned.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        e.count("SELECT count(*) FROM team").await,
        2,
        "only the selected teams"
    );
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 150);
    assert_eq!(
        e.count("SELECT count(*) FROM \"user\" WHERE email = 'dave@acme.test'")
            .await,
        0,
        "mapped to Bob, no placeholder"
    );
    assert!(
        e.count(&format!(
            "SELECT count(*) FROM issue WHERE assignee_id = '{}'",
            e.bob.0
        ))
        .await
            > 0
    );
    // DES (privat) und PLT wurden nicht importiert -> auch deren Inhalte nicht.
    assert_eq!(
        e.count("SELECT count(*) FROM project WHERE name = 'Brand refresh'")
            .await,
        0
    );
    let run = run_of(&e, planned.id).await;
    assert_eq!(run.stats["entities"]["Team"]["skipped"], 2);
    // Feindselige Werte wurden begrenzt/ersetzt.
    let (title_len, prio, est, due, dlen): (i32, i32, Option<i32>, Option<chrono::NaiveDate>, i32) = sqlx::query_as(
        "SELECT char_length(title), priority, estimate, due_date, char_length(description) FROM issue WHERE identifier = 'WEB-1'",
    )
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!((title_len, prio, est, due), (255, 0, None, None));
    assert!(dlen <= 100_000);
    assert_eq!(e.count("SELECT count(*) FROM workflow_state WHERE type = 'backlog' AND color = '#95A2B3' AND team_id IN (SELECT id FROM team WHERE key = 'WEB')").await, 2);
    assert_eq!(
        e.count("SELECT count(*) FROM initiative WHERE status = 'Planned'")
            .await,
        1,
        "the unknown status fell back to Planned"
    );
    assert_eq!(e.count("SELECT count(*) FROM team WHERE key = 'WEB' AND timezone = 'America/Los_Angeles' AND auto_close_period IS NULL").await, 1);
    let (entries, _) = e
        .app
        .import_report(&e.alice_ctx(), planned.id, Some("warning"), 200, 0)
        .await
        .unwrap();
    assert!(
        entries
            .iter()
            .any(|r| r.message.contains("unknown state type"))
    );
    assert!(entries.iter().any(|r| r.message.contains("truncated")));
    // Private Teams nur sichtbar importiert, wenn gewählt: hier gar nicht gewählt -> nichts Offeneres.
    assert_eq!(
        e.count("SELECT count(*) FROM team WHERE visibility = 'public'")
            .await,
        2
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn project_status_is_merged_into_an_existing_status_of_the_same_name(pool: PgPool) {
    let e = env(pool).await;
    sqlx::query("INSERT INTO project_status (organization_id, name, type, color) VALUES ($1, 'Backlog', 'backlog', '#BEC2C8')")
        .bind(e.org.0)
        .execute(&e.pool)
        .await
        .unwrap();
    let (_, out) = e.import_all(&fake(), ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(
        e.count("SELECT count(*) FROM project_status").await,
        5,
        "no duplicate `Backlog`"
    );
    assert_eq!(e.count("SELECT count(*) FROM project p JOIN project_status s ON s.id = p.status_id WHERE s.name = 'Backlog'").await, 1);
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn csv_import_end_to_end(pool: PgPool) {
    use lineal_import::CsvSource;
    let e = env(pool).await;
    let csv = "ID,Team,Title,Description,Status,Priority,Creator,Assignee,Labels,Created,Updated,Parent issue\n\
        ENG-5,Engineering,Five,Body,Todo,High,Alice,Bob,\"Bug\",2025-01-06T09:00:00.000Z,2025-01-07T09:00:00.000Z,\n\
        ENG-9,Engineering,Nine,,Done,,Bob,,,2025-01-08T09:00:00.000Z,2025-01-08T09:00:00.000Z,ENG-5\n";
    let source = CsvSource::from_csv(csv.as_bytes(), Some("ENG")).unwrap();
    assert!(!source.warnings().is_empty());
    // Lauf registrieren (CLI-Pfad) und ausführen.
    let scan = json!({ "teams": [{ "id": "csv-team-ENG", "key": "ENG", "name": "Engineering" }] });
    let run = e
        .app
        .import_register_scanned(&e.alice_ctx(), "linearCsv", scan)
        .await
        .unwrap();
    assert_eq!(run.status, "planned");
    e.app
        .import_start(
            &e.alice_ctx(),
            ImportStartInput {
                run_id: run.id,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let out = e
        .engine(&source, EngineConfig::default())
        .run(e.org, run.id)
        .await
        .unwrap();
    assert_eq!(out, RunOutcome::Completed);
    assert_eq!(e.count("SELECT count(*) FROM issue").await, 2);
    assert_eq!(
        e.count("SELECT last_issue_number FROM team WHERE key = 'ENG'")
            .await,
        9
    );
    assert_eq!(e.count("SELECT count(*) FROM issue i JOIN issue p ON p.id = i.parent_id WHERE i.identifier = 'ENG-9' AND p.identifier = 'ENG-5'").await, 1);
    assert_eq!(e.count("SELECT count(*) FROM \"user\" WHERE status = 'imported' AND email LIKE '%@import.invalid'").await, 2, "CSV users carry names only -> placeholders");
    let (entries, _) = e
        .app
        .import_report(&e.alice_ctx(), run.id, Some("info"), 50, 0)
        .await
        .unwrap();
    assert!(
        entries.iter().any(|r| r.message.contains("CSV import")),
        "missing data is reported"
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn imported_text_cannot_make_the_server_fetch_or_embed_foreign_resources(pool: PgPool) {
    let e = env(pool).await;
    let source = fake();
    let own_asset = format!("{}some-other-org/aaa/bbb", e.app.asset_prefix());
    source.patch(
        SourceEntity::Issues,
        "i-ENG-1",
        &json!({ "description": format!(
            "![meta](http://169.254.169.254/latest/meta-data) ![evil](https://evil.example.com/a.png) ![lan](https://10.0.0.5/x.png) \
             ![inj]({own_asset}) https://uploads.linear.app.evil.example.com/x.png"
        ) }),
    );
    source.patch(
        SourceEntity::Attachments,
        "at-1",
        &json!({ "url": own_asset }),
    );
    let (run, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, RunOutcome::Completed);
    // Kein einziger Abruf außer den echten Linear-Uploads aus den Fixtures.
    for u in source.downloads() {
        assert!(
            u.starts_with("https://uploads.linear.app/acme/"),
            "unexpected download {u}"
        );
    }
    let d: String = sqlx::query_scalar("SELECT description FROM issue WHERE identifier = 'ENG-1'")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert!(
        !d.contains(&e.app.asset_prefix()),
        "injected asset path must be neutralized: {d}"
    );
    // Kein Anhang verweist auf einen Lineal-Asset-Pfad, der nicht aus dem Import selbst stammt.
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM attachment WHERE url = '{own_asset}'"
        ))
        .await,
        0
    );
    let (entries, _) = e
        .app
        .import_report(&e.alice_ctx(), run, None, 500, 0)
        .await
        .unwrap();
    assert!(
        entries.iter().any(|r| r.message.contains("never fetched")),
        "reported"
    );
    assert!(
        entries
            .iter()
            .any(|r| r.message.contains("Lineal asset path"))
    );
}
