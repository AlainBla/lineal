//! Contract-Tests für die Gruppe `attachments` gegen echte Datenbank (Seed `acme`).

use super::image_urls;
use crate::envelope::CONTENT_KEY;
use crate::handlers::comments::tests::{call, make_issue, setup};
use lineal_app::ErrorKind;
use lineal_testkit::SeedData;
use lineal_testkit::SeedUser;
use serde_json::{Value, json};

fn kind(r: Result<Value, lineal_app::AppError>) -> ErrorKind {
    r.expect_err("expected an error").kind
}

#[test]
fn image_references_are_found_in_markdown_and_html() {
    let md = "Text ![a](https://x.test/a.png) and ![b](<https://x.test/b.png> \"Title\")\n\
              <img alt=\"c\" src=\"https://x.test/c.png\"> <IMG SRC='https://x.test/d.png'/> ![a](https://x.test/a.png)";
    assert_eq!(
        image_urls(md),
        [
            "https://x.test/a.png",
            "https://x.test/b.png",
            "https://x.test/c.png",
            "https://x.test/d.png"
        ]
    );
    assert!(image_urls("no images, [link](https://x.test)").is_empty());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn upload_flow_over_mcp(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let data = SeedData::acme();
    let issue = make_issue(&env, data.eng).await;
    let ident: String = sqlx::query_scalar("SELECT identifier FROM issue WHERE id = $1")
        .bind(issue)
        .fetch_one(&env.pool)
        .await
        .unwrap();

    let prepared = call(
        &env,
        SeedUser::Bob,
        "prepare_attachment_upload",
        json!({ "issue": ident, "filename": "screenshot.png", "contentType": "image/png", "size": 48213 }),
    )
    .await
    .unwrap();
    assert_eq!(prepared["uploadRequest"]["method"], "PUT");
    assert!(
        prepared["uploadRequest"]["url"]
            .as_str()
            .unwrap()
            .starts_with("mem://put/")
    );
    assert_eq!(
        prepared["uploadRequest"]["headers"]["content-type"],
        "image/png"
    );
    assert_eq!(
        prepared["uploadRequest"]["headers"]["x-content-length-range"],
        "48213,48213"
    );
    assert_eq!(prepared["suggested"]["title"], "screenshot.png");
    assert_eq!(prepared["suggested"]["subtitle"], Value::Null);
    assert!(prepared["expiresAt"].is_string());
    let asset = prepared["assetUrl"].as_str().unwrap().to_owned();
    let key = asset.split("/assets/").nth(1).unwrap().to_owned();

    // Finalisieren ohne Upload scheitert, danach PUT + finalize
    let err = call(
        &env,
        SeedUser::Bob,
        "create_attachment_from_upload",
        json!({ "issue": ident, "assetUrl": asset }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.details["reason"], "upload_missing");
    env.blobs
        .0
        .lock()
        .unwrap()
        .insert(key.clone(), (48213, "image/png".into()));
    env.blobs
        .1
        .lock()
        .unwrap()
        .insert(key, b"\x89PNG-fake-image".to_vec());
    let att = call(
        &env,
        SeedUser::Bob,
        "create_attachment_from_upload",
        json!({ "issue": ident, "assetUrl": asset, "title": "Login page" }),
    )
    .await
    .unwrap();
    assert_eq!(att["title"], "Login page");
    assert_eq!(att["issue"], ident);
    assert_eq!(att["contentType"], "image/png");
    assert_eq!(att["size"], 48213);
    assert_eq!(att["creator"]["name"], "Bob Example");
    assert!(att["url"].as_str().unwrap().starts_with("mem://get/"));
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Bob,
                "create_attachment_from_upload",
                json!({ "issue": ident, "assetUrl": asset })
            )
            .await
        ),
        ErrorKind::Conflict
    );

    // get_attachment (url und content): signierte URL, Ablauf, Hinweis
    let got = call(
        &env,
        SeedUser::Alice,
        "get_attachment",
        json!({ "id": att["id"], "format": "url" }),
    )
    .await
    .unwrap();
    assert!(got["url"].as_str().unwrap().ends_with("ttl=300"));
    assert!(got["expiresAt"].is_string());
    assert!(got.get("note").is_none());
    let content = call(
        &env,
        SeedUser::Alice,
        "get_attachment",
        json!({ "id": att["id"] }),
    )
    .await
    .unwrap();
    // B8.12: `content` liefert das Bild als image-Element (base64 + mimeType).
    let parts = content[CONTENT_KEY].as_array().unwrap();
    assert_eq!(parts[0]["type"], "text");
    assert_eq!(parts[1]["type"], "image");
    assert_eq!(parts[1]["mimeType"], "image/png");
    assert!(!parts[1]["data"].as_str().unwrap().is_empty());

    // extract_images: eigene Datei wird aufgelöst, Fremd-URL übersprungen, unbekannte eigene nicht gefunden
    let md = format!(
        "![own]({asset}) ![ext](https://example.org/x.png) ![gone]({}/assets/{}/aaa/bbb)",
        asset.split("/assets/").next().unwrap(),
        data.org.0
    );
    let res = call(
        &env,
        SeedUser::Alice,
        "extract_images",
        json!({ "markdown": md }),
    )
    .await
    .unwrap();
    let text = res[CONTENT_KEY][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    let statuses: Vec<_> = parsed["images"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["status"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(statuses, ["ok", "skipped_external", "not_found"]);
    assert!(
        parsed["images"][0]["downloadUrl"]
            .as_str()
            .unwrap()
            .starts_with("mem://get/")
    );
    let none = call(
        &env,
        SeedUser::Alice,
        "extract_images",
        json!({ "markdown": "plain" }),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(none[CONTENT_KEY][0]["text"].as_str().unwrap()).unwrap(),
        json!({ "images": [] })
    );

    // Der Anhang taucht in Kommentaren nicht auf, Bilder im Body werden nur für Body-Uploads signiert
    // (Issue-Anhänge sind über get_attachment erreichbar) – Delete zuletzt
    let del = call(
        &env,
        SeedUser::Bob,
        "delete_attachment",
        json!({ "id": att["id"] }),
    )
    .await
    .unwrap();
    assert_eq!(del, json!({ "success": true, "id": att["id"] }));
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Bob,
                "get_attachment",
                json!({ "id": att["id"] })
            )
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Bob,
                "delete_attachment",
                json!({ "id": att["id"] })
            )
            .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn attachment_errors_and_private_teams(pool: sqlx::PgPool) {
    let env = setup(pool).await;
    let data = SeedData::acme();
    let eng = make_issue(&env, data.eng).await.to_string();
    let des = make_issue(&env, data.des).await.to_string();
    let prepare = |issue: &str| json!({ "issue": issue, "filename": "a.pdf", "contentType": "application/pdf", "size": 10 });
    // Schema: Pflichtfelder, size > 0
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Bob,
                "prepare_attachment_upload",
                json!({ "issue": eng, "filename": "a" })
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Bob,
                "prepare_attachment_upload",
                json!({ "issue": eng, "filename": "a", "contentType": "x/y", "size": 0 })
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    // ≥ 2 GB → limit_exceeded; ungültiger MIME-Typ / Dateiname → invalid_request
    let mut big = prepare(&eng);
    big["size"] = json!(2_147_483_648_i64);
    assert_eq!(
        kind(call(&env, SeedUser::Bob, "prepare_attachment_upload", big).await),
        ErrorKind::LimitExceeded
    );
    let mut bad = prepare(&eng);
    bad["contentType"] = json!("pdf");
    assert_eq!(
        kind(call(&env, SeedUser::Bob, "prepare_attachment_upload", bad).await),
        ErrorKind::InvalidRequest
    );
    let mut bad = prepare(&eng);
    bad["filename"] = json!("");
    assert_eq!(
        kind(call(&env, SeedUser::Bob, "prepare_attachment_upload", bad).await),
        ErrorKind::InvalidRequest
    );
    // unbekanntes bzw. privates Issue → not_found
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Bob,
                "prepare_attachment_upload",
                prepare("ENG-4711")
            )
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Alice,
                "prepare_attachment_upload",
                prepare(&des)
            )
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Alice,
                "create_attachment_from_upload",
                json!({ "issue": des, "assetUrl": "http://localhost:8080/assets/x/y/z" })
            )
            .await
        ),
        ErrorKind::NotFound
    );
    // Fremder assetUrl
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Bob,
                "create_attachment_from_upload",
                json!({ "issue": eng, "assetUrl": "https://evil.example/assets/a/b/c" })
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    // Lese-Scope darf keine Uploads vorbereiten
    let mut ro = lineal_testkit::ctx_for(SeedUser::Bob);
    ro.scopes = lineal_app::Scopes::read_only();
    let r = crate::dispatch::dispatch(&env.app, &ro, "prepare_attachment_upload", prepare(&eng))
        .await
        .unwrap();
    assert_eq!(r.unwrap_err().kind, ErrorKind::Forbidden);
    assert_eq!(
        kind(
            call(
                &env,
                SeedUser::Bob,
                "get_attachment",
                json!({ "id": uuid::Uuid::now_v7().to_string() })
            )
            .await
        ),
        ErrorKind::NotFound
    );
}
