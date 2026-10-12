#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `workspaceSecuritySettings(Update)`, `workspaceMembers`, `userRoleUpdate`,
//! `userSuspend`, `userUnsuspend` (WP22).

use async_graphql::{Request, Variables};
use lineal_api::graphql::{LinealSchema, build_schema};
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
    query: &str,
    vars: Value,
) -> async_graphql::Response {
    schema
        .execute(
            Request::new(query)
                .variables(Variables::from_json(vars))
                .data(ctx),
        )
        .await
}

fn code(res: &async_graphql::Response) -> String {
    let e = res.errors.first().expect("expected error");
    let v = serde_json::to_value(e).unwrap();
    v["extensions"]["code"].as_str().unwrap_or("").to_owned()
}

const SETTINGS: &str =
    "{ workspaceSecuritySettings { invitationsRole adminManagementRole allowedInviteDomains } }";
const UPDATE: &str = "mutation($i: SecuritySettingsUpdateInput!) { workspaceSecuritySettingsUpdate(input: $i) { success securitySettings { invitationsRole allowedInviteDomains } } }";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn security_settings_roundtrip_and_rights(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let res = run(&schema, ctx_for(SeedUser::Bob), SETTINGS, json!({})).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(v["workspaceSecuritySettings"]["invitationsRole"], "user");
    assert_eq!(
        v["workspaceSecuritySettings"]["allowedInviteDomains"],
        json!([])
    );

    let res = run(&schema, ctx_for(SeedUser::Carol), SETTINGS, json!({})).await;
    assert_eq!(code(&res), "forbidden");

    let input =
        json!({"i": {"invitationsRole": "admin", "allowedInviteDomains": ["Acme.Example"]}});
    let res = run(&schema, ctx_for(SeedUser::Bob), UPDATE, input.clone()).await;
    assert_eq!(code(&res), "forbidden");
    let res = run(&schema, ctx_for(SeedUser::Alice), UPDATE, input).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(
        v["workspaceSecuritySettingsUpdate"]["securitySettings"],
        json!({"invitationsRole": "admin", "allowedInviteDomains": ["acme.example"]})
    );

    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        UPDATE,
        json!({"i": {"invitationsRole": "superuser"}}),
    )
    .await;
    assert_eq!(code(&res), "invalid_request");
}

const MEMBERS: &str = "query($s: Boolean!) { workspaceMembers(includeSuspended: $s) { id email role status teams { key } } }";
const SUSPEND: &str =
    "mutation($u: String!) { userSuspend(userId: $u) { success member { id status } } }";
const UNSUSPEND: &str =
    "mutation($u: String!) { userUnsuspend(userId: $u) { success member { id status } } }";
const ROLE: &str = "mutation($u: String!, $r: String!) { userRoleUpdate(userId: $u, role: $r) { success member { id role } } }";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn members_role_and_suspension(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let bob_id = SeedData::acme().bob.to_string();

    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        MEMBERS,
        json!({"s": false}),
    )
    .await;
    assert_eq!(code(&res), "forbidden");
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        MEMBERS,
        json!({"s": false}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let rows = v["workspaceMembers"].as_array().unwrap();
    let bob = rows.iter().find(|r| r["id"] == bob_id).unwrap();
    assert_eq!(bob["email"], "bob@acme.example");
    assert_eq!(bob["role"], "user");
    assert_eq!(bob["status"], "active");

    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        ROLE,
        json!({"u": bob_id, "r": "admin"}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        ROLE,
        json!({"u": bob_id, "r": "wizard"}),
    )
    .await;
    assert_eq!(code(&res), "invalid_request");

    // Bob (user im Kontext) darf nicht sperren; Alice sperrt und entsperrt.
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        SUSPEND,
        json!({"u": bob_id}),
    )
    .await;
    assert_eq!(code(&res), "forbidden");
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        SUSPEND,
        json!({"u": bob_id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(v["userSuspend"]["member"]["status"], "suspended");
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        MEMBERS,
        json!({"s": true}),
    )
    .await;
    let v = res.data.into_json().unwrap();
    assert!(
        v["workspaceMembers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == bob_id && r["status"] == "suspended")
    );
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        UNSUSPEND,
        json!({"u": bob_id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(v["userUnsuspend"]["member"]["status"], "active");

    // Sich selbst sperren: nein.
    let alice_id = SeedData::acme().alice.to_string();
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        SUSPEND,
        json!({"u": alice_id}),
    )
    .await;
    assert_eq!(code(&res), "invalid_request");
}
