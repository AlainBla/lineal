#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP23: Betreiber-Policy `LINEAL_WORKSPACE_CREATION` in `create_workspace_for_account`.

use lineal_app::context::{Actor, Scopes};
use lineal_app::ports::Store;
use lineal_app::usecases::org::CreateWorkspaceInput;
use lineal_app::{App, ErrorKind, RequestCtx, WorkspaceCreation};
use lineal_db::PgStore;
use lineal_domain::enums::UserRole;
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
    let app = setup(
        pool,
        WorkspaceCreation::Disabled,
        &["acme:alice@acme.example"],
    )
    .await;
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
async fn admins_requires_listed_workspace_email_and_admin_role(pool: PgPool) {
    // Auch eine nicht normalisierte Liste trifft (Vergleich ohne Beachtung der Schreibweise).
    let app = setup(
        pool,
        WorkspaceCreation::Admins,
        &["ACME:Alice@ACME.example", "acme:bob@acme.example"],
    )
    .await;
    // Bob steht in der Liste, hat im Workspace aber nur die Rolle User.
    let bob = app
        .create_workspace_for_account(&ctx_for(SeedUser::Bob), "Bobs", None)
        .await
        .unwrap_err();
    assert_eq!(bob.kind, ErrorKind::Forbidden);
    // Carol steht nicht in der Liste.
    let carol = app
        .create_workspace_for_account(&ctx_for(SeedUser::Carol), "Carols", None)
        .await
        .unwrap_err();
    assert_eq!(carol.kind, ErrorKind::Forbidden);
    // Gleiche Adresse als Owner eines fremden Workspaces: kein Instanz-Admin.
    let foreign = app
        .create_workspace(CreateWorkspaceInput {
            name: "Evil".into(),
            url_key: Some("evil".into()),
            owner_name: "Mallory".into(),
            owner_email: "alice@acme.example".into(),
            owner_password: Some("lineal-dev".into()),
        })
        .await
        .unwrap();
    let ctx = RequestCtx {
        org_id: foreign.org.id,
        actor: Actor::User {
            id: foreign.owner.id,
            role: UserRole::Owner,
        },
        request_id: "r".into(),
        scopes: Scopes::all(),
        team_scope: None,
        via_session: false,
    };
    let err = app
        .create_workspace_for_account(&ctx, "Stolen", None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Forbidden);
    // Richtiger Workspace + Owner: erlaubt.
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
