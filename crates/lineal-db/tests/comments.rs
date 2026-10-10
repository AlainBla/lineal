#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP09: Kommentare, Reaktionen und Anhänge gegen echte Datenbank (Seed `acme`).
//! Testdaten (Issues, Projekte, Initiativen) werden per SQL angelegt.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use lineal_app::ports::repos::comment::{ParentKind, ReactionTarget};
use lineal_app::ports::repos::user::OrderBy;
use lineal_app::ports::services::BlobInfo;
use lineal_app::ports::{BlobStore, Clock, Store};
use lineal_app::usecases::attachment::{LinkAttachmentInput, PrepareUploadInput};
use lineal_app::usecases::comment::{
    CreateCommentInput, DELETED_PLACEHOLDER, ListCommentsInput, ParentSpec, ReactionTargetSpec,
};
use lineal_app::{App, AppResult, ErrorKind, Scopes};
use lineal_db::PgStore;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use uuid::Uuid;

struct TestClock(Mutex<DateTime<Utc>>);
impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
struct MemBlobs(Mutex<HashMap<String, (u64, String)>>);

#[async_trait]
impl BlobStore for MemBlobs {
    async fn signed_put_url(&self, key: &str, _ct: &str, _ttl: StdDuration) -> AppResult<String> {
        Ok(format!("mem://put/{key}"))
    }
    async fn signed_get_url(&self, key: &str, ttl: StdDuration) -> AppResult<String> {
        Ok(format!("mem://get/{key}?ttl={}", ttl.as_secs()))
    }
    async fn head(&self, key: &str) -> AppResult<Option<BlobInfo>> {
        Ok(self.0.lock().unwrap().get(key).map(|(size, ct)| BlobInfo {
            size: *size,
            content_type: ct.clone(),
        }))
    }
}

struct Env {
    app: App,
    data: SeedData,
    pool: PgPool,
    clock: Arc<TestClock>,
    blobs: Arc<MemBlobs>,
}

async fn setup(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    let clock = Arc::new(TestClock(Mutex::new(
        "2026-10-10T08:00:00Z".parse::<DateTime<Utc>>().unwrap(),
    )));
    let blobs = Arc::new(MemBlobs::default());
    app.clock = clock.clone();
    app.blobs = blobs.clone();
    Env {
        app,
        data,
        pool,
        clock,
        blobs,
    }
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn slot(env: &Env, team: lineal_domain::ids::TeamId) -> u128 {
    if team == env.data.eng {
        1
    } else if team == env.data.web {
        2
    } else {
        3
    }
}

async fn make_issue(env: &Env, team: lineal_domain::ids::TeamId) -> Uuid {
    let id = Uuid::now_v7();
    let state = seed_workflow_state_ids(slot(env, team))[0];
    sqlx::query(
        "WITH n AS (UPDATE team SET last_issue_number = last_issue_number + 1 \
         WHERE id = $1 RETURNING last_issue_number) \
         INSERT INTO issue (id, organization_id, team_id, number, title, state_id) \
         SELECT $2, $3, $1, last_issue_number, 'Login broken', $4 FROM n",
    )
    .bind(team.0)
    .bind(id)
    .bind(env.data.org.0)
    .bind(state.0)
    .execute(&env.pool)
    .await
    .unwrap();
    id
}

async fn make_project(env: &Env, teams: &[lineal_domain::ids::TeamId], number: i32) -> Uuid {
    let id = Uuid::now_v7();
    let status: Uuid = sqlx::query_scalar("SELECT id FROM project_status LIMIT 1")
        .fetch_one(&env.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project (id, organization_id, name, lead_team_id, number, status_id) \
         VALUES ($1, $2, 'Comment Test Project', $3, $4, $5)",
    )
    .bind(id)
    .bind(env.data.org.0)
    .bind(teams[0].0)
    .bind(number + 100) // Seed (WP07) belegt bereits P-ENG-1
    .bind(status)
    .execute(&env.pool)
    .await
    .unwrap();
    for t in teams {
        sqlx::query(
            "INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1, $2, $3)",
        )
        .bind(env.data.org.0)
        .bind(id)
        .bind(t.0)
        .execute(&env.pool)
        .await
        .unwrap();
    }
    id
}

fn on_issue(issue: Uuid, body: &str) -> CreateCommentInput {
    CreateCommentInput {
        parent: Some(ParentSpec::Issue(issue.to_string())),
        body: body.into(),
        ..Default::default()
    }
}

fn reply(to: &str, body: &str) -> CreateCommentInput {
    CreateCommentInput {
        reply_to: Some(to.into()),
        body: body.into(),
        ..Default::default()
    }
}

async fn events(env: &Env, ty: &str) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT payload FROM outbox_event WHERE type = $1 ORDER BY created_at, sync_id",
    )
    .bind(ty)
    .fetch_all(&env.pool)
    .await
    .unwrap()
}

// ----------------------------------------------------------------------------------------------
// Kommentare
// ----------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "./migrations")]
async fn comment_on_private_team_issue_is_not_found_for_non_members(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.des).await;
    let bob = ctx_for(SeedUser::Bob); // Mitglied von DES
    let created = env
        .app
        .create_comment(&bob, on_issue(issue, "Design review needed"))
        .await
        .unwrap();
    assert!(created.view.issue_identifier.unwrap().starts_with("DES-"));

    for who in [SeedUser::Alice, SeedUser::Carol] {
        let ctx = ctx_for(who);
        assert_eq!(
            kind(env.app.create_comment(&ctx, on_issue(issue, "hi")).await),
            ErrorKind::NotFound,
            "{who:?} create"
        );
        let list = env
            .app
            .list_comments(
                &ctx,
                ListCommentsInput {
                    parent: ParentSpec::Issue(issue.to_string()),
                    order_by: OrderBy::CreatedAt,
                    cursor: None,
                    limit: 50,
                },
            )
            .await;
        assert_eq!(kind(list), ErrorKind::NotFound, "{who:?} list");
        // direkte Kommentar-ID verrät ebenfalls nichts
        assert_eq!(
            kind(
                env.app
                    .update_comment(&ctx, &created.view.comment.id.to_string(), "x")
                    .await
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(
                env.app
                    .delete_comment(&ctx, &created.view.comment.id.to_string())
                    .await
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(
                env.app
                    .add_reaction(
                        &ctx,
                        ReactionTargetSpec::Comment(created.view.comment.id.to_string()),
                        "thumbsup"
                    )
                    .await
            ),
            ErrorKind::NotFound
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn threads_replies_are_normalized_to_the_root(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let root = env
        .app
        .create_comment(&alice, on_issue(issue, "Root"))
        .await
        .unwrap();
    let root_id = root.view.comment.id;
    let r1 = env
        .app
        .create_comment(&bob, reply(&root_id.to_string(), "First reply"))
        .await
        .unwrap();
    assert_eq!(r1.view.comment.parent_id, Some(root_id));
    let r2 = env
        .app
        .create_comment(
            &alice,
            reply(&r1.view.comment.id.to_string(), "Reply to reply"),
        )
        .await
        .unwrap();
    assert_eq!(r2.view.comment.parent_id, Some(root_id), "normalized");
    assert_eq!(r2.view.comment.parent, root.view.comment.parent);

    let page = env
        .app
        .list_comments(
            &bob,
            ListCommentsInput {
                parent: ParentSpec::Issue(issue.to_string()),
                order_by: OrderBy::CreatedAt,
                cursor: None,
                limit: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.next.is_some());
    let page2 = env
        .app
        .list_comments(
            &bob,
            ListCommentsInput {
                parent: ParentSpec::Issue(issue.to_string()),
                order_by: OrderBy::CreatedAt,
                cursor: page.next,
                limit: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(page2.items.len(), 1);
    assert!(page2.next.is_none());
    // Reply plus parent plus unknown reply target / empty body / both parents
    assert_eq!(
        kind(
            env.app
                .create_comment(&bob, reply(&Uuid::now_v7().to_string(), "x"))
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(env.app.create_comment(&bob, on_issue(issue, "   ")).await),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            env.app
                .create_comment(
                    &bob,
                    CreateCommentInput {
                        body: "x".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // Autor abonniert das Issue (Top-Level) bzw. den Thread (Antwort)
    let subs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM issue_subscription WHERE issue_id = $1")
            .bind(issue)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(subs, 3, "alice issue, bob thread, alice thread");
}

#[sqlx::test(migrations = "./migrations")]
async fn only_the_author_can_edit(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let c = env
        .app
        .create_comment(&alice, on_issue(issue, "Original"))
        .await
        .unwrap();
    let id = c.view.comment.id.to_string();
    assert_eq!(
        kind(env.app.update_comment(&bob, &id, "Hijack").await),
        ErrorKind::Forbidden
    );
    env.clock
        .0
        .lock()
        .unwrap()
        .clone_from(&"2026-10-10T09:00:00Z".parse().unwrap());
    let edited = env.app.update_comment(&alice, &id, "Edited").await.unwrap();
    assert_eq!(edited.view.comment.body, "Edited");
    assert!(edited.view.comment.edited_at.is_some());
    assert_eq!(edited.view.comment.version, 2);
    assert_eq!(events(&env, "CommentUpdated").await.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_rules(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let alice = ctx_for(SeedUser::Alice); // Owner = Admin-Recht
    let bob = ctx_for(SeedUser::Bob);

    // Inline-Wurzel nicht löschbar; Antworten schon
    let inline = env
        .app
        .create_comment(
            &alice,
            CreateCommentInput {
                parent: Some(ParentSpec::Issue(issue.to_string())),
                body: "Why this value?".into(),
                quoted_text: Some("timeout = 30".into()),
                reply_to: None,
            },
        )
        .await
        .unwrap();
    let inline_id = inline.view.comment.id.to_string();
    assert_eq!(
        inline.view.comment.quoted_text.as_deref(),
        Some("timeout = 30")
    );
    let inline_reply = env
        .app
        .create_comment(&bob, reply(&inline_id, "Because"))
        .await
        .unwrap();
    assert_eq!(inline_reply.view.comment.quoted_text, None);
    let err = env
        .app
        .delete_comment(&alice, &inline_id)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert!(
        err.message
            .contains("Inline comment roots cannot be deleted")
    );
    // Antworten dürfen kein quotedText tragen
    assert_eq!(
        kind(
            env.app
                .create_comment(
                    &bob,
                    CreateCommentInput {
                        reply_to: Some(inline_id.clone()),
                        body: "x".into(),
                        quoted_text: Some("y".into()),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    env.app
        .delete_comment(&bob, &inline_reply.view.comment.id.to_string())
        .await
        .unwrap();

    // Fremden Kommentar löschen: Nicht-Admin → forbidden, Admin darf
    let by_alice = env
        .app
        .create_comment(&alice, on_issue(issue, "Alice says"))
        .await
        .unwrap();
    assert_eq!(
        kind(
            env.app
                .delete_comment(&bob, &by_alice.view.comment.id.to_string())
                .await
        ),
        ErrorKind::Forbidden
    );
    let by_bob = env
        .app
        .create_comment(&bob, on_issue(issue, "Bob says"))
        .await
        .unwrap();
    env.app
        .delete_comment(&alice, &by_bob.view.comment.id.to_string())
        .await
        .unwrap();
    // gelöscht → nicht mehr auffindbar, zweites Löschen ist not_found
    assert_eq!(
        kind(
            env.app
                .delete_comment(&alice, &by_bob.view.comment.id.to_string())
                .await
        ),
        ErrorKind::NotFound
    );

    // Top-Level mit Antworten: Platzhalter, Antworten bleiben
    let rid = by_alice.view.comment.id.to_string();
    env.app
        .create_comment(&bob, reply(&rid, "Reply stays"))
        .await
        .unwrap();
    env.app.delete_comment(&alice, &rid).await.unwrap();
    let page = env
        .app
        .list_comments(
            &alice,
            ListCommentsInput {
                parent: ParentSpec::Issue(issue.to_string()),
                order_by: OrderBy::CreatedAt,
                cursor: None,
                limit: 50,
            },
        )
        .await
        .unwrap();
    let bodies: Vec<_> = page.items.iter().map(|v| v.comment.body.clone()).collect();
    assert!(
        bodies.contains(&DELETED_PLACEHOLDER.to_string()),
        "{bodies:?}"
    );
    assert!(bodies.contains(&"Reply stays".to_string()));
    assert!(!bodies.contains(&"Bob says".to_string()));
    // Auf eine Wurzel mit Platzhalter kann weiter geantwortet werden
    let late = env
        .app
        .create_comment(&bob, reply(&rid, "Late reply"))
        .await
        .unwrap();
    assert_eq!(
        late.view.comment.parent_id.map(|p| p.to_string()),
        Some(rid.clone())
    );
    assert_eq!(events(&env, "CommentRemoved").await.len(), 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn resolve_and_unresolve_threads(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let root = env
        .app
        .create_comment(&alice, on_issue(issue, "Question"))
        .await
        .unwrap();
    let rid = root.view.comment.id.to_string();
    let answer = env
        .app
        .create_comment(&bob, reply(&rid, "Answer"))
        .await
        .unwrap();
    let other_root = env
        .app
        .create_comment(&alice, on_issue(issue, "Other"))
        .await
        .unwrap();
    // Antwort kann nicht selbst aufgelöst werden
    assert_eq!(
        kind(
            env.app
                .resolve_comment(&bob, &answer.view.comment.id.to_string(), None)
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // resolvingComment muss zum Thread gehören
    assert_eq!(
        kind(
            env.app
                .resolve_comment(&bob, &rid, Some(&other_root.view.comment.id.to_string()))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let resolved = env
        .app
        .resolve_comment(&bob, &rid, Some(&answer.view.comment.id.to_string()))
        .await
        .unwrap();
    let c = &resolved.view.comment;
    assert!(c.resolved_at.is_some());
    assert_eq!(c.resolving_user_id, Some(env.data.bob));
    assert_eq!(c.resolving_comment_id, Some(answer.view.comment.id));
    // neue Antwort öffnet den Thread nicht automatisch
    env.app
        .create_comment(&alice, reply(&rid, "Thanks"))
        .await
        .unwrap();
    let still = env.app.get_comment(&alice, &rid).await.unwrap();
    assert!(still.comment.resolved_at.is_some());
    let open = env.app.unresolve_comment(&alice, &rid).await.unwrap();
    assert!(open.view.comment.resolved_at.is_none());
    assert!(open.view.comment.resolving_comment_id.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn reactions_are_idempotent_and_aggregated(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let c = env
        .app
        .create_comment(&alice, on_issue(issue, "Nice"))
        .await
        .unwrap();
    let target = ReactionTargetSpec::Comment(c.view.comment.id.to_string());
    let r1 = env
        .app
        .add_reaction(&bob, target.clone(), ":thumbsup:")
        .await
        .unwrap();
    assert!(r1.created);
    let again = env
        .app
        .add_reaction(&bob, target.clone(), "thumbsup")
        .await
        .unwrap();
    assert!(!again.created);
    assert_eq!(again.reaction.id, r1.reaction.id);
    env.app
        .add_reaction(&alice, target.clone(), "thumbsup")
        .await
        .unwrap();
    env.app
        .add_reaction(&bob, target.clone(), "tada")
        .await
        .unwrap();
    assert_eq!(events(&env, "ReactionCreated").await.len(), 3);

    let v = env
        .app
        .get_comment(&alice, &c.view.comment.id.to_string())
        .await
        .unwrap();
    let thumbs = v.reactions.iter().find(|g| g.emoji == "thumbsup").unwrap();
    assert_eq!(thumbs.count, 2);
    assert!(thumbs.user_ids.contains(&env.data.alice) && thumbs.user_ids.contains(&env.data.bob));
    assert_eq!(v.reactions.len(), 2);

    // nur der reagierende User darf entfernen
    assert_eq!(
        kind(
            env.app
                .remove_reaction(&alice, &r1.reaction.id.to_string())
                .await
        ),
        ErrorKind::Forbidden
    );
    env.app
        .remove_reaction(&bob, &r1.reaction.id.to_string())
        .await
        .unwrap();
    assert_eq!(
        kind(
            env.app
                .remove_reaction(&bob, &r1.reaction.id.to_string())
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(events(&env, "ReactionRemoved").await.len(), 1);
    assert_eq!(
        kind(env.app.add_reaction(&bob, target, "a b").await),
        ErrorKind::InvalidRequest
    );
    // Reaktion auf das Issue selbst
    let on_issue_r = env
        .app
        .add_reaction(&bob, ReactionTargetSpec::Issue(issue.to_string()), "eyes")
        .await
        .unwrap();
    assert!(matches!(
        on_issue_r.reaction.target,
        ReactionTarget::Issue(_)
    ));
    // Lese-Scope darf nicht reagieren
    let mut ro = ctx_for(SeedUser::Bob);
    ro.scopes = Scopes::read_only();
    assert_eq!(
        kind(
            env.app
                .add_reaction(&ro, ReactionTargetSpec::Issue(issue.to_string()), "x")
                .await
        ),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn markdown_is_sanitized(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let bob = ctx_for(SeedUser::Bob);
    let c = env
        .app
        .create_comment(
            &bob,
            on_issue(
                issue,
                "Look <script>alert(1)</script><img src=x onerror=alert(2)> [click](javascript:alert(3)) **bold**",
            ),
        )
        .await
        .unwrap();
    let body = &c.view.comment.body;
    assert!(!body.contains("<script"), "{body}");
    assert!(!body.contains("onerror"), "{body}");
    assert!(!body.contains("javascript:"), "{body}");
    assert!(body.contains("**bold**"));
    let stored: String = sqlx::query_scalar("SELECT body FROM comment WHERE id = $1")
        .bind(c.view.comment.id.0)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    assert_eq!(&stored, body);
    let edited = env
        .app
        .update_comment(
            &bob,
            &c.view.comment.id.to_string(),
            "<iframe src=//evil></iframe>ok",
        )
        .await
        .unwrap();
    assert!(!edited.view.comment.body.contains("<iframe"));
}

#[sqlx::test(migrations = "./migrations")]
async fn mentions_only_include_users_with_access(pool: PgPool) {
    let env = setup(pool).await;
    // ENG-Issue: Carol (Gast, nur WEB) hat keinen Zugriff, Bob schon.
    let issue = make_issue(&env, env.data.eng).await;
    let alice = ctx_for(SeedUser::Alice);
    let c = env
        .app
        .create_comment(
            &alice,
            on_issue(issue, "cc @bob and @carol, also @nobody and a@b.example"),
        )
        .await
        .unwrap();
    assert_eq!(c.mentioned_user_ids, vec![env.data.bob]);
    let ev = events(&env, "CommentCreated").await;
    assert_eq!(ev.len(), 1);
    let ids = ev[0]["mentioned_user_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 1, "{ev:?}");
    assert_eq!(ids[0], env.data.bob.0.to_string());

    // Issue in WEB: Carol ist Mitglied → erwähnbar; Satzzeichen hinter dem Handle
    let web_issue = make_issue(&env, env.data.web).await;
    let w = env
        .app
        .create_comment(&alice, on_issue(web_issue, "Ping @carol."))
        .await
        .unwrap();
    assert_eq!(w.mentioned_user_ids, vec![env.data.carol]);

    // Update: nur neu hinzugekommene Erwähnungen
    let edited = env
        .app
        .update_comment(
            &alice,
            &w.view.comment.id.to_string(),
            "Ping @carol. And @bob.",
        )
        .await
        .unwrap();
    assert_eq!(edited.mentioned_user_ids, vec![env.data.bob]);
    let upd = events(&env, "CommentUpdated").await;
    assert_eq!(upd[0]["mentioned_user_ids"].as_array().unwrap().len(), 1);

    // privates Team: Alice ist kein Mitglied → kann nicht kommentieren; Bob erwähnt Carol (Gast) → leer
    let des_issue = make_issue(&env, env.data.des).await;
    let bob = ctx_for(SeedUser::Bob);
    let d = env
        .app
        .create_comment(&bob, on_issue(des_issue, "@carol @alice"))
        .await
        .unwrap();
    assert!(
        d.mentioned_user_ids.is_empty(),
        "{:?}",
        d.mentioned_user_ids
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn project_initiative_and_status_update_parents(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);

    // Projekt über ENG + DES: nur wer alle Teams sehen darf
    let project = make_project(&env, &[env.data.eng, env.data.des], 1).await;
    let spec = ParentSpec::Project(project.to_string());
    let ok = env
        .app
        .create_comment(
            &bob,
            CreateCommentInput {
                parent: Some(spec.clone()),
                body: "Kickoff".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        ok.view.comment.parent,
        lineal_app::ports::repos::comment::ParentRef::Project(_)
    ));
    for ctx in [&alice, &carol] {
        assert_eq!(
            kind(
                env.app
                    .create_comment(
                        ctx,
                        CreateCommentInput {
                            parent: Some(spec.clone()),
                            body: "x".into(),
                            ..Default::default()
                        }
                    )
                    .await
            ),
            ErrorKind::NotFound
        );
    }
    // Referenz über Kennung P-ENG-101 und Name
    for r in ["P-ENG-101", "Comment Test Project"] {
        let page = env
            .app
            .list_comments(
                &bob,
                ListCommentsInput {
                    parent: ParentSpec::Project(r.into()),
                    order_by: OrderBy::UpdatedAt,
                    cursor: None,
                    limit: 50,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1, "{r}");
    }

    // Statusupdate: genau ein Thread
    let update = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO project_update (id, organization_id, project_id, user_id, body) VALUES ($1, $2, $3, $4, 'Week 1')",
    )
    .bind(update)
    .bind(env.data.org.0)
    .bind(project)
    .bind(env.data.bob.0)
    .execute(&env.pool)
    .await
    .unwrap();
    let su = ParentSpec::StatusUpdate {
        id: update.to_string(),
        kind: None,
    };
    let first = env
        .app
        .create_comment(
            &bob,
            CreateCommentInput {
                parent: Some(su.clone()),
                body: "Thread start".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(first.view.comment.parent_id.is_none());
    let second = env
        .app
        .create_comment(
            &bob,
            CreateCommentInput {
                parent: Some(su),
                body: "Second".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(second.view.comment.parent_id, Some(first.view.comment.id));

    // Initiative: Workspace-Recht ViewInitiatives (Gast verboten), Schreib-Scope nötig
    let initiative = Uuid::now_v7();
    sqlx::query("INSERT INTO initiative (id, organization_id, number, name) VALUES ($1, $2, 101, 'Comment Test Initiative')")
        .bind(initiative)
        .bind(env.data.org.0)
        .execute(&env.pool)
        .await
        .unwrap();
    let ispec = ParentSpec::Initiative("I-101".into());
    let ic = env
        .app
        .create_comment(
            &bob,
            CreateCommentInput {
                parent: Some(ispec.clone()),
                body: "Go".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        ic.view.comment.parent,
        lineal_app::ports::repos::comment::ParentRef::Initiative(_)
    ));
    assert_eq!(
        kind(
            env.app
                .create_comment(
                    &carol,
                    CreateCommentInput {
                        parent: Some(ispec.clone()),
                        body: "x".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    let mut ro = ctx_for(SeedUser::Bob);
    ro.scopes = Scopes::read_only();
    assert_eq!(
        kind(
            env.app
                .create_comment(
                    &ro,
                    CreateCommentInput {
                        parent: Some(ispec),
                        body: "x".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    // Unbekanntes Elternobjekt
    assert_eq!(
        kind(
            env.app
                .create_comment(
                    &bob,
                    CreateCommentInput {
                        parent: Some(ParentSpec::Document("nope".into())),
                        body: "x".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    let _ = ParentKind::Issue;
}

// ----------------------------------------------------------------------------------------------
// Anhänge
// ----------------------------------------------------------------------------------------------

#[sqlx::test(migrations = "./migrations")]
async fn link_attachments_are_deduplicated_per_url(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let bob = ctx_for(SeedUser::Bob);
    let first = env
        .app
        .link_attachment(
            &bob,
            LinkAttachmentInput {
                issue: issue.to_string(),
                url: "https://Example.com/pr/1/#files".into(),
                title: Some("PR 1".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(first.created);
    assert_eq!(first.view.attachment.url, "https://example.com/pr/1");
    let second = env
        .app
        .link_attachment(
            &bob,
            LinkAttachmentInput {
                issue: issue.to_string(),
                url: "https://example.com/pr/1".into(),
                title: Some("PR 1 (renamed)".into()),
                subtitle: Some("open".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!second.created);
    assert_eq!(second.view.attachment.id, first.view.attachment.id);
    assert_eq!(second.view.attachment.title, "PR 1 (renamed)");
    assert_eq!(second.view.attachment.subtitle.as_deref(), Some("open"));
    let list = env
        .app
        .list_issue_attachments(&bob, &issue.to_string())
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(events(&env, "AttachmentCreated").await.len(), 1);
    assert_eq!(events(&env, "AttachmentUpdated").await.len(), 1);

    // get: externe URL unverändert, kein Ablauf
    let got = env
        .app
        .get_attachment(&bob, &first.view.attachment.id.to_string())
        .await
        .unwrap();
    assert_eq!(got.url, "https://example.com/pr/1");
    assert!(got.expires_at.is_none());
    assert_eq!(got.issue_identifier, first.view.issue_identifier);

    // ungültige URL
    assert_eq!(
        kind(
            env.app
                .link_attachment(
                    &bob,
                    LinkAttachmentInput {
                        issue: issue.to_string(),
                        url: "javascript:alert(1)".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );

    // löschen und erneut anhängen belebt denselben Anhang wieder
    env.app
        .delete_attachment(&bob, &first.view.attachment.id.to_string())
        .await
        .unwrap();
    assert_eq!(
        kind(
            env.app
                .get_attachment(&bob, &first.view.attachment.id.to_string())
                .await
        ),
        ErrorKind::NotFound
    );
    assert!(
        env.app
            .list_issue_attachments(&bob, &issue.to_string())
            .await
            .unwrap()
            .is_empty()
    );
    let again = env
        .app
        .link_attachment(
            &bob,
            LinkAttachmentInput {
                issue: issue.to_string(),
                url: "https://example.com/pr/1".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(again.created);
    // Verlauf: Hinzufügen, Entfernen, Hinzufügen
    let flags: Vec<String> = sqlx::query_scalar(
        "SELECT flags[1] FROM issue_history WHERE issue_id = $1 AND attachment_id IS NOT NULL ORDER BY created_at, id",
    )
    .bind(issue)
    .fetch_all(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        flags,
        ["attachmentAdded", "attachmentRemoved", "attachmentAdded"]
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn attachments_respect_team_visibility_and_scope(pool: PgPool) {
    let env = setup(pool).await;
    let des_issue = make_issue(&env, env.data.des).await;
    let eng_issue = make_issue(&env, env.data.eng).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let att = env
        .app
        .link_attachment(
            &bob,
            LinkAttachmentInput {
                issue: des_issue.to_string(),
                url: "https://example.com/spec".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = att.view.attachment.id.to_string();
    assert_eq!(
        kind(env.app.get_attachment(&alice, &id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(env.app.delete_attachment(&alice, &id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            env.app
                .link_attachment(
                    &alice,
                    LinkAttachmentInput {
                        issue: des_issue.to_string(),
                        url: "https://example.com/x".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            env.app
                .prepare_attachment_upload(
                    &alice,
                    PrepareUploadInput {
                        issue: Some(des_issue.to_string()),
                        filename: "a.png".into(),
                        content_type: "image/png".into(),
                        size: 10,
                        title: None,
                        subtitle: None,
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    let mut ro = ctx_for(SeedUser::Bob);
    ro.scopes = Scopes::read_only();
    assert_eq!(
        kind(
            env.app
                .link_attachment(
                    &ro,
                    LinkAttachmentInput {
                        issue: eng_issue.to_string(),
                        url: "https://example.com/x".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    // Lesen mit Read-Scope ist erlaubt
    env.app.get_attachment(&ro, &id).await.unwrap();
}

fn prepare(issue: Uuid, size: i64) -> PrepareUploadInput {
    PrepareUploadInput {
        issue: Some(issue.to_string()),
        filename: "screenshot.png".into(),
        content_type: "image/png".into(),
        size,
        title: None,
        subtitle: None,
    }
}

fn key_of(asset_url: &str) -> String {
    asset_url.split("/assets/").nth(1).unwrap().to_owned()
}

fn put(env: &Env, asset_url: &str, size: u64) {
    env.blobs
        .0
        .lock()
        .unwrap()
        .insert(key_of(asset_url), (size, "image/png".into()));
}

#[sqlx::test(migrations = "./migrations")]
async fn upload_flow_prepare_put_finalize(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let bob = ctx_for(SeedUser::Bob);
    let prepared = env
        .app
        .prepare_attachment_upload(&bob, prepare(issue, 48213))
        .await
        .unwrap();
    assert!(prepared.url.starts_with("mem://put/"));
    assert_eq!(prepared.method, "PUT");
    assert!(
        prepared
            .headers
            .contains(&("content-type".into(), "image/png".into()))
    );
    assert!(
        prepared
            .headers
            .contains(&("x-content-length-range".into(), "48213,48213".into()))
    );
    assert_eq!(prepared.expires_at, env.clock.now() + Duration::seconds(60));
    assert_eq!(prepared.suggested_title, "screenshot.png");
    assert!(
        prepared
            .asset_url
            .contains(&format!("/assets/{}/", env.data.org.0))
    );

    // Objekt fehlt
    assert_eq!(
        env.app
            .create_attachment_from_upload(
                &bob,
                &issue.to_string(),
                &prepared.asset_url,
                None,
                None
            )
            .await
            .unwrap_err()
            .details["reason"],
        "upload_missing"
    );
    // falsche Größe
    put(&env, &prepared.asset_url, 1);
    assert_eq!(
        env.app
            .create_attachment_from_upload(
                &bob,
                &issue.to_string(),
                &prepared.asset_url,
                None,
                None
            )
            .await
            .unwrap_err()
            .details["reason"],
        "upload_size_mismatch"
    );
    put(&env, &prepared.asset_url, 48213);
    let done = env
        .app
        .create_attachment_from_upload(
            &bob,
            &issue.to_string(),
            &prepared.asset_url,
            Some("Login page"),
            None,
        )
        .await
        .unwrap();
    let a = &done.view.attachment;
    assert_eq!(a.title, "Login page");
    assert_eq!(a.source_type.as_deref(), Some("upload"));
    assert_eq!(a.size, Some(48213));
    assert_eq!(a.content_type.as_deref(), Some("image/png"));
    assert_eq!(a.url, prepared.asset_url);
    assert!(done.view.url.starts_with("mem://get/"), "{}", done.view.url);
    assert!(done.view.url.ends_with("ttl=300"));
    assert_eq!(
        done.view.expires_at,
        Some(env.clock.now() + Duration::seconds(300))
    );
    assert_eq!(done.view.creator.as_ref().unwrap().0, env.data.bob);
    assert_eq!(events(&env, "AttachmentCreated").await.len(), 1);

    // Download-URL wird beim Lesen frisch signiert (nach Zugriffsprüfung)
    let got = env
        .app
        .get_attachment(&bob, &a.id.to_string())
        .await
        .unwrap();
    assert!(got.url.starts_with("mem://get/"));

    // zweites Finalisieren: conflict
    assert_eq!(
        kind(
            env.app
                .create_attachment_from_upload(
                    &bob,
                    &issue.to_string(),
                    &prepared.asset_url,
                    None,
                    None
                )
                .await
        ),
        ErrorKind::Conflict
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn upload_with_expired_or_foreign_reservation_fails(pool: PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, env.data.eng).await;
    let other_issue = make_issue(&env, env.data.eng).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let reason = |e: lineal_app::AppError| e.details["reason"].as_str().unwrap().to_owned();

    // abgelaufen
    let prepared = env
        .app
        .prepare_attachment_upload(&bob, prepare(issue, 10))
        .await
        .unwrap();
    put(&env, &prepared.asset_url, 10);
    *env.clock.0.lock().unwrap() += Duration::hours(2);
    let err = env
        .app
        .create_attachment_from_upload(&bob, &issue.to_string(), &prepared.asset_url, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(reason(err), "upload_expired");

    // anderes Issue
    *env.clock.0.lock().unwrap() -= Duration::hours(2);
    let p2 = env
        .app
        .prepare_attachment_upload(&bob, prepare(issue, 10))
        .await
        .unwrap();
    put(&env, &p2.asset_url, 10);
    let err = env
        .app
        .create_attachment_from_upload(&bob, &other_issue.to_string(), &p2.asset_url, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(reason(err), "upload_issue_mismatch");

    // anderer User
    let err = env
        .app
        .create_attachment_from_upload(&alice, &issue.to_string(), &p2.asset_url, None, None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(reason(err), "upload_not_owned");

    // fremder Workspace / erfundener Schlüssel / fremde Basis-URL
    let key = key_of(&p2.asset_url);
    let tail = key.split_once('/').unwrap().1;
    for bad in [
        p2.asset_url
            .replace(&env.data.org.0.to_string(), &Uuid::now_v7().to_string()),
        format!(
            "{}/assets/{}/{}",
            "https://evil.example", env.data.org.0, tail
        ),
        format!("{}/assets/{}/x/y", "http://localhost:8080", env.data.org.0),
        "not a url".to_owned(),
    ] {
        let err = env
            .app
            .create_attachment_from_upload(&bob, &issue.to_string(), &bad, None, None)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidRequest, "{bad}");
    }

    // Größe und Typ
    assert_eq!(
        kind(
            env.app
                .prepare_attachment_upload(&bob, prepare(issue, 2 * 1024 * 1024 * 1024))
                .await
        ),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        kind(
            env.app
                .prepare_attachment_upload(&bob, prepare(issue, 0))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let mut bad_type = prepare(issue, 5);
    bad_type.content_type = "image".into();
    assert_eq!(
        kind(env.app.prepare_attachment_upload(&bob, bad_type).await),
        ErrorKind::InvalidRequest
    );
    let mut bad_name = prepare(issue, 5);
    bad_name.filename = "../etc/passwd".into();
    assert_eq!(
        kind(env.app.prepare_attachment_upload(&bob, bad_name).await),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            env.app
                .prepare_attachment_upload(&bob, prepare(Uuid::now_v7(), 5))
                .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn attachment_ids_are_scoped_and_deletes_restricted(pool: PgPool) {
    let env = setup(pool).await;
    let eng = make_issue(&env, env.data.eng).await;
    let bob = ctx_for(SeedUser::Bob);
    let alice = ctx_for(SeedUser::Alice); // Workspace-Owner
    let carol = ctx_for(SeedUser::Carol); // Gast ohne ENG
    let att = env
        .app
        .link_attachment(
            &alice,
            LinkAttachmentInput {
                issue: eng.to_string(),
                url: "https://example.com/a".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = att.view.attachment.id.to_string();
    // Gast ohne Team-Zugriff: nur not_found, auch bei erratener ID
    assert_eq!(
        kind(env.app.get_attachment(&carol, &id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(env.app.delete_attachment(&carol, &id).await),
        ErrorKind::NotFound
    );
    // Fremde Organisation: ID unbekannt
    let mut foreign = ctx_for(SeedUser::Alice);
    foreign.org_id = lineal_domain::ids::OrgId(Uuid::now_v7());
    assert_eq!(
        kind(env.app.get_attachment(&foreign, &id).await),
        ErrorKind::NotFound
    );
    // Nicht-Ersteller ohne Owner-/Admin-Rolle darf nicht löschen, Admin schon
    assert_eq!(
        kind(env.app.delete_attachment(&bob, &id).await),
        ErrorKind::Forbidden
    );
    env.app.delete_attachment(&alice, &id).await.unwrap();
    // Ersteller darf eigenen Anhang löschen
    let own = env
        .app
        .link_attachment(
            &bob,
            LinkAttachmentInput {
                issue: eng.to_string(),
                url: "https://example.com/b".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    env.app
        .delete_attachment(&bob, &own.view.attachment.id.to_string())
        .await
        .unwrap();
}
