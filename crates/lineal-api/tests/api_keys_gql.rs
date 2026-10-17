#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `apiKeys`, `apiKeyCreate`, `apiKeyRevoke` (WP19).

use async_graphql::{Request, Variables};
use lineal_api::graphql::{LinealSchema, build_schema};
use lineal_api::middleware::auth::CredentialClass;
use lineal_app::RequestCtx;
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;

async fn setup(pool: sqlx::PgPool) -> LinealSchema {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    build_schema(test_app(Arc::new(store)))
}

async fn run(
    schema: &LinealSchema,
    ctx: RequestCtx,
    class: CredentialClass,
    query: &str,
    vars: Value,
) -> async_graphql::Response {
    schema
        .execute(
            Request::new(query)
                .variables(Variables::from_json(vars))
                .data(ctx)
                .data(class),
        )
        .await
}

const CREATE: &str = "mutation($i: ApiKeyCreateInput!) { apiKeyCreate(input: $i) { success token apiKey { id label keyPrefix scopes teams { id key name } expiresAt revokedAt } } }";

fn code(res: &async_graphql::Response) -> String {
    let e = res.errors.first().expect("expected error");
    let v = serde_json::to_value(e).unwrap();
    v["extensions"]["code"].as_str().unwrap_or("").to_owned()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn create_list_and_revoke_roundtrip(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        CREATE,
        json!({"i": {"label": "CI", "scopes": ["READ", "WRITE"], "teamIds": ["ENG"]}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let p = &v["apiKeyCreate"];
    let token = p["token"].as_str().unwrap();
    assert!(token.starts_with("lin_api_"));
    assert_eq!(p["apiKey"]["scopes"], json!(["read", "write"]));
    assert_eq!(p["apiKey"]["teams"][0]["key"], "ENG");
    assert_eq!(p["apiKey"]["keyPrefix"], token[..12]);
    let id = p["apiKey"]["id"].as_str().unwrap().to_owned();

    // Liste: kein Klartext, kein Hash.
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        "{ apiKeys { id label keyPrefix scopes teams { key } createdAt lastUsedAt expiresAt revokedAt } }",
        json!({}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let body = serde_json::to_string(&res.data.into_json().unwrap()).unwrap();
    assert!(body.contains("\"CI\""));
    assert!(!body.contains(token));
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        "{ apiKeys { keyHash } }",
        json!({}),
    )
    .await;
    assert!(!res.errors.is_empty());

    // Alice sieht Bobs Key nicht und kann ihn nicht widerrufen (auch als Owner).
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        "{ apiKeys { id } }",
        json!({}),
    )
    .await;
    assert_eq!(res.data.into_json().unwrap()["apiKeys"], json!([]));
    let rev = "mutation($id: String!) { apiKeyRevoke(id: $id) { success } }";
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        rev,
        json!({"id": id}),
    )
    .await;
    assert_eq!(code(&res), "not_found");

    // Bob widerruft.
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        rev,
        json!({"id": id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        "{ apiKeys { revokedAt } }",
        json!({}),
    )
    .await;
    let v = res.data.into_json().unwrap();
    assert!(v["apiKeys"][0]["revokedAt"].is_string());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn only_cookie_session_may_create(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    for class in [CredentialClass::ApiKey, CredentialClass::OAuth] {
        let res = run(
            &schema,
            ctx_for(SeedUser::Bob),
            class,
            CREATE,
            json!({"i": {"label": "x", "scopes": ["READ"]}}),
        )
        .await;
        assert_eq!(code(&res), "forbidden");
    }
    // Ohne Klassen-Angabe (Ausführung am Router vorbei) ebenfalls abgelehnt.
    let res = schema
        .execute(
            Request::new(CREATE)
                .variables(Variables::from_json(
                    json!({"i": {"label": "x", "scopes": ["READ"]}}),
                ))
                .data(ctx_for(SeedUser::Bob)),
        )
        .await;
    assert_eq!(code(&res), "forbidden");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn validation_scope_teams_and_limits(pool: sqlx::PgPool) {
    let schema = setup(pool.clone()).await;
    // Bob aus dem privaten Design-Team entfernen -> DES ist für ihn unsichtbar.
    let data = SeedData::acme();
    sqlx::query("DELETE FROM team_membership WHERE team_id = $1 AND user_id = $2")
        .bind(data.des.0)
        .bind(data.bob.0)
        .execute(&pool)
        .await
        .unwrap();
    let create = |input: Value| {
        let schema = &schema;
        async move {
            run(
                schema,
                ctx_for(SeedUser::Bob),
                CredentialClass::Session,
                CREATE,
                json!({ "i": input }),
            )
            .await
        }
    };
    // Unsichtbares Team (privat) -> not_found.
    let res = create(json!({"label": "x", "scopes": ["READ"], "teamIds": ["DES"]})).await;
    assert_eq!(code(&res), "not_found");
    // Label leer / zu lang.
    let res = create(json!({"label": "  ", "scopes": ["READ"]})).await;
    assert_eq!(code(&res), "invalid_request");
    let res = create(json!({"label": "a".repeat(101), "scopes": ["READ"]})).await;
    assert_eq!(code(&res), "invalid_request");
    // Ablaufdatum in der Vergangenheit / ungültig.
    let res =
        create(json!({"label": "x", "scopes": ["READ"], "expiresAt": "2001-01-01T00:00:00Z"}))
            .await;
    assert_eq!(code(&res), "invalid_request");
    let res = create(json!({"label": "x", "scopes": ["READ"], "expiresAt": "morgen"})).await;
    assert_eq!(code(&res), "invalid_request");
    // Admin-Scope gibt es in der API nicht.
    let res = create(json!({"label": "x", "scopes": ["ADMIN"]})).await;
    assert!(!res.errors.is_empty());
    // Gast darf keine Keys anlegen.
    let res = run(
        &schema,
        ctx_for(SeedUser::Carol),
        CredentialClass::Session,
        CREATE,
        json!({"i": {"label": "x", "scopes": ["READ"]}}),
    )
    .await;
    assert_eq!(code(&res), "forbidden");
    // Lese-Scope des Aufrufers reicht nicht (Mutation braucht write).
    let mut ro = ctx_for(SeedUser::Bob);
    ro.scopes = lineal_app::Scopes::read_only();
    let res = run(
        &schema,
        ro,
        CredentialClass::Session,
        CREATE,
        json!({"i": {"label": "x", "scopes": ["READ"]}}),
    )
    .await;
    assert_eq!(code(&res), "forbidden");

    // Limit aktiver Keys: 25 erlaubt, der 26. scheitert; Widerruf macht Platz.
    let mut first_id = String::new();
    for n in 0..25 {
        let res = create(json!({"label": format!("k{n}"), "scopes": ["READ"]})).await;
        assert!(res.errors.is_empty(), "{n}: {:?}", res.errors);
        if n == 0 {
            first_id = res.data.into_json().unwrap()["apiKeyCreate"]["apiKey"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
        }
    }
    let res = create(json!({"label": "k25", "scopes": ["READ"]})).await;
    assert_eq!(code(&res), "limit_exceeded");
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        "mutation($id: String!) { apiKeyRevoke(id: $id) { success } }",
        json!({"id": first_id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let res = create(json!({"label": "k25", "scopes": ["READ"]})).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);

    // Ungültige / unbekannte ID.
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        "mutation { apiKeyRevoke(id: \"nope\") { success } }",
        json!({}),
    )
    .await;
    assert_eq!(code(&res), "not_found");
}

/// Ein Key im Session-Cookie ist kein Session-Login: kein 200 und kein neuer Key (Eskalation
/// von einem team-beschränkten zu einem unbeschränkten Key).
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn api_key_in_session_cookie_cannot_mint_keys(pool: sqlx::PgPool) {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use lineal_app::usecases::auth::CreateApiKeyInput;
    use tower::ServiceExt;

    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    // Team-beschränkter Key (nur ENG), angelegt aus einer Session.
    let created = app
        .create_api_key(
            &ctx_for(SeedUser::Bob),
            CreateApiKeyInput {
                label: "scoped".into(),
                scopes: vec!["read".into(), "write".into()],
                teams: Some(vec!["ENG".into()]),
                expires_at: None,
            },
        )
        .await
        .unwrap();
    let router = lineal_api::router(app.clone(), false);
    let body = json!({ "query": "mutation { apiKeyCreate(input: { label: \"evil\", scopes: [READ, WRITE] }) { token } }" });
    let req = |name: &str, value: String| {
        Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .header("origin", "http://localhost:8080")
            .header("x-lineal-csrf", "1")
            .header(name, value)
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    // Als Cookie: 401 (Token-Art passt nicht zum Transport).
    let res = router
        .clone()
        .oneshot(req("cookie", format!("lineal_session={}", created.key)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    // Als Authorization-Header: Anmeldung gültig, aber Anlegen verboten.
    let res = router
        .oneshot(req("authorization", created.key.clone()))
        .await
        .unwrap();
    let bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("forbidden"), "{text}");
    assert!(!text.contains("lin_api_"), "{text}");
    let n = app
        .list_api_keys(&ctx_for(SeedUser::Bob))
        .await
        .unwrap()
        .len();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn use_case_requires_session_and_keeps_team_scope(pool: sqlx::PgPool) {
    use lineal_app::usecases::auth::CreateApiKeyInput;

    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    let input = |teams: Option<Vec<&str>>| CreateApiKeyInput {
        label: "k".into(),
        scopes: vec!["read".into()],
        teams: teams.map(|t| t.into_iter().map(String::from).collect()),
        expires_at: None,
    };
    // Kein Session-Kontext (API-Key/OAuth) -> forbidden.
    let mut not_session = ctx_for(SeedUser::Bob);
    not_session.via_session = false;
    let err = app
        .create_api_key(&not_session, input(None))
        .await
        .unwrap_err();
    assert_eq!(err.kind, lineal_app::ErrorKind::Forbidden);

    // Team-beschränkter Kontext: ohne Angabe erbt der Key die Beschränkung ...
    let mut scoped = ctx_for(SeedUser::Bob);
    scoped.team_scope = Some(vec![data.eng]);
    let k = app.create_api_key(&scoped, input(None)).await.unwrap();
    assert_eq!(k.record.team_ids, Some(vec![data.eng]));
    // ... eine Teilmenge ist erlaubt, ein fremdes Team nicht.
    let k = app
        .create_api_key(&scoped, input(Some(vec!["ENG"])))
        .await
        .unwrap();
    assert_eq!(k.record.team_ids, Some(vec![data.eng]));
    let err = app
        .create_api_key(&scoped, input(Some(vec!["WEB"])))
        .await
        .unwrap_err();
    // Außerhalb des Scopes unsichtbar (not_found) oder verboten – nie erlaubt.
    assert!(matches!(
        err.kind,
        lineal_app::ErrorKind::Forbidden | lineal_app::ErrorKind::NotFound
    ));
}

/// F-31: Auflisten und Widerrufen von Keys nur per Web-Session, nie mit API-Key/OAuth-Token.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn listing_and_revoking_keys_needs_a_session(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        CREATE,
        json!({"i": {"label": "CI", "scopes": ["READ", "WRITE"]}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let id = res.data.into_json().unwrap()["apiKeyCreate"]["apiKey"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let rev = "mutation($id: String!) { apiKeyRevoke(id: $id) { success } }";
    for class in [CredentialClass::ApiKey, CredentialClass::OAuth] {
        let res = run(
            &schema,
            ctx_for(SeedUser::Bob),
            class,
            "{ apiKeys { id } }",
            json!({}),
        )
        .await;
        assert_eq!(code(&res), "forbidden");
        let res = run(
            &schema,
            ctx_for(SeedUser::Bob),
            class,
            rev,
            json!({"id": id}),
        )
        .await;
        assert_eq!(code(&res), "forbidden");
    }
    // Auch das Use-Case-Gate greift (Klasse Session, aber ctx.via_session = false).
    let mut key_ctx = ctx_for(SeedUser::Bob);
    key_ctx.via_session = false;
    let res = run(
        &schema,
        key_ctx.clone(),
        CredentialClass::Session,
        "{ apiKeys { id } }",
        json!({}),
    )
    .await;
    assert_eq!(code(&res), "forbidden");
    let res = run(
        &schema,
        key_ctx,
        CredentialClass::Session,
        rev,
        json!({"id": id}),
    )
    .await;
    assert_eq!(code(&res), "forbidden");
    // Die Session darf weiterhin widerrufen.
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        CredentialClass::Session,
        rev,
        json!({"id": id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}
