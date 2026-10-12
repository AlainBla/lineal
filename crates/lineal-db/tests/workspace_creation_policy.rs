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
use uuid::Uuid;

async fn setup(pool: PgPool, mode: WorkspaceCreation, admins: &[Uuid]) -> App {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.config.workspace_creation = mode;
    app.config.instance_admins = admins.to_vec();
    app
}

fn alice() -> Uuid {
    SeedData::acme().alice.0
}

fn is_forbidden<T>(r: Result<T, lineal_app::AppError>) -> bool {
    r.err().map(|e| e.kind) == Some(ErrorKind::Forbidden)
}

#[sqlx::test(migrations = "./migrations")]
async fn disabled_forbids_everyone(pool: PgPool) {
    let app = setup(pool, WorkspaceCreation::Disabled, &[alice()]).await;
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
async fn admins_allows_listed_owner_and_survives_url_key_rename(pool: PgPool) {
    let app = setup(pool.clone(), WorkspaceCreation::Admins, &[alice()]).await;
    // Der urlKey ist veränderlich und spielt keine Rolle.
    sqlx::query("UPDATE organization SET url_key = 'renamed-acme'")
        .execute(&pool)
        .await
        .unwrap();
    let created = app
        .create_workspace_for_account(&ctx_for(SeedUser::Alice), "Alices", None)
        .await
        .unwrap();
    assert_eq!(created.org.url_key, "alices");
}

#[sqlx::test(migrations = "./migrations")]
async fn admins_rejects_unlisted_role_user_suspended_and_same_email_elsewhere(pool: PgPool) {
    let data = SeedData::acme();
    // Bob (Rolle User) und Carol stehen in der Liste, Alice nicht.
    let app = setup(
        pool.clone(),
        WorkspaceCreation::Admins,
        &[data.bob.0, data.carol.0],
    )
    .await;
    // Nicht in der Liste (Alice ist Owner, aber nicht eingetragen).
    assert!(is_forbidden(
        app.create_workspace_for_account(&ctx_for(SeedUser::Alice), "A", None)
            .await
    ));
    // In der Liste, aber Rolle User.
    assert!(is_forbidden(
        app.create_workspace_for_account(&ctx_for(SeedUser::Bob), "B", None)
            .await
    ));
    // Gleiche E-Mail wie Alice als Owner eines fremden Workspaces: andere User-ID.
    let foreign = app
        .create_workspace(CreateWorkspaceInput {
            name: "Evil".into(),
            url_key: Some("evil".into()),
            owner_name: "Mallory".into(),
            owner_email: "alice@acme.example".into(),
            owner_password: Some("lineal-dev".into()),
            credential_kind: None,
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
        client_ip: None,
        credential_kind: None,
    };
    assert!(is_forbidden(
        app.create_workspace_for_account(&ctx, "Stolen", None).await
    ));
    // Carol: in der Liste, Admin, aber suspendiert -> verboten.
    sqlx::query(r#"UPDATE "user" SET role = 'admin', active = false WHERE id = $1"#)
        .bind(data.carol.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(is_forbidden(
        app.create_workspace_for_account(&ctx_for(SeedUser::Carol), "C", None)
            .await
    ));
    // Als aktiver Admin erlaubt.
    sqlx::query(r#"UPDATE "user" SET active = true WHERE id = $1"#)
        .bind(data.carol.0)
        .execute(&pool)
        .await
        .unwrap();
    app.create_workspace_for_account(&ctx_for(SeedUser::Carol), "C", None)
        .await
        .unwrap();
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
