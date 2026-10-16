//! GraphQL-Contract-Tests für `comment` gegen echte Datenbank (Seed `acme`).

use crate::graphql::build_schema;
use lineal_app::ports::Store;
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

async fn setup(pool: sqlx::PgPool) -> (Arc<App>, sqlx::PgPool) {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (test_app(Arc::new(store)), pool)
}

async fn issue(pool: &sqlx::PgPool, team: lineal_domain::ids::TeamId, slot: u128) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "WITH n AS (UPDATE team SET last_issue_number = last_issue_number + 1 \
         WHERE id = $1 RETURNING last_issue_number) \
         INSERT INTO issue (id, organization_id, team_id, number, title, state_id) \
         SELECT $2, $3, $1, last_issue_number, 'Login broken', $4 FROM n",
    )
    .bind(team.0)
    .bind(id)
    .bind(SeedData::acme().org.0)
    .bind(seed_workflow_state_ids(slot)[0].0)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn run(app: &Arc<App>, ctx: RequestCtx, query: &str, vars: Value) -> async_graphql::Response {
    let schema = build_schema(app.clone());
    schema
        .execute(
            async_graphql::Request::new(query)
                .variables(async_graphql::Variables::from_json(vars))
                .data(ctx)
                .data(crate::context::DetailBudget::default()),
        )
        .await
}

fn data(res: async_graphql::Response) -> Value {
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    res.data.into_json().unwrap()
}

fn code(res: &async_graphql::Response) -> String {
    let e = res.errors.first().expect("expected an error");
    let ext = e.extensions.as_ref().expect("extensions");
    serde_json::to_value(ext).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

const CREATE: &str = "mutation($input: CommentCreateInput!) { commentCreate(input: $input) { success lastSyncId comment { id body parentId quotedText issueId user { name displayName } reactionData { emoji count userIds } } } }";

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn comment_lifecycle_over_graphql(pool: sqlx::PgPool) {
    let (app, pool) = setup(pool).await;
    let eng = issue(&pool, SeedData::acme().eng, 1).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);

    let created = data(
        run(
            &app,
            alice.clone(),
            CREATE,
            json!({ "input": { "issueId": eng.to_string(), "body": "Hello <script>x</script>", "quotedText": "timeout" } }),
        )
        .await,
    );
    let c = &created["commentCreate"];
    assert_eq!(c["success"], true);
    assert!(c["lastSyncId"].as_f64().unwrap() > 0.0);
    assert!(
        !c["comment"]["body"]
            .as_str()
            .unwrap()
            .replace("\\<", "")
            .contains("<script")
    );
    assert_eq!(c["comment"]["issueId"], eng.to_string());
    assert_eq!(c["comment"]["quotedText"], "timeout");
    assert_eq!(c["comment"]["user"]["displayName"], "alice");
    let root = c["comment"]["id"].as_str().unwrap().to_owned();

    // Antwort, Reaktion (zweimal idempotent), Auflösen
    let reply = data(
        run(
            &app,
            bob.clone(),
            CREATE,
            json!({ "input": { "parentId": root, "body": "Because" } }),
        )
        .await,
    );
    assert_eq!(reply["commentCreate"]["comment"]["parentId"], root);
    let react = "mutation($input: ReactionCreateInput!) { reactionCreate(input: $input) { success reaction { id emoji commentId } } }";
    let r1 = data(
        run(
            &app,
            bob.clone(),
            react,
            json!({ "input": { "commentId": root, "emoji": "thumbsup" } }),
        )
        .await,
    );
    let r2 = data(
        run(
            &app,
            bob.clone(),
            react,
            json!({ "input": { "commentId": root, "emoji": "thumbsup" } }),
        )
        .await,
    );
    assert_eq!(
        r1["reactionCreate"]["reaction"]["id"],
        r2["reactionCreate"]["reaction"]["id"]
    );
    let res = run(
        &app,
        bob.clone(),
        react,
        json!({ "input": { "emoji": "x" } }),
    )
    .await;
    assert_eq!(code(&res), "invalid_request");

    let q = data(
        run(
            &app,
            alice.clone(),
            "query($id: String!) { comments(issueId: $id, first: 1, orderBy: createdAt) { nodes { id reactionData { emoji count userIds } } pageInfo { hasNextPage endCursor } } }",
            json!({ "id": eng.to_string() }),
        )
        .await,
    );
    assert_eq!(q["comments"]["pageInfo"]["hasNextPage"], true);
    assert_eq!(q["comments"]["nodes"].as_array().unwrap().len(), 1);
    let cursor = q["comments"]["pageInfo"]["endCursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let q2 = data(
        run(
            &app,
            alice.clone(),
            "query($id: String!, $after: String) { comments(issueId: $id, first: 5, after: $after, orderBy: createdAt) { nodes { id reactionData { emoji count } } pageInfo { hasNextPage } } }",
            json!({ "id": eng.to_string(), "after": cursor }),
        )
        .await,
    );
    assert_eq!(q2["comments"]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(q2["comments"]["nodes"][0]["id"], root);
    assert_eq!(q2["comments"]["nodes"][0]["reactionData"][0]["count"], 1);

    let resolved = data(
        run(
            &app,
            bob.clone(),
            "mutation($id: String!) { commentResolve(id: $id) { comment { id } } }",
            json!({ "id": root }),
        )
        .await,
    );
    assert_eq!(resolved["commentResolve"]["comment"]["id"], root);

    // Forbidden: fremden Kommentar bearbeiten; Inline-Wurzel nicht löschbar
    let upd = "mutation($id: String!, $input: CommentUpdateInput!) { commentUpdate(id: $id, input: $input) { comment { body } } }";
    let res = run(
        &app,
        bob.clone(),
        upd,
        json!({ "id": root, "input": { "body": "hijack" } }),
    )
    .await;
    assert_eq!(code(&res), "forbidden");
    let ok = data(
        run(
            &app,
            alice.clone(),
            upd,
            json!({ "id": root, "input": { "body": "edited" } }),
        )
        .await,
    );
    assert_eq!(ok["commentUpdate"]["comment"]["body"], "edited");
    let del = "mutation($id: String!) { commentDelete(id: $id) { success entityId } }";
    let res = run(&app, alice.clone(), del, json!({ "id": root })).await;
    assert_eq!(code(&res), "invalid_request");
    let gone = data(
        run(
            &app,
            bob.clone(),
            del,
            json!({ "id": reply["commentCreate"]["comment"]["id"] }),
        )
        .await,
    );
    assert_eq!(gone["commentDelete"]["success"], true);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_team_and_scopes_over_graphql(pool: sqlx::PgPool) {
    let (app, pool) = setup(pool).await;
    let des = issue(&pool, SeedData::acme().des, 3).await;
    let bob = ctx_for(SeedUser::Bob);
    let alice = ctx_for(SeedUser::Alice);
    data(
        run(
            &app,
            bob.clone(),
            CREATE,
            json!({ "input": { "issueId": des.to_string(), "body": "private" } }),
        )
        .await,
    );
    let res = run(
        &app,
        alice.clone(),
        CREATE,
        json!({ "input": { "issueId": des.to_string(), "body": "hi" } }),
    )
    .await;
    assert_eq!(code(&res), "not_found");
    let res = run(
        &app,
        alice,
        "query($id: String!) { comments(issueId: $id) { nodes { id } } }",
        json!({ "id": des.to_string() }),
    )
    .await;
    assert_eq!(code(&res), "not_found");
    let mut ro = bob;
    ro.scopes = lineal_app::Scopes::read_only();
    let res = run(
        &app,
        ro,
        CREATE,
        json!({ "input": { "issueId": des.to_string(), "body": "x" } }),
    )
    .await;
    assert_eq!(code(&res), "forbidden");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn attachments_over_graphql(pool: sqlx::PgPool) {
    let (app, pool) = setup(pool).await;
    let eng = issue(&pool, SeedData::acme().eng, 1).await;
    let bob = ctx_for(SeedUser::Bob);
    let link = "mutation($id: String!, $url: String!) { attachmentLinkURL(issueId: $id, url: $url, title: \"PR\") { success attachment { id url title sourceType } } }";
    let a = data(
        run(
            &app,
            bob.clone(),
            link,
            json!({ "id": eng.to_string(), "url": "https://Example.com/pr/1#x" }),
        )
        .await,
    );
    assert_eq!(
        a["attachmentLinkURL"]["attachment"]["url"],
        "https://example.com/pr/1"
    );
    let b = data(
        run(
            &app,
            bob.clone(),
            link,
            json!({ "id": eng.to_string(), "url": "https://example.com/pr/1/" }),
        )
        .await,
    );
    assert_eq!(
        a["attachmentLinkURL"]["attachment"]["id"],
        b["attachmentLinkURL"]["attachment"]["id"]
    );
    let res = run(
        &app,
        bob.clone(),
        link,
        json!({ "id": eng.to_string(), "url": "javascript:alert(1)" }),
    )
    .await;
    assert_eq!(code(&res), "invalid_request");

    // fileUpload ohne Ziel → Upload-URL, Header, assetUrl
    let up = data(
        run(
            &app,
            bob.clone(),
            "mutation { fileUpload(filename: \"a.png\", contentType: \"image/png\", size: 10) { success uploadFile { uploadUrl assetUrl headers { key value } filename size } } }",
            json!({}),
        )
        .await,
    );
    let f = &up["fileUpload"]["uploadFile"];
    assert!(f["assetUrl"].as_str().unwrap().contains("/assets/"));
    assert!(
        f["headers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["key"] == "content-type")
    );
    assert_eq!(f["size"], 10);

    let del = "mutation($id: String!) { attachmentDelete(id: $id) { success entityId } }";
    let d = data(
        run(
            &app,
            bob.clone(),
            del,
            json!({ "id": a["attachmentLinkURL"]["attachment"]["id"] }),
        )
        .await,
    );
    assert_eq!(d["attachmentDelete"]["success"], true);
    let res = run(
        &app,
        bob,
        "query($id: String!) { attachment(id: $id) { id } }",
        json!({ "id": a["attachmentLinkURL"]["attachment"]["id"] }),
    )
    .await;
    assert_eq!(code(&res), "not_found");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issue_attachments_link_count_and_resolve_assets(pool: sqlx::PgPool) {
    let (app, pool) = setup(pool).await;
    let eng = issue(&pool, SeedData::acme().eng, 1).await;
    let bob = ctx_for(SeedUser::Bob);
    let link = "mutation($id: String!, $url: String!) { attachmentLinkURL(issueId: $id, url: $url, title: \"PR\") { success } }";
    data(
        run(
            &app,
            bob.clone(),
            link,
            json!({ "id": eng.to_string(), "url": "https://example.com/pr/9" }),
        )
        .await,
    );

    let q = "query($id: String!) { issue(id: $id) { linkCount stateEnteredAt snoozedUntilAt attachments { title url sourceType } } }";
    let d = data(run(&app, bob.clone(), q, json!({ "id": eng.to_string() })).await);
    assert_eq!(d["issue"]["linkCount"], 1);
    assert_eq!(d["issue"]["attachments"][0]["title"], "PR");
    assert_eq!(d["issue"]["snoozedUntilAt"], Value::Null);
    assert!(d["issue"]["stateEnteredAt"].is_string());

    // Ein Upload ohne Ziel-Issue (Markdown-Bild) wird in eine signierte URL aufgelöst.
    let up = data(
        run(
            &app,
            bob.clone(),
            "mutation { fileUpload(filename: \"a.png\", contentType: \"image/png\", size: 10) { uploadFile { assetUrl } } }",
            json!({}),
        )
        .await,
    );
    let asset = up["fileUpload"]["uploadFile"]["assetUrl"].as_str().unwrap();
    let resolve = "query($urls: [String!]!) { resolveAssets(urls: $urls) { assetUrl downloadUrl title contentType } }";
    let r = data(
        run(
            &app,
            bob.clone(),
            resolve,
            json!({ "urls": [asset, "https://evil.example/assets/x/y/z"] }),
        )
        .await,
    );
    let list = r["resolveAssets"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["assetUrl"], asset);
    assert!(list[0]["downloadUrl"].as_str().unwrap().contains("/files/"));
    // Die dauerhafte URL bleibt unsigniert.
    assert!(!asset.contains("/files/"));
}
