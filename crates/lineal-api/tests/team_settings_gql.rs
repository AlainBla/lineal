#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `Team.memberCount`/`allMembersCanJoin`, `teamCreate`/`teamUpdate` (WP26).

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

const CREATE: &str = "mutation($i: TeamCreateInput!) { teamCreate(input: $i) { team { id key allMembersCanJoin memberCount } } }";
const UPDATE: &str = "mutation($id: String!, $i: TeamUpdateInput!) { teamUpdate(id: $id, input: $i) { team { allMembersCanJoin } } }";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn create_and_update_expose_join_flag_and_member_count(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        CREATE,
        json!({"i": {"name": "KollegePlus", "key": "KP", "allMembersCanJoin": false}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let team = &v["teamCreate"]["team"];
    assert_eq!(team["key"], "KP");
    assert_eq!(team["allMembersCanJoin"], false);
    assert_eq!(team["memberCount"], 1);
    let id = team["id"].as_str().unwrap().to_owned();

    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        UPDATE,
        json!({"id": id, "i": {"allMembersCanJoin": true}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    assert_eq!(v["teamUpdate"]["team"]["allMembersCanJoin"], true);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn explicit_null_clears_icon_and_color_but_absent_keeps_them(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let q = "mutation($i: TeamUpdateInput!) { teamUpdate(id: \"ENG\", input: $i) { team { icon color name } } }";
    let alice = || ctx_for(SeedUser::Alice);
    let res = run(
        &schema,
        alice(),
        q,
        json!({"i": {"icon": "Rocket", "color": "#112233"}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    // Feld fehlt: unverändert.
    let res = run(
        &schema,
        alice(),
        q,
        json!({"i": {"timezone": "Europe/Berlin"}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let t = res.data.into_json().unwrap()["teamUpdate"]["team"].clone();
    assert_eq!(t["icon"], "Rocket");
    assert_eq!(t["color"], "#112233");
    // Nur das Icon auf null: die Farbe bleibt.
    let res = run(&schema, alice(), q, json!({"i": {"icon": null}})).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let t = res.data.into_json().unwrap()["teamUpdate"]["team"].clone();
    assert!(t["icon"].is_null());
    assert_eq!(t["color"], "#112233");
    let res = run(&schema, alice(), q, json!({"i": {"color": null}})).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let t = res.data.into_json().unwrap()["teamUpdate"]["team"].clone();
    assert!(t["color"].is_null());
}

const SET_OWNER: &str = "mutation($t: String!, $u: String!, $o: Boolean!) { teamMembershipUpdate(teamId: $t, userId: $u, owner: $o) { success } }";
const MEMBERS: &str = "query($t: String!) { teamMembers(team: $t) { id name owner role } team(id: $t) { viewer { isMember isOwner canJoin } } }";

/// F.14: Owner ernennen/entziehen; Rechte laut docs/02 §6.4.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn owner_can_be_set_and_revoked_with_rules(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let bob_id = SeedData::acme().bob.to_string();
    // Bob ist in ENG nur Mitglied: er darf keine Owner ernennen.
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        SET_OWNER,
        json!({"t": "ENG", "u": bob_id, "o": true}),
    )
    .await;
    assert!(!res.errors.is_empty(), "member must not set owner");
    // Alice (Owner/Admin) ernennt Bob.
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        SET_OWNER,
        json!({"t": "ENG", "u": bob_id, "o": true}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        MEMBERS,
        json!({"t": "ENG"}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let bob = v["teamMembers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == bob_id)
        .unwrap();
    assert_eq!(bob["owner"], true);
    assert_eq!(v["team"]["viewer"]["isOwner"], true);
    assert_eq!(v["team"]["viewer"]["isMember"], true);
    assert_eq!(v["team"]["viewer"]["canJoin"], false);
    // Entziehen (kein Pflicht-Owner, docs/02 §4.3).
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        SET_OWNER,
        json!({"t": "ENG", "u": bob_id, "o": false}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    // Gast wird nie Owner.
    let carol_id = SeedData::acme().carol.to_string();
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        SET_OWNER,
        json!({"t": "WEB", "u": carol_id, "o": true}),
    )
    .await;
    assert!(!res.errors.is_empty());
    // Gast sieht Team-Mitglieder von ENG nicht.
    let res = run(
        &schema,
        ctx_for(SeedUser::Carol),
        MEMBERS,
        json!({"t": "ENG"}),
    )
    .await;
    assert!(!res.errors.is_empty());
}

/// F.19: `viewer.canJoin` folgt `allMembersCanJoin`; Selbstbeitritt und Verlassen per Mutation.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn viewer_can_join_and_leave(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        CREATE,
        json!({"i": {"name": "Offen", "key": "OPN", "allMembersCanJoin": true}}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let bob_id = SeedData::acme().bob.to_string();
    let q = "query { team(id: \"OPN\") { viewer { isMember canJoin } } }";
    let res = run(&schema, ctx_for(SeedUser::Bob), q, json!({})).await;
    let v = res.data.into_json().unwrap();
    assert_eq!(v["team"]["viewer"]["canJoin"], true);
    let join = "mutation($t: String!, $u: String!) { teamMembershipCreate(teamId: $t, userId: $u) { success } }";
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        join,
        json!({"t": "OPN", "u": bob_id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let res = run(&schema, ctx_for(SeedUser::Bob), q, json!({})).await;
    let v = res.data.into_json().unwrap();
    assert_eq!(v["team"]["viewer"]["isMember"], true);
    let leave = "mutation($t: String!, $u: String!) { teamMembershipDelete(teamId: $t, userId: $u) { success } }";
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        leave,
        json!({"t": "OPN", "u": bob_id}),
    )
    .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}
