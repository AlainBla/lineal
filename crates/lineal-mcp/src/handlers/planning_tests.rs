//! Contract-Tests der Planungsgruppen (`projects`, `milestones`, `cycles`, `initiatives`,
//! `status_updates`) gegen echte Datenbank (Seed `acme`, Uhr 2026-10-10).

use crate::dispatch::dispatch;
use lineal_app::ports::Store;
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use std::sync::Arc;

async fn setup(pool: sqlx::PgPool) -> Arc<App> {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.ids = Arc::new(V7);
    Arc::new(app)
}

struct V7;
impl lineal_app::ports::IdGen for V7 {
    fn new_uuid(&self) -> uuid::Uuid {
        uuid::Uuid::now_v7()
    }
}

async fn call(
    app: &App,
    who: SeedUser,
    tool: &str,
    args: Value,
) -> Result<Value, lineal_app::AppError> {
    dispatch(app, &ctx_for(who), tool, args)
        .await
        .expect("known tool")
}

fn err_kind(r: Result<Value, lineal_app::AppError>) -> ErrorKind {
    r.expect_err("expected an error").kind
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn projects_contract(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let list = call(&app, SeedUser::Alice, "list_projects", json!({}))
        .await
        .unwrap();
    assert_eq!(list["hasNextPage"], false);
    assert!(list.get("cursor").is_none());
    let p = &list["projects"][0];
    // Kennung als `id`, UUID separat; progress 0..100; Priority-Objekt; StatusRef; Refs mit UUID.
    assert_eq!(p["id"], "P-ENG-1");
    assert!(p["uuid"].as_str().unwrap().len() == 36);
    assert_eq!(p["name"], "Checkout v2");
    assert_eq!(p["summary"], "Neuer Checkout mit Wallets");
    assert!(p["description"].is_null());
    assert_eq!(p["priority"], json!({ "value": 2, "name": "High" }));
    assert_eq!(p["status"]["name"], "In Progress");
    assert_eq!(p["status"]["type"], "started");
    assert_eq!(p["progress"], 0.0);
    assert_eq!(p["leadTeam"]["key"], "ENG");
    assert_eq!(p["teams"][0]["key"], "ENG");
    assert_eq!(p["initiatives"][0]["identifier"], "I-1");
    assert_eq!(
        p["initiatives"][0]["id"].as_str().unwrap().len(),
        36,
        "eingebettet: UUID"
    );
    assert!(p["url"].as_str().unwrap().contains("/project/checkout-v2-"));
    // Alle Default-Schlüssel immer vorhanden; members/milestones/trashed/sortOrder nur bei Bedarf.
    for key in [
        "startDate",
        "targetDate",
        "startDateResolution",
        "health",
        "lead",
        "labels",
        "startedAt",
    ] {
        assert!(p.get(key).is_some(), "{key}");
    }
    assert!(p.get("members").is_none() && p.get("milestones").is_none());
    // fields: id plus angefragte Felder in angefragter Reihenfolge; Listenfelder ohne Wert -> [].
    let f = call(
        &app,
        SeedUser::Alice,
        "list_projects",
        json!({ "fields": ["name", "members", "sortOrder"] }),
    )
    .await
    .unwrap();
    let keys: Vec<&str> = f["projects"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["id", "name", "members", "sortOrder"]);
    assert_eq!(
        f["projects"][0]["members"][0]["email"],
        "alice@acme.example"
    );
    // Filter mit unbekannter Referenz: not_found; customView: invalid_request.
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "list_projects",
                json!({ "team": "NOPE" })
            )
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "list_projects",
                json!({ "customView": "x" })
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );

    // get_project mit Meilensteinen und Ressourcen.
    let g = call(&app, SeedUser::Alice, "get_project", json!({ "query": "P-ENG-1", "includeMilestones": true, "includeMembers": true, "includeResources": true })).await.unwrap();
    assert_eq!(g["milestones"][0]["name"], "Alpha");
    assert_eq!(g["milestones"][0]["status"], "next");
    assert_eq!(g["milestones"][0]["project"]["identifier"], "P-ENG-1");
    assert_eq!(
        g["resources"],
        json!({ "documents": [], "links": [], "attachments": [] })
    );
    assert_eq!(g["resourceCount"], 0);
    // Typfremde Kennung.
    let e = call(
        &app,
        SeedUser::Alice,
        "get_project",
        json!({ "query": "ENG-1" }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(
        call(
            &app,
            SeedUser::Alice,
            "get_project",
            json!({ "query": "P-ENG-99" })
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::NotFound
    );

    // save_project: Create (Teams, Lead, Status, Link, Initiative), Update (Tri-State, patch), Fehler.
    let c = call(
        &app,
        SeedUser::Bob,
        "save_project",
        json!({
            "name": "Mobile", "summary": "kurz", "description": "Zeile eins\nZeile zwei", "addTeams": ["ENG", "DES"],
            "leadTeam": "ENG", "lead": "me", "state": "planned", "priority": 3, "startDate": "2026-10-14",
            "startDateResolution": "quarter", "links": [{ "url": "https://example.com/x", "title": "Doku" }],
            "addInitiatives": ["I-1"]
        }),
    )
    .await
    .unwrap();
    assert_eq!(c["id"], "P-ENG-2");
    assert_eq!(c["status"]["type"], "planned");
    assert_eq!(c["startDate"], "2026-10-01");
    assert_eq!(c["lead"]["name"], "Bob Example");
    assert_eq!(c["teams"].as_array().unwrap().len(), 2);
    assert_eq!(c["initiatives"][0]["identifier"], "I-1");
    // Projekt-Hülle: Alice sieht DES nicht.
    let hull = call(
        &app,
        SeedUser::Alice,
        "get_project",
        json!({ "query": "P-ENG-2" }),
    )
    .await
    .unwrap();
    assert_eq!(hull["teams"].as_array().unwrap().len(), 1);
    // Alice darf ein Projekt mit einem für sie unsichtbaren Team nicht ändern.
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "save_project",
                json!({ "id": "P-ENG-2", "name": "X" })
            )
            .await
        ),
        ErrorKind::NotFound
    );
    let u = call(&app, SeedUser::Bob, "save_project", json!({ "id": "P-ENG-2", "lead": null, "patch": [{ "op": "replace", "old_string": "Zeile eins", "new_string": "Erste Zeile" }] })).await.unwrap();
    assert!(u["lead"].is_null());
    assert!(
        u["description"]
            .as_str()
            .unwrap()
            .starts_with("Erste Zeile")
    );
    let ambiguous = call(&app, SeedUser::Bob, "save_project", json!({ "id": "P-ENG-2", "patch": [{ "op": "replace", "old_string": "Zeile", "new_string": "X" }] })).await.unwrap_err();
    assert_eq!(ambiguous.kind, ErrorKind::Conflict);
    assert_eq!(
        call(
            &app,
            SeedUser::Bob,
            "save_project",
            json!({ "name": "X", "addTeams": ["ENG"], "setTeams": ["ENG"] })
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        call(
            &app,
            SeedUser::Bob,
            "save_project",
            json!({ "name": "X", "addTeams": ["ENG"], "summary": "x".repeat(256) })
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        call(&app, SeedUser::Bob, "save_project", json!({ "name": "X" }))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        call(
            &app,
            SeedUser::Bob,
            "save_project",
            json!({ "name": "X", "addTeams": ["ENG"], "patch": [{ "op": "append", "text": "x" }] })
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
    // Teams mit Issues entfernen: invalid_request (MCP) – hier ohne Issues erlaubt.
    let r = call(
        &app,
        SeedUser::Bob,
        "save_project",
        json!({ "id": "P-ENG-2", "removeTeams": ["DES"] }),
    )
    .await
    .unwrap();
    assert_eq!(r["teams"].as_array().unwrap().len(), 1);
    // Gast sieht Projekte ohne eigene Teams nicht.
    let carol = call(&app, SeedUser::Carol, "list_projects", json!({}))
        .await
        .unwrap();
    assert!(carol["projects"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn milestones_and_cycles_contract(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let m = call(
        &app,
        SeedUser::Alice,
        "list_milestones",
        json!({ "project": "P-ENG-1" }),
    )
    .await
    .unwrap();
    assert_eq!(m["hasNextPage"], false);
    assert_eq!(m["milestones"][0]["name"], "Alpha");
    assert_eq!(m["milestones"][0]["sortOrder"], 1.0);
    assert!(m["milestones"][0]["progress"].is_number());
    let s = call(
        &app,
        SeedUser::Alice,
        "save_milestone",
        json!({ "project": "P-ENG-1", "name": "Beta", "targetDate": "2027-01-31" }),
    )
    .await
    .unwrap();
    assert_eq!(s["sortOrder"], 2.0);
    assert_eq!(s["targetDate"], "2027-01-31");
    let g = call(
        &app,
        SeedUser::Alice,
        "get_milestone",
        json!({ "project": "P-ENG-1", "query": "beta" }),
    )
    .await
    .unwrap();
    assert_eq!(
        (g["issueCount"].clone(), g["completedIssueCount"].clone()),
        (json!(0), json!(0))
    );
    let u = call(
        &app,
        SeedUser::Alice,
        "save_milestone",
        json!({ "project": "P-ENG-1", "id": "Beta", "targetDate": null, "description": "x" }),
    )
    .await
    .unwrap();
    assert!(u["targetDate"].is_null());
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "save_milestone",
                json!({ "project": "P-ENG-1", "name": "alpha" })
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "get_milestone",
                json!({ "project": "P-ENG-1", "query": "Nope" })
            )
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "list_milestones",
                json!({ "project": "Nope" })
            )
            .await
        ),
        ErrorKind::NotFound
    );

    // list_cycles: isCurrent statt isActive, aufsteigend nach Nummer, progress 0..100, Hülle ohne cursor.
    let c = call(
        &app,
        SeedUser::Alice,
        "list_cycles",
        json!({ "teamId": "ENG" }),
    )
    .await
    .unwrap();
    assert_eq!(c["hasNextPage"], false);
    let cycles = c["cycles"].as_array().unwrap();
    assert_eq!(
        cycles
            .iter()
            .map(|x| x["number"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(cycles[1]["isCurrent"], true);
    assert!(cycles[1].get("isActive").is_none());
    assert_eq!(cycles[0]["isPast"], true);
    assert_eq!(cycles[0]["isPrevious"], true);
    assert_eq!(cycles[2]["isNext"], true);
    assert_eq!(cycles[3]["isFuture"], true);
    assert_eq!(cycles[1]["startsAt"], "2026-10-04T22:00:00.000Z");
    assert_eq!(cycles[1]["endsAt"], "2026-10-18T22:00:00.000Z");
    assert_eq!(cycles[1]["team"]["key"], "ENG");
    assert!(cycles[1]["progress"].is_number());
    assert!(cycles[1]["scopeHistory"].is_array());
    let cur = call(
        &app,
        SeedUser::Alice,
        "list_cycles",
        json!({ "teamId": "eng", "type": "current" }),
    )
    .await
    .unwrap();
    assert_eq!(cur["cycles"].as_array().unwrap().len(), 1);
    let prev = call(
        &app,
        SeedUser::Alice,
        "list_cycles",
        json!({ "teamId": "ENG", "type": "previous" }),
    )
    .await
    .unwrap();
    assert_eq!(prev["cycles"][0]["number"], 1);
    // Team ohne Cycles: leere Liste; privates Team: not_found; unbekanntes Team: not_found.
    let none = call(
        &app,
        SeedUser::Alice,
        "list_cycles",
        json!({ "teamId": "WEB" }),
    )
    .await
    .unwrap();
    assert_eq!(none, json!({ "cycles": [], "hasNextPage": false }));
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "list_cycles",
                json!({ "teamId": "DES" })
            )
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "list_cycles",
                json!({ "teamId": "NOPE" })
            )
            .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn initiatives_contract(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let l = call(&app, SeedUser::Alice, "list_initiatives", json!({}))
        .await
        .unwrap();
    let i = &l["initiatives"][0];
    assert_eq!(i["id"], "I-1");
    assert_eq!(
        i["status"],
        json!({ "id": "active", "name": "Active", "type": "started" })
    );
    assert_eq!(i["priority"], json!({ "value": 1, "name": "Urgent" }));
    assert_eq!(i["owner"]["name"], "Alice Example");
    assert!(i["leadTeam"].is_null());
    assert_eq!(i["parentInitiatives"], json!([]));
    assert!(i.get("projects").is_none());
    let g = call(
        &app,
        SeedUser::Alice,
        "get_initiative",
        json!({ "query": "I-1", "includeProjects": true, "includeSubInitiatives": true }),
    )
    .await
    .unwrap();
    assert_eq!(g["projects"][0]["identifier"], "P-ENG-1");
    assert_eq!(g["projects"][0]["status"]["type"], "started");
    assert_eq!(g["subInitiatives"], json!([]));
    // Anlegen mit Elternteil, Status per `type`, Zyklus -> invalid_request.
    let n = call(&app, SeedUser::Alice, "save_initiative", json!({ "name": "Wallet-Support", "parentInitiatives": ["I-1"], "status": "started", "summary": "kurz", "targetDate": "2026-12-31" })).await.unwrap();
    assert_eq!(n["id"], "I-2");
    assert_eq!(n["status"]["id"], "active");
    assert_eq!(n["parentInitiatives"][0]["identifier"], "I-1");
    let e = call(
        &app,
        SeedUser::Alice,
        "save_initiative",
        json!({ "id": "I-1", "parentInitiatives": ["I-2"] }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.details["reason"], "cycle");
    assert!(e.message.contains("cycle"));
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "save_initiative",
                json!({ "id": "I-1", "status": "bogus" })
            )
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(err_kind(call(&app, SeedUser::Alice, "save_initiative", json!({ "id": "I-1", "patch": [{ "op": "append", "text": "x" }], "description": "y" })).await), ErrorKind::InvalidRequest);
    let filtered = call(
        &app,
        SeedUser::Alice,
        "list_initiatives",
        json!({ "parentInitiative": "I-1" }),
    )
    .await
    .unwrap();
    assert_eq!(filtered["initiatives"].as_array().unwrap().len(), 1);
    // Gäste: forbidden.
    assert_eq!(
        err_kind(call(&app, SeedUser::Carol, "list_initiatives", json!({})).await),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn status_updates_contract(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let c = call(&app, SeedUser::Alice, "save_status_update", json!({ "type": "project", "project": "P-ENG-1", "body": "Alles gut.", "health": "atRisk" })).await.unwrap();
    assert_eq!(c["type"], "project");
    assert_eq!(c["health"], "atRisk");
    assert_eq!(c["project"]["identifier"], "P-ENG-1");
    assert!(c["initiative"].is_null());
    assert_eq!(c["user"]["name"], "Alice Example");
    assert!(
        c["url"]
            .as_str()
            .unwrap()
            .contains("/updates#project-update-")
    );
    assert_eq!(c["isDiffHidden"], false);
    let id = c["id"].as_str().unwrap().to_string();
    // Default-Health = vorheriges Update.
    let d = call(
        &app,
        SeedUser::Bob,
        "save_status_update",
        json!({ "type": "project", "project": "P-ENG-1", "body": "Weiter." }),
    )
    .await
    .unwrap();
    assert_eq!(d["health"], "atRisk");
    let list = call(
        &app,
        SeedUser::Alice,
        "get_status_updates",
        json!({ "type": "project", "project": "P-ENG-1" }),
    )
    .await
    .unwrap();
    assert_eq!(list["statusUpdates"].as_array().unwrap().len(), 2);
    assert_eq!(list["hasNextPage"], false);
    let one = call(
        &app,
        SeedUser::Alice,
        "get_status_updates",
        json!({ "type": "project", "id": id }),
    )
    .await
    .unwrap();
    assert_eq!(one["body"], "Alles gut.");
    // Typkonflikte, fehlende Parameter, fremdes Update editieren, Löschen.
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "get_status_updates",
                json!({ "type": "initiative", "project": "P-ENG-1" })
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "save_status_update",
                json!({ "type": "project", "body": "x" })
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "save_status_update",
                json!({ "type": "project", "project": "P-ENG-1" })
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Bob,
                "save_status_update",
                json!({ "type": "project", "id": id, "body": "hack" })
            )
            .await
        ),
        ErrorKind::Forbidden
    );
    let e = call(
        &app,
        SeedUser::Alice,
        "save_status_update",
        json!({ "type": "project", "id": id, "body": "Korrigiert." }),
    )
    .await
    .unwrap();
    assert!(e["editedAt"].is_string());
    let del = call(
        &app,
        SeedUser::Alice,
        "delete_status_update",
        json!({ "type": "project", "id": id }),
    )
    .await
    .unwrap();
    assert_eq!(del["success"], true);
    assert_eq!(del["id"], id);
    assert!(del["archivedAt"].is_string());
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Alice,
                "delete_status_update",
                json!({ "type": "project", "id": id })
            )
            .await
        ),
        ErrorKind::NotFound
    );
    // Initiative-Updates.
    let iu = call(
        &app,
        SeedUser::Alice,
        "save_status_update",
        json!({ "type": "initiative", "initiative": "I-1", "body": "Läuft." }),
    )
    .await
    .unwrap();
    assert_eq!(iu["type"], "initiative");
    assert_eq!(iu["initiative"]["identifier"], "I-1");
    assert!(iu["project"].is_null());
    assert_eq!(iu["health"], "onTrack");
    let il = call(
        &app,
        SeedUser::Alice,
        "get_status_updates",
        json!({ "type": "initiative" }),
    )
    .await
    .unwrap();
    assert_eq!(il["statusUpdates"].as_array().unwrap().len(), 1);
    assert_eq!(
        err_kind(
            call(
                &app,
                SeedUser::Carol,
                "get_status_updates",
                json!({ "type": "initiative" })
            )
            .await
        ),
        ErrorKind::Forbidden
    );
}
