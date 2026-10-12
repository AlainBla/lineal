//! `lineal create-workspace` gegen die Datenbank (Passwort wie über `LINEAL_OWNER_PASSWORD`).

#![allow(clippy::unwrap_used)]
use lineal_db::PgStore;
use lineal_server::adapters::build_app;
use lineal_server::cli::create_workspace::{CreateWorkspaceArgs, execute};
use lineal_server::config::Config;
use sqlx::PgPool;
use std::sync::Arc;

fn app(pool: PgPool) -> Arc<lineal_app::App> {
    let cfg = Config::from_lookup(|k| match k {
        "LINEAL_DEV" => Some("1".into()),
        "LINEAL_BASE_URL" => Some("https://lineal.example/".into()),
        _ => None,
    })
    .unwrap();
    build_app(Arc::new(PgStore::new(pool)), &cfg)
}

fn args(url_key: Option<&str>) -> CreateWorkspaceArgs {
    CreateWorkspaceArgs {
        name: "Acme Corp".into(),
        url_key: url_key.map(Into::into),
        owner_email: "alice@acme.example".into(),
        owner_name: "Alice".into(),
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn creates_workspace_and_owner_with_hashed_password(pool: PgPool) {
    let app = app(pool.clone());
    let out = execute(&app, &args(Some("acme-corp")), "correct-horse-1".into())
        .await
        .unwrap();
    assert!(out.contains("acme-corp"), "{out}");
    assert!(out.contains("Login: https://lineal.example/login"), "{out}");
    let (role, hash): (String, Option<String>) = sqlx::query_as(
        r#"SELECT role::text, password_hash FROM "user" WHERE email = 'alice@acme.example'"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(role, "owner");
    let hash = hash.unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains("correct-horse-1"));
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn taken_url_key_and_weak_password_fail(pool: PgPool) {
    let app = app(pool);
    execute(&app, &args(Some("acme-corp")), "correct-horse-1".into())
        .await
        .unwrap();
    let err = execute(&app, &args(Some("acme-corp")), "correct-horse-1".into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("urlKey"), "{err}");
    let err = execute(&app, &args(Some("other-corp")), "short".into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("password"), "{err}");
}
