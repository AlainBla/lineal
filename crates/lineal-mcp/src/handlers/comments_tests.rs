//! Contract-Tests für die Gruppe `comments` gegen echte Datenbank (Seed `acme`).

use crate::dispatch::dispatch;
use crate::envelope::tool_result;
use crate::validate::validate_args;
use async_trait::async_trait;
use lineal_app::ports::services::BlobInfo;
use lineal_app::ports::{BlobStore, Store};
use lineal_app::{App, AppResult, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::ids::TeamId;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct MemBlobs(pub(crate) Mutex<HashMap<String, (u64, String)>>);

#[async_trait]
impl BlobStore for MemBlobs {
    async fn signed_put_url(&self, key: &str, _ct: &str, _ttl: Duration) -> AppResult<String> {
        Ok(format!("mem://put/{key}"))
    }
    async fn signed_get_url(&self, key: &str, ttl: Duration) -> AppResult<String> {
        Ok(format!("mem://get/{key}?ttl={}", ttl.as_secs()))
    }
    async fn head(&self, key: &str) -> AppResult<Option<BlobInfo>> {
        Ok(self.0.lock().unwrap().get(key).map(|(size, ct)| BlobInfo {
            size: *size,
            content_type: ct.clone(),
        }))
    }
}

pub(crate) struct Env {
    pub app: App,
    pub pool: sqlx::PgPool,
    pub blobs: Arc<MemBlobs>,
}

pub(crate) async fn setup(pool: sqlx::PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    let blobs = Arc::new(MemBlobs::default());
    app.blobs = blobs.clone();
    Env { app, pool, blobs }
}

/// Legt ein Issue im Team per SQL an; liefert UUID.
pub(crate) async fn make_issue(env: &Env, team: TeamId) -> Uuid {
    let data = SeedData::acme();
    let slot = if team == data.eng {
        1
    } else if team == data.web {
        2
    } else {
        3
    };
    let id = Uuid::now_v7();
    sqlx::query(
        "WITH n AS (UPDATE team SET last_issue_number = last_issue_number + 1 \
         WHERE id = $1 RETURNING last_issue_number) \
         INSERT INTO issue (id, organization_id, team_id, number, title, state_id) \
         SELECT $2, $3, $1, last_issue_number, 'Login broken', $4 FROM n",
    )
    .bind(team.0)
    .bind(id)
    .bind(data.org.0)
    .bind(seed_workflow_state_ids(slot)[0].0)
    .execute(&env.pool)
    .await
    .unwrap();
    id
}

/// Wie der Transport: erst Schema-Validierung, dann Handler.
pub(crate) async fn call(
    env: &Env,
    who: SeedUser,
    tool: &str,
    args: Value,
) -> Result<Value, lineal_app::AppError> {
    validate_args(tool, &args)?;
    dispatch(&env.app, &ctx_for(who), tool, args)
        .await
        .expect("known tool")
}

fn err_kind(r: Result<Value, lineal_app::AppError>) -> ErrorKind {
    r.expect_err("expected an error").kind
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn save_list_and_delete_comments(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let eng = SeedData::acme().eng;
    let issue = make_issue(&env, eng).await;
    let ident: String = sqlx::query_scalar("SELECT identifier FROM issue WHERE id = $1")
        .bind(issue)
        .fetch_one(&env.pool)
        .await
        .unwrap();

    // Thread eröffnen (per Kennung), Antwort über parentId (Eltern-Felder werden ignoriert)
    let root = call(
        &env,
        SeedUser::Alice,
        "save_comment",
        json!({ "issueId": ident, "body": "Reproduced. @bob can you check <script>alert(1)</script>?" }),
    )
    .await
    .unwrap();
    assert!(!root["body"].as_str().unwrap().contains("<script"));
    assert_eq!(root["parentId"], Value::Null);
    assert_eq!(root["quotedText"], Value::Null);
    assert_eq!(root["onBehalfOf"], Value::Null);
    assert_eq!(root["author"]["name"], "Alice Example");
    assert_eq!(root["editedAt"], Value::Null);
    assert_eq!(root["attachments"], json!([]));
    assert_eq!(root["reactions"], json!([]));
    assert!(
        root["url"].as_str().unwrap().ends_with(&format!(
            "/issue/{ident}#comment-{}",
            root["id"].as_str().unwrap()
        )),
        "{}",
        root["url"]
    );
    let root_id = root["id"].as_str().unwrap().to_owned();
    let reply = call(
        &env,
        SeedUser::Bob,
        "save_comment",
        json!({ "parentId": root_id, "body": "Looking" }),
    )
    .await
    .unwrap();
    assert_eq!(reply["parentId"], root_id);
    let reply2 = call(
        &env,
        SeedUser::Alice,
        "save_comment",
        json!({ "parentId": reply["id"], "body": "Thanks" }),
    )
    .await
    .unwrap();
    assert_eq!(reply2["parentId"], root_id, "normalized to the root");

    // Liste: flach, Wurzeln und Antworten, Paginierung mit Cursor
    let list = call(
        &env,
        SeedUser::Bob,
        "list_comments",
        json!({ "issueId": ident, "limit": 2, "orderBy": "createdAt" }),
    )
    .await
    .unwrap();
    assert_eq!(list["comments"].as_array().unwrap().len(), 2);
    assert_eq!(list["hasNextPage"], true);
    // neueste zuerst
    assert_eq!(list["comments"][0]["id"], reply2["id"]);
    let cursor = list["cursor"].as_str().unwrap().to_owned();
    let page2 = call(
        &env,
        SeedUser::Bob,
        "list_comments",
        json!({ "issueId": ident, "limit": 2, "orderBy": "createdAt", "cursor": cursor }),
    )
    .await
    .unwrap();
    assert_eq!(page2["comments"].as_array().unwrap().len(), 1);
    assert_eq!(page2["comments"][0]["id"], root_id);
    assert_eq!(page2["hasNextPage"], false);
    assert!(page2.get("cursor").is_none());
    // Cursor gehört zu den Filtern
    assert_eq!(
        err_kind(
            call(
                &env,
                SeedUser::Bob,
                "list_comments",
                json!({ "issueId": issue.to_string(), "limit": 2, "orderBy": "createdAt", "cursor": cursor }),
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );

    // Update: nur Autor, setzt editedAt
    let id = root["id"].clone();
    assert_eq!(
        err_kind(
            call(
                &env,
                SeedUser::Bob,
                "save_comment",
                json!({ "id": id, "body": "mine now" })
            )
            .await
        ),
        ErrorKind::Forbidden
    );
    let edited = call(
        &env,
        SeedUser::Alice,
        "save_comment",
        json!({ "id": id, "body": "Edited text" }),
    )
    .await
    .unwrap();
    assert_eq!(edited["body"], "Edited text");
    assert!(edited["editedAt"].is_string());

    // Delete: Antwort löschbar, Antwort `{success,id}`; Wurzel mit Antworten wird Platzhalter
    let del = call(
        &env,
        SeedUser::Bob,
        "delete_comment",
        json!({ "id": reply["id"] }),
    )
    .await
    .unwrap();
    assert_eq!(del, json!({ "success": true, "id": reply["id"] }));
    call(&env, SeedUser::Alice, "delete_comment", json!({ "id": id }))
        .await
        .unwrap();
    let after = call(
        &env,
        SeedUser::Alice,
        "list_comments",
        json!({ "issueId": ident }),
    )
    .await
    .unwrap();
    let bodies: Vec<_> = after["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        bodies.contains(&"Kommentar gelöscht".to_string()),
        "{bodies:?}"
    );
    assert!(bodies.contains(&"Thanks".to_string()));
    assert_eq!(bodies.len(), 2);

    // Ergebnis wird als Text-Content verpackt (kein _mcpContent)
    assert_eq!(tool_result(del)["isError"], false);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn argument_rules_and_error_kinds(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, SeedData::acme().eng).await.to_string();
    let k = |r| err_kind(r);

    // Schema: body ist Pflicht
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "save_comment",
            json!({ "issueId": issue })
        )
        .await),
        ErrorKind::InvalidRequest
    );
    // kein Parent / mehrere Parents / leerer Body
    assert_eq!(
        k(call(&env, SeedUser::Bob, "save_comment", json!({ "body": "x" })).await),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "save_comment",
            json!({ "body": "x", "issueId": issue, "projectId": "P-ENG-1" })
        )
        .await),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "save_comment",
            json!({ "body": "  ", "issueId": issue })
        )
        .await),
        ErrorKind::InvalidRequest
    );
    // parentId + id
    assert_eq!(
        k(call(&env, SeedUser::Bob, "save_comment", json!({ "body": "x", "id": Uuid::now_v7().to_string(), "parentId": Uuid::now_v7().to_string() })).await),
        ErrorKind::InvalidRequest
    );
    // statusUpdateType ohne statusUpdateId bzw. mit parentId/id
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "list_comments",
            json!({ "issueId": issue, "statusUpdateType": "project" })
        )
        .await),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        k(call(&env, SeedUser::Bob, "save_comment", json!({ "body": "x", "parentId": Uuid::now_v7().to_string(), "statusUpdateType": "project" })).await),
        ErrorKind::InvalidRequest
    );
    // list ohne bzw. mit mehreren Parents
    assert_eq!(
        k(call(&env, SeedUser::Bob, "list_comments", json!({})).await),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "list_comments",
            json!({ "issueId": issue, "documentId": "abc" })
        )
        .await),
        ErrorKind::InvalidRequest
    );
    // unbekannte Objekte
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "list_comments",
            json!({ "issueId": "ENG-99999" })
        )
        .await),
        ErrorKind::NotFound
    );
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "save_comment",
            json!({ "body": "x", "parentId": Uuid::now_v7().to_string() })
        )
        .await),
        ErrorKind::NotFound
    );
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "delete_comment",
            json!({ "id": Uuid::now_v7().to_string() })
        )
        .await),
        ErrorKind::NotFound
    );
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "delete_comment",
            json!({ "id": "garbage" })
        )
        .await),
        ErrorKind::NotFound
    );
    assert_eq!(
        k(call(
            &env,
            SeedUser::Bob,
            "list_comments",
            json!({ "documentId": "no-such-doc" })
        )
        .await),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_team_is_not_found_and_inline_roots_stay(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let data = SeedData::acme();
    let des_issue = make_issue(&env, data.des).await.to_string();
    // Nicht-Mitglied (Alice, Carol) → not_found; Mitglied (Bob) → ok
    let c = call(
        &env,
        SeedUser::Bob,
        "save_comment",
        json!({ "issueId": des_issue, "body": "private" }),
    )
    .await
    .unwrap();
    for who in [SeedUser::Alice, SeedUser::Carol] {
        assert_eq!(
            err_kind(
                call(
                    &env,
                    who,
                    "save_comment",
                    json!({ "issueId": des_issue, "body": "x" })
                )
                .await
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            err_kind(call(&env, who, "list_comments", json!({ "issueId": des_issue })).await),
            ErrorKind::NotFound
        );
        assert_eq!(
            err_kind(call(&env, who, "delete_comment", json!({ "id": c["id"] })).await),
            ErrorKind::NotFound
        );
    }
    // Lese-Scope: Schreiben verboten
    let mut ro = ctx_for(SeedUser::Bob);
    ro.scopes = lineal_app::Scopes::read_only();
    let r = dispatch(
        &env.app,
        &ro,
        "save_comment",
        json!({ "issueId": des_issue, "body": "x" }),
    )
    .await
    .unwrap();
    assert_eq!(r.unwrap_err().kind, ErrorKind::Forbidden);
    let r = dispatch(
        &env.app,
        &ro,
        "list_comments",
        json!({ "issueId": des_issue }),
    )
    .await
    .unwrap();
    assert_eq!(r.unwrap()["comments"].as_array().unwrap().len(), 1);

    // Inline-Wurzel (quotedText per SQL, MCP kann sie nur lesen/beantworten)
    let eng_issue = make_issue(&env, data.eng).await;
    let inline = call(
        &env,
        SeedUser::Alice,
        "save_comment",
        json!({ "issueId": eng_issue.to_string(), "body": "Why 30?" }),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE comment SET quoted_text = 'timeout = 30' WHERE id = $1")
        .bind(inline["id"].as_str().unwrap().parse::<Uuid>().unwrap())
        .execute(&env.pool)
        .await
        .unwrap();
    let list = call(
        &env,
        SeedUser::Bob,
        "list_comments",
        json!({ "issueId": eng_issue.to_string() }),
    )
    .await
    .unwrap();
    assert_eq!(list["comments"][0]["quotedText"], "timeout = 30");
    let err = call(
        &env,
        SeedUser::Alice,
        "delete_comment",
        json!({ "id": inline["id"] }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert!(
        err.message
            .starts_with("Inline comment roots cannot be deleted")
    );
    let reply = call(
        &env,
        SeedUser::Bob,
        "save_comment",
        json!({ "parentId": inline["id"], "body": "Because" }),
    )
    .await
    .unwrap();
    assert_eq!(reply["quotedText"], Value::Null);
    call(
        &env,
        SeedUser::Bob,
        "delete_comment",
        json!({ "id": reply["id"] }),
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn mentions_land_in_the_outbox_only_with_access(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let issue = make_issue(&env, SeedData::acme().eng).await.to_string();
    // Carol (Gast, nur WEB) hat keinen Zugriff auf ENG
    call(
        &env,
        SeedUser::Alice,
        "save_comment",
        json!({ "issueId": issue, "body": "cc @bob @carol" }),
    )
    .await
    .unwrap();
    let payload: Value =
        sqlx::query_scalar("SELECT payload FROM outbox_event WHERE type = 'CommentCreated'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(
        payload["mentioned_user_ids"],
        json!([SeedData::acme().bob.0.to_string()])
    );
}

/// Freigabe (docs/07 §3.17): der Empfänger (Gast Carol, nur WEB) liest und kommentiert das geteilte
/// ENG-Issue samt Anhängen, sieht aber weder andere ENG-Issues noch deren Kommentare; nach dem
/// Entzug ist alles wieder `not_found`.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn share_recipient_reads_and_comments_only_on_the_shared_issue(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let data = SeedData::acme();
    sqlx::query(
        "UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": true}'::jsonb WHERE id = $1",
    )
    .bind(data.org.0)
    .execute(&env.pool)
    .await
    .unwrap();
    let shared = make_issue(&env, data.eng).await;
    let other = make_issue(&env, data.eng).await;
    let mut idents = vec![];
    for id in [shared, other] {
        let ident: String = sqlx::query_scalar("SELECT identifier FROM issue WHERE id = $1")
            .bind(id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
        idents.push(ident);
    }
    let (s_id, o_id) = (idents[0].clone(), idents[1].clone());
    let mut comment_ids = vec![];
    for id in [&s_id, &o_id] {
        let c = call(
            &env,
            SeedUser::Alice,
            "save_comment",
            json!({ "issueId": id, "body": "Internal note" }),
        )
        .await
        .unwrap();
        comment_ids.push(c["id"].as_str().unwrap().to_owned());
    }
    let att = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO attachment (id, organization_id, issue_id, url, title, source_type, creator_id) \
         VALUES ($1,$2,$3,'https://example.com/spec','Spec','url',$4)",
    )
    .bind(att)
    .bind(data.org.0)
    .bind(shared)
    .bind(data.alice.0)
    .execute(&env.pool)
    .await
    .unwrap();

    // Vor der Freigabe: nichts sichtbar.
    for id in [&s_id, &o_id] {
        assert_eq!(
            err_kind(
                call(
                    &env,
                    SeedUser::Carol,
                    "list_comments",
                    json!({ "issueId": id })
                )
                .await
            ),
            ErrorKind::NotFound
        );
    }

    sqlx::query(
        "INSERT INTO issue_share (organization_id, issue_id, user_id, shared_by_id) VALUES ($1,$2,$3,$4)",
    )
    .bind(data.org.0)
    .bind(shared)
    .bind(data.carol.0)
    .bind(data.bob.0)
    .execute(&env.pool)
    .await
    .unwrap();

    let list = call(
        &env,
        SeedUser::Carol,
        "list_comments",
        json!({ "issueId": s_id }),
    )
    .await
    .unwrap();
    assert_eq!(list["comments"].as_array().unwrap().len(), 1);
    let got = call(&env, SeedUser::Carol, "get_issue", json!({ "id": s_id }))
        .await
        .unwrap();
    assert_eq!(got["id"], s_id);
    let a = call(
        &env,
        SeedUser::Carol,
        "get_attachment",
        json!({ "id": att.to_string() }),
    )
    .await
    .unwrap();
    assert_eq!(a["title"], "Spec");
    // Kommentieren (neuer Kommentar und Antwort) ist erlaubt.
    let mine = call(
        &env,
        SeedUser::Carol,
        "save_comment",
        json!({ "issueId": s_id, "body": "Guest here" }),
    )
    .await
    .unwrap();
    assert_eq!(mine["author"]["name"], "Carol Example");
    call(
        &env,
        SeedUser::Carol,
        "save_comment",
        json!({ "parentId": comment_ids[0], "body": "Reply" }),
    )
    .await
    .unwrap();

    // Das andere ENG-Issue und sein Kommentar bleiben unsichtbar, auch schreibend.
    for r in [
        call(
            &env,
            SeedUser::Carol,
            "list_comments",
            json!({ "issueId": o_id }),
        )
        .await,
        call(
            &env,
            SeedUser::Carol,
            "save_comment",
            json!({ "issueId": o_id, "body": "x" }),
        )
        .await,
        call(
            &env,
            SeedUser::Carol,
            "save_comment",
            json!({ "parentId": comment_ids[1], "body": "x" }),
        )
        .await,
        call(&env, SeedUser::Carol, "get_issue", json!({ "id": o_id })).await,
    ] {
        assert_eq!(err_kind(r), ErrorKind::NotFound);
    }

    // Entzug: wieder `not_found`.
    sqlx::query("DELETE FROM issue_share WHERE issue_id = $1")
        .bind(shared)
        .execute(&env.pool)
        .await
        .unwrap();
    for r in [
        call(
            &env,
            SeedUser::Carol,
            "list_comments",
            json!({ "issueId": s_id }),
        )
        .await,
        call(
            &env,
            SeedUser::Carol,
            "save_comment",
            json!({ "issueId": s_id, "body": "x" }),
        )
        .await,
        call(
            &env,
            SeedUser::Carol,
            "get_attachment",
            json!({ "id": att.to_string() }),
        )
        .await,
    ] {
        assert_eq!(err_kind(r), ErrorKind::NotFound);
    }
}

/// Freigaben gelten nicht für team-beschränkte API-Keys und nicht, wenn der Workspace Freigaben
/// abgeschaltet hat (zentrale Regel `resolve::issue::share_recipient`).
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn share_does_not_escape_team_scope_or_disabled_sharing(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let data = SeedData::acme();
    sqlx::query(
        "UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": true}'::jsonb WHERE id = $1",
    )
    .bind(data.org.0)
    .execute(&env.pool)
    .await
    .unwrap();
    let shared = make_issue(&env, data.eng).await;
    let ident: String = sqlx::query_scalar("SELECT identifier FROM issue WHERE id = $1")
        .bind(shared)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO issue_share (organization_id, issue_id, user_id, shared_by_id) VALUES ($1,$2,$3,$4)",
    )
    .bind(data.org.0)
    .bind(shared)
    .bind(data.carol.0)
    .bind(data.bob.0)
    .execute(&env.pool)
    .await
    .unwrap();
    // Gegenprobe: mit normaler Session sieht Carol das geteilte Issue.
    call(&env, SeedUser::Carol, "get_issue", json!({ "id": ident }))
        .await
        .unwrap();

    // Team-beschränkter Key (nur WEB): kein Zugriff über die Freigabe.
    let mut scoped = ctx_for(SeedUser::Carol);
    scoped.team_scope = Some(vec![data.web]);
    for (tool, args) in [
        ("get_issue", json!({ "id": ident })),
        ("list_comments", json!({ "issueId": ident })),
    ] {
        let r = dispatch(&env.app, &scoped, tool, args)
            .await
            .expect("known tool");
        assert_eq!(err_kind(r), ErrorKind::NotFound, "{tool}");
    }

    // Freigaben im Workspace abgeschaltet: bestehende Freigaben wirken nicht mehr.
    sqlx::query(
        "UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": false}'::jsonb WHERE id = $1",
    )
    .bind(data.org.0)
    .execute(&env.pool)
    .await
    .unwrap();
    for (tool, args) in [
        ("get_issue", json!({ "id": ident })),
        ("list_comments", json!({ "issueId": ident })),
    ] {
        let r = call(&env, SeedUser::Carol, tool, args).await;
        assert_eq!(err_kind(r), ErrorKind::NotFound, "{tool}");
    }
}
