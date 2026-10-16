#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Import-Platzhalter einem bestehenden User zuordnen (`import_placeholder_merge`).

mod common;

use common::*;
use lineal_app::ErrorKind;
use lineal_app::usecases::import::ImportStartInput;
use lineal_import::{FakeSource, SourceEntity};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn user_id(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM \"user\" WHERE display_name = $1")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn add_user(e: &Env, handle: &str, email: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO \"user\" (id, organization_id, name, display_name, email, role) VALUES ($1, $2, $3, $3, $4, 'user')",
    )
    .bind(id)
    .bind(e.org.0)
    .bind(handle)
    .bind(email)
    .execute(&e.pool)
    .await
    .unwrap();
    id
}

fn source_with_mentions() -> FakeSource {
    let source = fake();
    source.patch(
        SourceEntity::Issues,
        "i-ENG-2",
        &json!({ "description": "Frage an @dave, nicht an @davey oder mail@dave.test. Danke @dave." }),
    );
    source.patch(
        SourceEntity::Issues,
        "i-ENG-4",
        &json!({ "assignee": { "id": "u-dave" }, "creator": { "id": "u-dave" } }),
    );
    source
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn placeholder_is_merged_into_an_existing_user_with_all_references(pool: PgPool) {
    let source = source_with_mentions();
    let e = env(pool.clone()).await;
    let (_, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, lineal_import::engine::RunOutcome::Completed);
    let dave = user_id(&pool, "dave").await;
    let (status, active): (String, bool) =
        sqlx::query_as("SELECT status, active FROM \"user\" WHERE id = $1")
            .bind(dave)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), active), ("imported", false));

    // Ziel: ein echter User mit anderer E-Mail; ist schon (ohne Owner-Flag) im Team PLT und
    // abonniert dasselbe Issue wie der Platzhalter.
    let erin = add_user(&e, "erin", "erin@other.test").await;
    let plt: Uuid = sqlx::query_scalar("SELECT id FROM team WHERE key = 'PLT'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO team_membership (id, organization_id, team_id, user_id, owner) VALUES ($1, $2, $3, $4, false)")
        .bind(Uuid::now_v7())
        .bind(e.org.0)
        .bind(plt)
        .bind(erin)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE team_membership SET owner = true WHERE user_id = $1 AND team_id = $2")
        .bind(dave)
        .bind(plt)
        .execute(&pool)
        .await
        .unwrap();
    let dup_issue: Uuid = sqlx::query_scalar("SELECT id FROM issue ORDER BY created_at LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    for user in [dave, erin] {
        sqlx::query("INSERT INTO issue_subscription (id, organization_id, issue_id, user_id, reason) VALUES ($1, $2, $3, $4, 'manual')")
            .bind(Uuid::now_v7())
            .bind(e.org.0)
            .bind(dup_issue)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    // Benachrichtigungen: eine für den Platzhalter, eine von ihm ausgelöste für erin
    // (nach dem Umhängen wäre actor = user: entfällt wegen der Check-Regel).
    for (user, actor) in [(dave, None), (erin, Some(dave))] {
        sqlx::query("INSERT INTO notification (id, organization_id, user_id, type, category, target_type, actor_id) VALUES ($1, $2, $3, 'importTest', 'system', 'system', $4)")
            .bind(Uuid::now_v7())
            .bind(e.org.0)
            .bind(user)
            .bind(actor)
            .execute(&pool)
            .await
            .unwrap();
    }
    let count = |sql: &'static str, id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let assigned = count("SELECT count(*) FROM issue WHERE assignee_id = $1", dave).await;
    let comments = count("SELECT count(*) FROM comment WHERE user_id = $1", dave).await;
    let leads = count("SELECT count(*) FROM project WHERE lead_id = $1", dave).await;
    let subs = count(
        "SELECT count(*) FROM issue_subscription WHERE user_id = $1",
        dave,
    )
    .await;
    let updates = count(
        "SELECT count(*) FROM project_update WHERE user_id = $1",
        dave,
    )
    .await;
    assert!(
        assigned > 0 && comments > 0 && leads > 0 && subs > 0 && updates > 0,
        "{assigned} {comments} {leads} {subs} {updates}"
    );
    let erin_subs_before = count(
        "SELECT count(*) FROM issue_subscription WHERE user_id = $1",
        erin,
    )
    .await;

    // Rechte: nur Owner/Admin.
    let denied = e
        .app
        .import_placeholder_merge(&e.bob_ctx(), dave, erin)
        .await
        .unwrap_err();
    assert_eq!(denied.kind, ErrorKind::Forbidden);

    let stats = e
        .app
        .import_placeholder_merge(&e.alice_ctx(), dave, erin)
        .await
        .unwrap();
    assert!(stats.moved > 0 && stats.dropped >= 2, "{stats:?}");
    assert!(stats.mentions >= 1);

    assert_eq!(
        count("SELECT count(*) FROM \"user\" WHERE id = $1", dave).await,
        0,
        "placeholder deleted"
    );
    assert_eq!(
        count("SELECT count(*) FROM issue WHERE assignee_id = $1", erin).await,
        assigned
    );
    assert_eq!(
        count("SELECT count(*) FROM comment WHERE user_id = $1", erin).await,
        comments
    );
    assert_eq!(
        count("SELECT count(*) FROM project WHERE lead_id = $1", erin).await,
        leads
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM project_update WHERE user_id = $1",
            erin
        )
        .await,
        updates
    );
    // Abos: Vereinigung, das Duplikat entfällt.
    let erin_subs = count(
        "SELECT count(*) FROM issue_subscription WHERE user_id = $1",
        erin,
    )
    .await;
    assert_eq!(erin_subs, erin_subs_before + subs - 1);
    // Team-Mitgliedschaft: eine Zeile, Owner-Flag übernommen.
    let (n, owner): (i64, bool) = sqlx::query_as(
        "SELECT count(*), bool_or(owner) FROM team_membership WHERE user_id = $1 AND team_id = $2",
    )
    .bind(erin)
    .bind(plt)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((n, owner), (1, true));
    // Benachrichtigungen: die eigene wandert, die selbst ausgelöste entfällt.
    assert_eq!(
        count("SELECT count(*) FROM notification WHERE user_id = $1", erin).await,
        1
    );
    // Mapping zeigt auf das Ziel (spätere Abgleiche).
    let mapped: Uuid = sqlx::query_scalar(
        "SELECT target_id FROM import_mapping WHERE source_type = 'User' AND source_id = 'u-dave'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(mapped, erin);
    // Erwähnungen im Text: nur der genaue Handle.
    let d: String = sqlx::query_scalar(
        "SELECT i.description FROM issue i JOIN team t ON t.id = i.team_id WHERE t.key = 'ENG' AND i.number = 2",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        d,
        "Frage an @erin, nicht an @davey oder mail@dave.test. Danke @erin."
    );
    // Audit-Eintrag.
    assert_eq!(
        e.count("SELECT count(*) FROM audit_log WHERE action = 'import.placeholder_merged'")
            .await,
        1
    );
}

#[sqlx::test(migrator = "lineal_db::MIGRATOR")]
async fn merge_rules_placeholder_to_active_user_only(pool: PgPool) {
    let source = fake();
    let e = env(pool.clone()).await;
    let (_, out) = e.import_all(&source, ImportStartInput::default()).await;
    assert_eq!(out, lineal_import::engine::RunOutcome::Completed);
    let dave = user_id(&pool, "dave").await;
    let carol = user_id(&pool, "carol").await;
    let erin = add_user(&e, "erin", "erin@other.test").await;
    let alice = e.alice.0;
    let ctx = e.alice_ctx();
    // aktiv -> aktiv nie
    let err = e
        .app
        .import_placeholder_merge(&ctx, e.bob.0, erin)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    // Platzhalter -> Platzhalter nie
    let err = e
        .app
        .import_placeholder_merge(&ctx, dave, carol)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    // identisch
    assert!(
        e.app
            .import_placeholder_merge(&ctx, dave, dave)
            .await
            .is_err()
    );
    // Ziel aus einem anderen Workspace -> not_found
    let other = env_n(pool.clone(), 2).await;
    let foreign = other.alice.0;
    let err = e
        .app
        .import_placeholder_merge(&ctx, dave, foreign)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    // nichts wurde verändert
    assert_eq!(
        e.count(&format!(
            "SELECT count(*) FROM \"user\" WHERE id = '{dave}'"
        ))
        .await,
        1
    );
    // Platzhalter -> Alice (aktiv) ist erlaubt.
    e.app
        .import_placeholder_merge(&ctx, dave, alice)
        .await
        .unwrap();
}
