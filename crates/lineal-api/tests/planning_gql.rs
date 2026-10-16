#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP07: GraphQL-Contract der Planung gegen echte Datenbank – gleiche Rechte wie MCP.

use async_graphql::Request;
use lineal_api::graphql::build_schema;
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;

async fn run(
    schema: &lineal_api::graphql::LinealSchema,
    who: SeedUser,
    q: &str,
) -> (Value, Vec<String>) {
    let res = schema.execute(Request::new(q).data(ctx_for(who))).await;
    let codes = res
        .errors
        .iter()
        .map(|e| {
            e.extensions
                .as_ref()
                .and_then(|x| x.get("code"))
                .map(|c| c.to_string())
                .unwrap_or_default()
        })
        .collect();
    (res.data.into_json().unwrap(), codes)
}

async fn setup(pool: PgPool) -> lineal_api::graphql::LinealSchema {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    build_schema(test_app(Arc::new(store)))
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn projects_cycles_initiatives_over_graphql(pool: PgPool) {
    let schema = setup(pool).await;
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "{ projects { identifier name progress status { name type } leadTeam { key } teams { key } milestones { name status } } \
          cycles(teamId: \"ENG\") { number isActive startsAt } initiatives { identifier status healthRollup { none } projects { identifier } } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(d["projects"][0]["identifier"], "P-ENG-1");
    assert_eq!(d["projects"][0]["status"]["type"], "started");
    assert_eq!(d["projects"][0]["milestones"][0]["name"], "Alpha");
    assert_eq!(d["cycles"].as_array().unwrap().len(), 4);
    assert_eq!(d["cycles"][1]["isActive"], true);
    assert_eq!(d["initiatives"][0]["status"], "Active");
    assert_eq!(d["initiatives"][0]["projects"][0]["identifier"], "P-ENG-1");

    // Projekt-Hülle: Bob legt ENG+DES an, Alice sieht nur ENG; Änderung ohne Recht für DES = not_found.
    let (d, e) = run(
        &schema,
        SeedUser::Bob,
        "mutation { projectCreate(input: { name: \"Mobile\", addTeams: [\"ENG\", \"DES\"], description: \"hi <script>x</script> [a](javascript:x)\" }) { project { identifier teams { key } description } } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(
        d["projectCreate"]["project"]["teams"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        !d["projectCreate"]["project"]["description"]
            .as_str()
            .unwrap()
            .replace("\\<", "")
            .contains("<script")
    );
    let (d, _) = run(
        &schema,
        SeedUser::Alice,
        "{ project(id: \"P-ENG-2\") { teams { key } } }",
    )
    .await;
    assert_eq!(d["project"]["teams"].as_array().unwrap().len(), 1);
    let (_, e) = run(
        &schema,
        SeedUser::Alice,
        "mutation { projectUpdate(id: \"P-ENG-2\", input: { name: \"X\" }) { success } }",
    )
    .await;
    // Zugriff auf ein Projekt-Team genügt (docs/04 §2.2): Alice darf ändern, DES bleibt unsichtbar.
    assert!(e.is_empty(), "{e:?}");
    let (_, e) = run(
        &schema,
        SeedUser::Bob,
        "mutation { projectUpdate(id: \"P-ENG-2\", input: { color: \"red\" }) { success } }",
    )
    .await;
    assert_eq!(e, ["\"invalid_request\""]);

    // Initiative: Zyklus -> invalid_request; Gast -> forbidden.
    let (_, e) = run(&schema, SeedUser::Alice, "mutation { initiativeCreate(input: { name: \"Zwei\", parentInitiatives: [\"I-1\"] }) { success } }").await;
    assert!(e.is_empty());
    let (_, e) = run(&schema, SeedUser::Alice, "mutation { initiativeUpdate(id: \"I-1\", input: { parentInitiatives: [\"I-2\"] }) { success } }").await;
    assert_eq!(e, ["\"invalid_request\""]);
    let (_, e) = run(&schema, SeedUser::Carol, "{ initiatives { identifier } }").await;
    assert_eq!(e, ["\"forbidden\""]);

    // Cycles: Zeitplan nicht änderbar für erbende Sub-Teams, ungültige Werte.
    let (_, e) = run(
        &schema,
        SeedUser::Alice,
        "mutation { teamCyclesUpdate(teamId: \"ENG\", input: { cycleDuration: 9 }) { success } }",
    )
    .await;
    assert_eq!(e, ["\"invalid_request\""]);
}
