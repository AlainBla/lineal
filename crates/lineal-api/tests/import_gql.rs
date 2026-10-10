#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP18: GraphQL-Vertrag des Imports gegen echte Datenbank – nur Admins, Token nie in Antworten.

use async_graphql::Request;
use lineal_api::graphql::build_schema;
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;

const TOKEN: &str = "lin_api_GQLSECRETTOKEN0123456789";

async fn run(schema: &lineal_api::graphql::LinealSchema, who: SeedUser, q: &str) -> (Value, Vec<String>, String) {
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
    let raw = format!("{:?}", res.errors);
    (res.data.into_json().unwrap(), codes, raw)
}

async fn setup(pool: PgPool) -> lineal_api::graphql::LinealSchema {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    build_schema(test_app(Arc::new(store)))
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn only_admins_can_use_the_import_api(pool: PgPool) {
    let schema = setup(pool).await;
    let scan = format!("mutation {{ importScan(input: {{ source: \"linear\", token: \"{TOKEN}\" }}) {{ id status }} }}");
    // Bob (user) und Carol (guest): forbidden, und das Token steht in keiner Fehlermeldung.
    for who in [SeedUser::Bob, SeedUser::Carol] {
        let (_, codes, raw) = run(&schema, who, &scan).await;
        assert_eq!(codes, ["\"forbidden\""], "{who:?}");
        assert!(!raw.contains("GQLSECRETTOKEN"), "token in error output: {raw}");
        let (_, codes, _) = run(&schema, who, "{ importRuns { id } }").await;
        assert_eq!(codes, ["\"forbidden\""]);
    }
    // Alice (owner): Lauf entsteht im Status `scanning`; das Token wird nie zurückgegeben.
    let (d, codes, raw) = run(&schema, SeedUser::Alice, &scan).await;
    assert!(codes.is_empty(), "{codes:?} {raw}");
    assert_eq!(d["importScan"]["status"], "scanning");
    assert!(!d.to_string().contains("GQLSECRETTOKEN"));
    let id = d["importScan"]["id"].as_str().unwrap().to_owned();
    let (d, codes, _) = run(
        &schema,
        SeedUser::Alice,
        &format!("{{ importRun(id: \"{id}\") {{ id status source dryRun scan options error }} importRuns {{ id }} }}"),
    )
    .await;
    assert!(codes.is_empty());
    assert_eq!(d["importRun"]["source"], "linear");
    assert_eq!(d["importRuns"].as_array().unwrap().len(), 1);
    // Bob kann den Lauf nicht lesen, steuern oder abbrechen.
    for q in [
        format!("{{ importRun(id: \"{id}\") {{ id }} }}"),
        format!("{{ importReport(runId: \"{id}\") {{ errorCount }} }}"),
        format!("mutation {{ importCancel(runId: \"{id}\") {{ id }} }}"),
        format!("mutation {{ importPause(runId: \"{id}\") {{ id }} }}"),
        format!("mutation {{ importResume(runId: \"{id}\") {{ id }} }}"),
        format!("mutation {{ importStart(input: {{ runId: \"{id}\" }}) {{ id }} }}"),
    ] {
        let (_, codes, _) = run(&schema, SeedUser::Bob, &q).await;
        assert_eq!(codes, ["\"forbidden\""], "{q}");
    }
    // Ein noch nicht gescannter Lauf kann nicht gestartet werden.
    let (_, codes, _) = run(
        &schema,
        SeedUser::Alice,
        &format!("mutation {{ importStart(input: {{ runId: \"{id}\", dryRun: true }}) {{ id }} }}"),
    )
    .await;
    assert_eq!(codes, ["\"conflict\""]);
    // Ungültige Tokens/Ids werden abgelehnt, ohne das Token zu wiederholen.
    let (_, codes, raw) = run(
        &schema,
        SeedUser::Alice,
        "mutation { importScan(input: { source: \"linear\", token: \"a b\" }) { id } }",
    )
    .await;
    assert_eq!(codes, ["\"invalid_request\""]);
    assert!(!raw.contains("a b"));
    let (_, codes, _) = run(&schema, SeedUser::Alice, "{ importRun(id: \"nope\") { id } }").await;
    assert_eq!(codes, ["\"invalid_request\""]);
    // Abbrechen löscht das Token.
    let (d, codes, _) = run(
        &schema,
        SeedUser::Alice,
        &format!("mutation {{ importCancel(runId: \"{id}\") {{ status }} }}"),
    )
    .await;
    assert!(codes.is_empty());
    assert_eq!(d["importCancel"]["status"], "canceled");
}
