#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `deletedTeams`/`teamUnarchive`, `previewTeamNesting` und `nestTeam` mit `stateMapping`
//! (WP29).

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

async fn ok(schema: &LinealSchema, user: SeedUser, query: &str, vars: Value) -> Value {
    let res = run(schema, ctx_for(user), query, vars).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    res.data.into_json().unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn deleted_teams_are_listed_for_admins_only_and_can_be_restored(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let list = "{ deletedTeams { id key name restorableUntil trashedAt subTeamCount } }";
    // Nichts gelöscht.
    let v = ok(&schema, SeedUser::Alice, list, json!({})).await;
    assert_eq!(v["deletedTeams"], json!([]));
    // WEB löschen.
    ok(
        &schema,
        SeedUser::Alice,
        "mutation { teamDelete(id: \"WEB\") { success } }",
        json!({}),
    )
    .await;
    let v = ok(&schema, SeedUser::Alice, list, json!({})).await;
    let rows = v["deletedTeams"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["key"], "WEB");
    assert_eq!(rows[0]["subTeamCount"], 0);
    assert!(rows[0]["restorableUntil"].as_str().unwrap() > rows[0]["trashedAt"].as_str().unwrap());
    // Bob (user) und Carol (guest) sehen nichts.
    for user in [SeedUser::Bob, SeedUser::Carol] {
        let v = ok(&schema, user, list, json!({})).await;
        assert_eq!(v["deletedTeams"], json!([]), "{user:?}");
    }
    // Bob darf nicht wiederherstellen.
    let id = rows[0]["id"].as_str().unwrap().to_owned();
    let res = run(
        &schema,
        ctx_for(SeedUser::Bob),
        "mutation($id: String!) { teamUnarchive(id: $id) { success } }",
        json!({ "id": id }),
    )
    .await;
    assert!(!res.errors.is_empty());
    // Alice stellt wieder her; danach ist die Liste leer und das Team sichtbar.
    ok(
        &schema,
        SeedUser::Alice,
        "mutation($id: String!) { teamUnarchive(id: $id) { success team { key } } }",
        json!({ "id": id }),
    )
    .await;
    let v = ok(&schema, SeedUser::Alice, list, json!({})).await;
    assert_eq!(v["deletedTeams"], json!([]));
    let v = ok(
        &schema,
        SeedUser::Alice,
        "{ team(id: \"WEB\") { key } }",
        json!({}),
    )
    .await;
    assert_eq!(v["team"]["key"], "WEB");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sub_teams_deleted_together_with_their_parent_are_not_listed_separately(
    pool: sqlx::PgPool,
) {
    let schema = setup(pool).await;
    ok(
        &schema,
        SeedUser::Alice,
        "mutation { teamDelete(id: \"ENG\", handleSubTeams: \"delete\") { success } }",
        json!({}),
    )
    .await;
    let v = ok(
        &schema,
        SeedUser::Alice,
        "{ deletedTeams { key subTeamCount } }",
        json!({}),
    )
    .await;
    assert_eq!(
        v["deletedTeams"],
        json!([{ "key": "ENG", "subTeamCount": 1 }])
    );
}

const PREVIEW: &str = "query($t: String!, $p: String) { previewTeamNesting(teamId: $t, parentId: $p) { conflicts { kind message details } } }";
const NEST: &str = "mutation($t: String!, $p: String, $o: NestTeamOptions) { nestTeam(teamId: $t, parentId: $p, options: $o) { team { key parentId inheritWorkflowStatuses } } }";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn nesting_preview_lists_status_mapping_and_nest_accepts_overrides(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    // WEB aus ENG lösen, dann Vorschau für das erneute Nesting.
    ok(
        &schema,
        SeedUser::Alice,
        NEST,
        json!({ "t": "WEB", "p": null }),
    )
    .await;
    let v = ok(
        &schema,
        SeedUser::Alice,
        PREVIEW,
        json!({ "t": "WEB", "p": "ENG" }),
    )
    .await;
    let conflicts = v["previewTeamNesting"]["conflicts"].as_array().unwrap();
    let states = conflicts
        .iter()
        .find(|c| c["kind"] == "workflowState")
        .expect("workflowState conflict");
    let entries = states["details"]["states"].as_array().unwrap();
    assert!(!entries.is_empty());
    for e in entries {
        assert!(e["id"].is_string() && e["targetId"].is_string() && e["targetName"].is_string());
    }
    // ENG hat Cycles: das Team übernimmt den Zeitplan (Konflikt `cycle`).
    assert!(conflicts.iter().any(|c| c["kind"] == "cycle"));
    // Gleichnamiges Label in ENG und in WEB: Konflikt `label` (bleibt Team-Label).
    for team in ["ENG", "WEB"] {
        let input = json!({ "name": "Dup", "teamId": team });
        ok(
            &schema,
            SeedUser::Alice,
            "mutation($i: IssueLabelCreateInput!) { issueLabelCreate(input: $i) { success } }",
            json!({ "i": input }),
        )
        .await;
    }
    let v = ok(
        &schema,
        SeedUser::Alice,
        PREVIEW,
        json!({ "t": "WEB", "p": "ENG" }),
    )
    .await;
    let label = v["previewTeamNesting"]["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "label")
        .expect("label conflict")
        .clone();
    assert_eq!(label["details"]["labels"][0]["name"], "Dup");
    // Die Vorschau schreibt nichts: WEB ist weiterhin Root.
    let v = ok(
        &schema,
        SeedUser::Alice,
        "{ team(id: \"WEB\") { parentId } }",
        json!({}),
    )
    .await;
    assert!(v["team"]["parentId"].is_null());
    // Ungültige Zuordnung (Quelle = Status des Parents): invalid_request.
    let eng_states = ok(
        &schema,
        SeedUser::Alice,
        "{ workflowStates(teamId: \"ENG\") { id } }",
        json!({}),
    )
    .await;
    let eng_state = eng_states["workflowStates"][0]["id"].as_str().unwrap();
    let res = run(
        &schema,
        ctx_for(SeedUser::Alice),
        NEST,
        json!({ "t": "WEB", "p": "ENG", "o": { "stateMapping": [
            { "fromStateId": eng_state, "toStateId": eng_state }
        ] } }),
    )
    .await;
    assert_eq!(res.errors.len(), 1);
    // Gültige Zuordnung wird ausgeführt.
    let web_states = ok(
        &schema,
        SeedUser::Alice,
        "{ workflowStates(teamId: \"WEB\") { id type } }",
        json!({}),
    )
    .await;
    let from = web_states["workflowStates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["type"] == "started")
        .expect("started state");
    let to = eng_states["workflowStates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .next()
        .unwrap();
    let v = ok(
        &schema,
        SeedUser::Alice,
        NEST,
        json!({ "t": "WEB", "p": "ENG", "o": { "stateMapping": [
            { "fromStateId": from["id"], "toStateId": to }
        ] } }),
    )
    .await;
    assert_eq!(v["nestTeam"]["team"]["inheritWorkflowStatuses"], true);
    assert!(v["nestTeam"]["team"]["parentId"].is_string());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn nesting_preview_and_nest_need_an_admin(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    for q in [PREVIEW, NEST] {
        let res = run(
            &schema,
            ctx_for(SeedUser::Bob),
            q,
            json!({ "t": "WEB", "p": "ENG" }),
        )
        .await;
        assert!(!res.errors.is_empty(), "{q}");
    }
}
