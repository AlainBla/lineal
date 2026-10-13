#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP06: GraphQL-Contract für `issue`/`issues` und Issue-Labels gegen echte Datenbank.

use async_graphql::Request;
use lineal_api::graphql::build_schema;
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;

async fn run(
    schema: &lineal_api::graphql::LinealSchema,
    who: SeedUser,
    query: &str,
) -> async_graphql::Response {
    // Wie die Router-Pipeline: frisches Request-Budget für die Nachladefelder.
    schema
        .execute(
            Request::new(query)
                .data(ctx_for(who))
                .data(lineal_api::context::DetailBudget::default()),
        )
        .await
}

fn code(res: &async_graphql::Response) -> String {
    let err = res.errors.first().expect("error expected");
    let v = serde_json::to_value(err).unwrap();
    v["extensions"]["code"].as_str().unwrap_or("").to_owned()
}

fn data(res: async_graphql::Response) -> Value {
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    res.data.into_json().unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issue_mutations_queries_and_visibility(pool: sqlx::PgPool) {
    let sql_pool = pool.clone();
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let schema = build_schema(test_app(Arc::new(store)));

    let created = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueCreate(input: {teamId: "ENG", title: "Fokus-Ring", priority: 2}) {
                success issue { id identifier title priority stateName stateType } } }"#,
        )
        .await,
    );
    assert_eq!(created["issueCreate"]["issue"]["identifier"], "ENG-1");
    assert_eq!(created["issueCreate"]["issue"]["priority"], 2);
    assert_eq!(created["issueCreate"]["issue"]["stateType"], "backlog");

    // B7.11: die SLA-Felder sind lesbar.
    sqlx::query(
        "UPDATE issue SET sla_type = 'all', sla_started_at = '2026-10-01T00:00:00Z', \
         sla_breaches_at = '2026-10-05T12:00:00Z' WHERE identifier = 'ENG-1'",
    )
    .execute(&sql_pool)
    .await
    .unwrap();
    let sla = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"{ issue(id: "ENG-1") { slaType slaStartedAt slaBreachesAt } }"#,
        )
        .await,
    );
    assert_eq!(sla["issue"]["slaType"], "all");
    assert!(
        sla["issue"]["slaBreachesAt"]
            .as_str()
            .is_some_and(|t| t.starts_with("2026-10-05T12:00:00"))
    );
    assert!(sla["issue"]["slaStartedAt"].is_string());

    // Tri-State: null leert, weggelassen bleibt
    let upd = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueUpdate(id: "ENG-1", input: {assigneeId: "bob", estimate: null, title: "Neu"}) {
                issue { title assigneeId priority } } }"#,
        )
        .await,
    );
    assert_eq!(upd["issueUpdate"]["issue"]["title"], "Neu");
    assert_eq!(upd["issueUpdate"]["issue"]["priority"], 2);
    let cleared = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueUpdate(id: "ENG-1", input: {assigneeId: null}) { issue { assigneeId } } }"#,
        )
        .await,
    );
    assert_eq!(cleared["issueUpdate"]["issue"]["assigneeId"], Value::Null);

    // Liste und Einzelabfrage
    let list = data(
        run(
            &schema,
            SeedUser::Alice,
            "{ issues(first: 10) { nodes { identifier } hasNextPage } }",
        )
        .await,
    );
    assert_eq!(list["issues"]["nodes"], json!([{ "identifier": "ENG-1" }]));

    // Privates Team: not_found über Kennung und Liste
    let des = data(
        run(
            &schema,
            SeedUser::Bob,
            r#"mutation { issueCreate(input: {teamId: "DES", title: "Logo"}) { issue { id } } }"#,
        )
        .await,
    );
    let des_id = des["issueCreate"]["issue"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for q in [
        r#"{ issue(id: "DES-1") { id } }"#.to_owned(),
        format!(r#"{{ issue(id: "{des_id}") {{ id }} }}"#),
    ] {
        let res = run(&schema, SeedUser::Alice, &q).await;
        assert_eq!(code(&res), "not_found");
    }
    let res = run(
        &schema,
        SeedUser::Alice,
        r#"mutation { issueUpdate(id: "DES-1", input: {title: "x"}) { issue { id } } }"#,
    )
    .await;
    assert_eq!(code(&res), "not_found");
    let list = data(
        run(
            &schema,
            SeedUser::Alice,
            "{ issues { nodes { identifier } } }",
        )
        .await,
    );
    assert_eq!(list["issues"]["nodes"].as_array().unwrap().len(), 1);

    // Teamwechsel: neue Kennung, alte löst auf
    let moved = data(
        run(
            &schema,
            SeedUser::Bob,
            r#"mutation { issueCreate(input: {teamId: "WEB", title: "Umzug"}) { issue { identifier } } }"#,
        )
        .await,
    );
    assert_eq!(moved["issueCreate"]["issue"]["identifier"], "WEB-1");
    let m = data(
        run(
            &schema,
            SeedUser::Bob,
            r#"mutation { issueUpdate(id: "WEB-1", input: {teamId: "DES"}) { issue { identifier previousIdentifiers } } }"#,
        )
        .await,
    );
    assert_eq!(m["issueUpdate"]["issue"]["identifier"], "DES-2");
    assert_eq!(
        m["issueUpdate"]["issue"]["previousIdentifiers"],
        json!(["WEB-1"])
    );
    let by_old = data(
        run(
            &schema,
            SeedUser::Bob,
            r#"{ issue(id: "WEB-1") { identifier } }"#,
        )
        .await,
    );
    assert_eq!(by_old["issue"]["identifier"], "DES-2");

    // Papierkorb
    let del = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueDelete(id: "ENG-1") { success issue { trashed } } }"#,
        )
        .await,
    );
    assert_eq!(del["issueDelete"]["issue"]["trashed"], true);
    let gone = run(&schema, SeedUser::Alice, r#"{ issue(id: "ENG-1") { id } }"#).await;
    assert_eq!(code(&gone), "not_found");
    let back = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueUnarchive(id: "ENG-1") { issue { trashed } } }"#,
        )
        .await,
    );
    assert_eq!(back["issueUnarchive"]["issue"]["trashed"], false);

    // Fehlermodell: ungültige Priorität
    let bad = run(
        &schema,
        SeedUser::Alice,
        r#"mutation { issueUpdate(id: "ENG-1", input: {priority: 9}) { issue { id } } }"#,
    )
    .await;
    assert_eq!(code(&bad), "invalid_request");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn label_mutations_and_visibility(pool: sqlx::PgPool) {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let schema = build_schema(test_app(Arc::new(store)));
    let g = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueLabelCreate(input: {name: "Platform", isGroup: true}) { issueLabel { id isGroup groupType } } }"#,
        )
        .await,
    );
    assert_eq!(
        g["issueLabelCreate"]["issueLabel"]["groupType"],
        "singleSelect"
    );
    let c = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueLabelCreate(input: {name: "iOS", parentId: "Platform"}) { issueLabel { id parentId } } }"#,
        )
        .await,
    );
    assert!(c["issueLabelCreate"]["issueLabel"]["parentId"].is_string());
    data(
        run(
            &schema,
            SeedUser::Bob,
            r#"mutation { issueLabelCreate(input: {name: "Geheim", teamId: "DES"}) { issueLabel { id } } }"#,
        )
        .await,
    );
    let list = data(run(&schema, SeedUser::Alice, "{ issueLabels { name } }").await);
    let names: Vec<_> = list["issueLabels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(names.contains(&"Bug".to_owned()) && !names.contains(&"Geheim".to_owned()));
    let r = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueLabelRetire(id: "Platform") { issueLabel { retiredAt } } }"#,
        )
        .await,
    );
    assert!(r["issueLabelRetire"]["issueLabel"]["retiredAt"].is_string());
    let r = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueLabelRestore(id: "Platform") { issueLabel { retiredAt } } }"#,
        )
        .await,
    );
    assert_eq!(
        r["issueLabelRestore"]["issueLabel"]["retiredAt"],
        Value::Null
    );
    let u = data(
        run(&schema, SeedUser::Alice, r##"mutation { issueLabelUpdate(id: "iOS", input: {color: "#123456"}) { issueLabel { color } } }"##).await,
    );
    assert_eq!(u["issueLabelUpdate"]["issueLabel"]["color"], "#123456");
    let hidden = run(
        &schema,
        SeedUser::Alice,
        r#"mutation { issueLabelRetire(id: "Geheim") { issueLabel { id } } }"#,
    )
    .await;
    assert_eq!(code(&hidden), "not_found");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issue_children_relations_and_history_fields(pool: sqlx::PgPool) {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let schema = build_schema(test_app(Arc::new(store)));

    let parent = data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueCreate(input: {teamId: "ENG", title: "Warenkorb"}) { issue { id } } }"#,
        )
        .await,
    );
    let parent_id = parent["issueCreate"]["issue"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for (team, who, title) in [
        ("ENG", SeedUser::Alice, "Kind sichtbar"),
        ("DES", SeedUser::Bob, "Kind im privaten Team"),
    ] {
        let q = format!(
            r#"mutation {{ issueCreate(input: {{teamId: "{team}", title: "{title}", parentId: "{parent_id}"}}) {{ success }} }}"#
        );
        let res = run(&schema, who, &q).await;
        assert!(res.errors.is_empty(), "{team}: {:?}", res.errors);
    }
    data(
        run(
            &schema,
            SeedUser::Alice,
            &format!(
                r#"mutation {{ issueRelationCreate(input: {{issueId: "{parent_id}", relatedIssueId: "ENG-2", type: "blocks"}}) {{ success }} }}"#
            ),
        )
        .await,
    );
    data(
        run(
            &schema,
            SeedUser::Alice,
            r#"mutation { issueUpdate(id: "ENG-1", input: {title: "Warenkorb vereinfachen"}) { success } }"#,
        )
        .await,
    );

    let q = r#"{ issue(id: "ENG-1") {
        title
        children { identifier title }
        relations { blocks { identifier title } blockedBy { identifier } duplicateOf { identifier } }
        history { source changes flags createdAt }
    } }"#;
    let alice = data(run(&schema, SeedUser::Alice, q).await);
    let issue = &alice["issue"];
    // Das Kind im privaten Team ist für Alice unsichtbar.
    assert_eq!(
        issue["children"],
        json!([{ "identifier": "ENG-2", "title": "Kind sichtbar" }])
    );
    assert_eq!(
        issue["relations"]["blocks"],
        json!([{ "identifier": "ENG-2", "title": "Kind sichtbar" }])
    );
    assert_eq!(issue["relations"]["blockedBy"], json!([]));
    assert_eq!(issue["relations"]["duplicateOf"], Value::Null);
    let history = issue["history"].as_array().unwrap();
    // Änderungen im selben Moment werden mit dem Anlege-Eintrag zusammengefasst.
    assert!(!history.is_empty(), "{history:?}");
    // Die Test-Anfrage läuft als Cookie-Session: Herkunft `user` (API-Keys/OAuth ergäben `api`).
    assert_eq!(history[0]["source"], "user");
    assert!(
        history[0]["flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "created")
    );
    assert!(history[0]["createdAt"].is_string());

    // Bob sieht beide Kinder.
    let bob = data(run(&schema, SeedUser::Bob, q).await);
    assert_eq!(bob["issue"]["children"].as_array().unwrap().len(), 2);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn detail_fields_are_budgeted_per_request(pool: sqlx::PgPool) {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let schema = build_schema(test_app(Arc::new(store)));
    for i in 0..12 {
        data(
            run(
                &schema,
                SeedUser::Alice,
                &format!(
                    r#"mutation {{ issueCreate(input: {{teamId: "ENG", title: "Nr {i}"}}) {{ success }} }}"#
                ),
            )
            .await,
        );
    }

    // Detailansicht: drei Felder an einem Issue sind in Ordnung.
    let detail = run(
        &schema,
        SeedUser::Alice,
        r#"{ issue(id: "ENG-1") { history { source } relations { blocks { id } } children { id } } }"#,
    )
    .await;
    assert!(detail.errors.is_empty(), "{:?}", detail.errors);

    // Liste mit `history` über 12 Issues: nach 10 Aufrufen ist Schluss (invalid_request).
    let list = run(
        &schema,
        SeedUser::Alice,
        "{ issues(first: 12) { nodes { identifier history { source } } } }",
    )
    .await;
    assert_eq!(code(&list), "invalid_request");
    assert!(
        list.errors[0]
            .message
            .contains("only available in the detail view"),
        "{:?}",
        list.errors
    );

    // Ohne Budget in den Context-Daten (am Router vorbei) wird abgelehnt statt unbegrenzt erlaubt.
    let bare = schema
        .execute(
            Request::new(r#"{ issue(id: "ENG-1") { history { source } } }"#)
                .data(ctx_for(SeedUser::Alice)),
        )
        .await;
    assert_eq!(code(&bare), "invalid_request");
}
