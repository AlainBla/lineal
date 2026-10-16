#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP10: GraphQL-Contract für Dokumente und Templates gegen echte Datenbank – gleiche Regeln wie MCP.

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
async fn documents_over_graphql(pool: PgPool) {
    let schema = setup(pool).await;
    let (d, e) = run(
        &schema,
        SeedUser::Bob,
        "mutation { documentCreate(input: { title: \"Intern\", content: \"hi <script>x</script> secret-word\", teamId: \"DES\" }) { document { id slugId url content contentVersion teamId } } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let doc = &d["documentCreate"]["document"];
    assert!(
        !doc["content"]
            .as_str()
            .unwrap()
            .replace("\\<", "")
            .contains("<script")
    );
    let id = doc["id"].as_str().unwrap().to_owned();
    // Bob sieht es, Alice (kein DES-Mitglied) nicht: get, Liste und Suche.
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        &format!("{{ document(id: \"{id}\") {{ id }} }}"),
    )
    .await;
    assert!(d.is_null() || d["document"].is_null());
    assert_eq!(e, ["\"not_found\""]);
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "{ documents { nodes { id } } searchDocuments(term: \"secret-word\") { nodes { id } } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(d["documents"]["nodes"].as_array().unwrap().len(), 0);
    assert_eq!(d["searchDocuments"]["nodes"].as_array().unwrap().len(), 0);
    let (d, e) = run(
        &schema,
        SeedUser::Bob,
        "{ searchDocuments(term: \"secret-word\") { nodes { id } } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(d["searchDocuments"]["nodes"].as_array().unwrap().len(), 1);

    // Zwei Parents -> invalid_request; Update mit Patch; Historie.
    let (_, e) = run(
        &schema,
        SeedUser::Alice,
        "mutation { documentCreate(input: { title: \"X\", teamId: \"ENG\", projectId: \"P-ENG-1\" }) { success } }",
    )
    .await;
    assert_eq!(e, ["\"invalid_request\""]);
    let (d, e) = run(
        &schema,
        SeedUser::Bob,
        &format!("mutation {{ documentUpdate(id: \"{id}\", input: {{ patch: [{{ op: \"append\", text: \"!\" }}] }}) {{ document {{ content }} }} }}"),
    )
    .await;
    // `patch` ist ein JSON-Skalar: Objekt-Literale sind kein gültiges JSON-Literal -> mit Variablen testen.
    let _ = (d, e);
    let req = Request::new(
        "mutation($id: String!, $p: JSON) { documentUpdate(id: $id, input: { patch: $p }) { document { content } } }",
    )
    .variables(async_graphql::Variables::from_json(serde_json::json!({
        "id": id, "p": [{ "op": "append", "text": "!" }]
    })))
    .data(ctx_for(SeedUser::Bob));
    let res = schema.execute(req).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let (d, e) = run(
        &schema,
        SeedUser::Bob,
        &format!(
            "{{ documentContentHistory(documentId: \"{id}\") {{ nodes {{ kind markdown }} }} }}"
        ),
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    let kinds: Vec<_> = d["documentContentHistory"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["edited", "edited", "created"]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn templates_over_graphql(pool: PgPool) {
    let schema = setup(pool).await;
    let req = Request::new(
        "mutation($d: JSON!) { templateCreate(input: { type: issue, name: \"Feature\", teamId: \"ENG\", templateData: $d }) { template { id name type teamKey } } }",
    )
    .variables(async_graphql::Variables::from_json(serde_json::json!({
        "d": { "title": "Feature: ", "subIssues": [ { "title": "A" }, { "title": "B" }, { "title": "C" } ] }
    })))
    .data(ctx_for(SeedUser::Alice));
    let res = schema.execute(req).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "{ templates(teamId: \"WEB\") { name type teamKey } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(d["templates"][0]["teamKey"], "ENG");
    let (d, e) = run(
        &schema,
        SeedUser::Alice,
        "mutation { templateIssueCreate(templateId: \"Feature\", input: { teamId: \"ENG\", title: \"Dark Mode\" }) { issue { identifier } subIssues { identifier title } warnings } }",
    )
    .await;
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(
        d["templateIssueCreate"]["subIssues"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    // Gast darf keine Workspace-Templates anlegen.
    let (_, e) = run(
        &schema,
        SeedUser::Carol,
        "mutation { templateCreate(input: { type: document, name: \"X\", templateData: {} }) { success } }",
    )
    .await;
    assert!(!e.is_empty());
}
