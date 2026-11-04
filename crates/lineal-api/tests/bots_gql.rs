#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `bots`, `botCreate`, `botApiKeyCreate`, `botApiKeyRevoke` (LIN-22).

use async_graphql::{Request, Variables};
use lineal_api::graphql::build_schema;
use lineal_api::middleware::auth::CredentialClass;
use lineal_app::ports::Store;
use lineal_app::{Actor, App, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;

async fn setup(pool: sqlx::PgPool) -> Arc<App> {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    test_app(Arc::new(store))
}

async fn run(
    app: &Arc<App>,
    ctx: RequestCtx,
    class: CredentialClass,
    query: &str,
    vars: Value,
) -> async_graphql::Response {
    build_schema(app.clone())
        .execute(
            Request::new(query)
                .variables(Variables::from_json(vars))
                .data(ctx)
                .data(class),
        )
        .await
}

fn code(res: &async_graphql::Response) -> String {
    let e = res.errors.first().expect("expected error");
    let v = serde_json::to_value(e).unwrap();
    v["extensions"]["code"].as_str().unwrap_or("").to_owned()
}

const CREATE_BOT: &str = "mutation($i: BotCreateInput!) { botCreate(input: $i) { success bot { id name displayName email active apiKeys { id } } } }";
const CREATE_KEY: &str = "mutation($b: String!, $i: ApiKeyCreateInput!) { botApiKeyCreate(botId: $b, input: $i) { success token apiKey { id scopes teams { key } } } }";
const REVOKE_KEY: &str =
    "mutation($b: String!, $id: String!) { botApiKeyRevoke(botId: $b, id: $id) { success } }";
const LIST: &str = "{ bots { id name displayName active apiKeys { id label scopes revokedAt } } }";

async fn create_bot(app: &Arc<App>) -> String {
    let res = run(
        app,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        CREATE_BOT,
        json!({"i": {"name": "Lineal Bot", "email": "lineal-bot@acme.example"}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let bot = &v["botCreate"]["bot"];
    assert_eq!(bot["displayName"], "lineal-bot");
    assert_eq!(bot["active"], true);
    bot["id"].as_str().unwrap().to_owned()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn admin_creates_bot_mints_key_and_key_acts_as_app(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let bot = create_bot(&app).await;

    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        CREATE_KEY,
        json!({"b": bot, "i": {"label": "CI", "scopes": ["READ", "WRITE"], "teamIds": ["ENG"]}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let token = v["botApiKeyCreate"]["token"].as_str().unwrap().to_owned();
    let key_id = v["botApiKeyCreate"]["apiKey"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        v["botApiKeyCreate"]["apiKey"]["scopes"],
        json!(["read", "write"])
    );
    assert_eq!(v["botApiKeyCreate"]["apiKey"]["teams"][0]["key"], "ENG");

    // Der Key gehört dem Bot und handelt als App (nie mit admin-Scope).
    let principal = app.principal_for_token(&token).await.unwrap();
    match principal.actor {
        Actor::App { id } => assert_eq!(id.to_string(), bot),
        other => panic!("expected app actor, got {other:?}"),
    }
    assert!(principal.scopes.write);
    assert!(!principal.scopes.admin);

    // Liste: Bot mit Key, ohne Klartext; normale User erscheinen nicht.
    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        LIST,
        json!({}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(v["bots"].as_array().unwrap().len(), 1);
    assert_eq!(v["bots"][0]["apiKeys"][0]["label"], "CI");
    assert!(!serde_json::to_string(&v).unwrap().contains(&token));

    // Widerruf über den Bot; danach ist der Key ungültig.
    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        REVOKE_KEY,
        json!({"b": bot, "id": key_id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert!(app.principal_for_token(&token).await.is_err());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bots_cannot_log_in_and_get_no_email(pool: sqlx::PgPool) {
    let store = PgStore::new(pool.clone());
    let app = setup(pool).await;
    let bot = create_bot(&app).await;
    let bot_id = lineal_domain::ids::UserId(uuid::Uuid::parse_str(&bot).unwrap());
    let bot_ctx = RequestCtx {
        actor: Actor::App { id: bot_id },
        ..ctx_for(SeedUser::Alice)
    };
    let settings = app.get_notification_settings(&bot_ctx).await.unwrap();
    assert!(!settings.email_enabled);

    // Selbst mit gesetztem Passwort kein Browser-Login.
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    tx.users()
        .set_password_hash(bot_id, "fake:secret-password", chrono::Utc::now())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let res = app
        .login(lineal_app::usecases::auth::LoginInput {
            email: "lineal-bot@acme.example".into(),
            password: "secret-password".into(),
            ..Default::default()
        })
        .await;
    assert!(res.is_err());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn only_admins_with_a_session_manage_bots(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    // Normale User und Gäste nicht.
    for user in [SeedUser::Bob, SeedUser::Carol] {
        let res = run(
            &app,
            ctx_for(user),
            CredentialClass::Session,
            CREATE_BOT,
            json!({"i": {"name": "X", "email": "x-bot@acme.example"}}),
        )
        .await;
        assert_eq!(code(&res), "forbidden", "{user:?}");
        let res = run(
            &app,
            ctx_for(user),
            CredentialClass::Session,
            LIST,
            json!({}),
        )
        .await;
        assert_eq!(code(&res), "forbidden", "{user:?}");
    }
    // Admin, aber per API-Key: nein.
    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        CredentialClass::ApiKey,
        CREATE_BOT,
        json!({"i": {"name": "X", "email": "x-bot@acme.example"}}),
    )
    .await;
    assert_eq!(code(&res), "forbidden");
    let res = app
        .create_bot(
            &RequestCtx {
                via_session: false,
                ..ctx_for(SeedUser::Alice)
            },
            lineal_app::usecases::bot::CreateBotInput {
                name: "X".into(),
                email: "x-bot@acme.example".into(),
            },
        )
        .await;
    assert!(res.is_err());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bot_keys_are_limited_to_bots_and_never_admin(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let bot = create_bot(&app).await;
    let bob = SeedData::acme().bob.to_string();

    // Keine Keys für normale User über den Bot-Weg.
    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        CREATE_KEY,
        json!({"b": bob, "i": {"label": "x", "scopes": ["READ"]}}),
    )
    .await;
    assert_eq!(code(&res), "not_found");

    // Kein admin-Scope (auch nicht über den Use-Case direkt).
    let res = app
        .create_bot_api_key(
            &ctx_for(SeedUser::Alice),
            lineal_domain::ids::UserId(uuid::Uuid::parse_str(&bot).unwrap()),
            lineal_app::usecases::auth::CreateApiKeyInput {
                label: "x".into(),
                scopes: vec!["read".into(), "admin".into()],
                teams: None,
                expires_at: None,
            },
        )
        .await;
    assert!(res.is_err());

    // Fremde Keys (Bobs eigener) lassen sich nicht über den Bot widerrufen.
    let own = app
        .create_api_key(
            &ctx_for(SeedUser::Bob),
            lineal_app::usecases::auth::CreateApiKeyInput {
                label: "bob".into(),
                scopes: vec!["read".into()],
                teams: None,
                expires_at: None,
            },
        )
        .await
        .unwrap();
    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        REVOKE_KEY,
        json!({"b": bot, "id": own.record.id.to_string()}),
    )
    .await;
    assert_eq!(code(&res), "not_found");
    assert!(app.principal_for_token(&own.key).await.is_ok());

    // Doppelte E-Mail.
    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        CREATE_BOT,
        json!({"i": {"name": "Again", "email": "lineal-bot@acme.example"}}),
    )
    .await;
    assert_eq!(code(&res), "conflict");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn bot_key_comments_as_the_bot(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let bot = create_bot(&app).await;
    let bot_ctx = RequestCtx {
        actor: Actor::App {
            id: lineal_domain::ids::UserId(uuid::Uuid::parse_str(&bot).unwrap()),
        },
        scopes: lineal_app::Scopes {
            read: true,
            write: true,
            admin: false,
        },
        via_session: false,
        ..ctx_for(SeedUser::Alice)
    };
    let res = run(
        &app,
        ctx_for(SeedUser::Alice),
        CredentialClass::Session,
        "mutation { issueCreate(input: {teamId: \"ENG\", title: \"Für den Bot\"}) { issue { identifier } } }",
        json!({}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let ident = res.data.into_json().unwrap()["issueCreate"]["issue"]["identifier"]
        .as_str()
        .unwrap()
        .to_owned();
    let comment = format!(
        "mutation {{ commentCreate(input: {{ issueId: \"{ident}\", body: \"Hallo vom Bot\" }}) {{ success comment {{ body user {{ displayName }} }} }} }}"
    );
    let comment = comment.as_str();
    // Öffentliches Team: wie ein normaler User auch ohne Mitgliedschaft.
    let res = run(&app, bot_ctx, CredentialClass::ApiKey, comment, json!({})).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(
        v["commentCreate"]["comment"]["user"]["displayName"],
        "lineal-bot"
    );
}
