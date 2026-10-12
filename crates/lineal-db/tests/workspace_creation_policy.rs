#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP23: Betreiber-Policy `LINEAL_WORKSPACE_CREATION` in `create_workspace_for_account`.

use lineal_app::ports::Store;
use lineal_app::{App, ErrorKind, WorkspaceCreation};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

async fn setup(pool: PgPool, mode: WorkspaceCreation, admins: &[&str]) -> App {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.config.workspace_creation = mode;
    app.config.instance_admins = admins.iter().map(|s| (*s).to_string()).collect();
    app
}

#[sqlx::test(migrations = "./migrations")]
async fn disabled_forbids_everyone(pool: PgPool) {
    let app = setup(pool, WorkspaceCreation::Disabled, &["alice@acme.example"]).await;
    let err = app
        .create_workspace_for_account(&ctx_for(SeedUser::Alice), "Second", None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Forbidden);
    assert_eq!(
        err.message,
        "Workspace creation is disabled on this instance."
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn admins_allows_only_listed_emails_case_insensitively(pool: PgPool) {
    // Auch eine nicht normalisierte Liste trifft (Vergleich ohne Beachtung der Schreibweise).
    let app = setup(pool, WorkspaceCreation::Admins, &["Alice@ACME.example"]).await;
    let bob = app
        .create_workspace_for_account(&ctx_for(SeedUser::Bob), "Bobs", None)
        .await
        .unwrap_err();
    assert_eq!(bob.kind, ErrorKind::Forbidden);
    let created = app
        .create_workspace_for_account(&ctx_for(SeedUser::Alice), "Alices", None)
        .await
        .unwrap();
    assert_eq!(created.org.url_key, "alices");
}

#[sqlx::test(migrations = "./migrations")]
async fn all_allows_any_active_user(pool: PgPool) {
    let app = setup(pool, WorkspaceCreation::All, &[]).await;
    let created = app
        .create_workspace_for_account(&ctx_for(SeedUser::Bob), "Bobs", None)
        .await
        .unwrap();
    assert_eq!(created.owner.email, "bob@acme.example");
}
