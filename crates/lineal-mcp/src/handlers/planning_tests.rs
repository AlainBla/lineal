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
    // Filter mit unbekannter Referenz: not_found; unbekannte customView: not_found (WP11).
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
        ErrorKind::NotFound
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
    // Zugriff auf ein Projekt-Team genügt (docs/04 §2.2): Alice darf ändern, DES bleibt unsichtbar.
    let changed = call(
        &app,
        SeedUser::Alice,
        "save_project",
        json!({ "id": "P-ENG-2", "name": "X" }),
    )
    .await
    .unwrap();
    assert_eq!(changed["teams"].as_array().unwrap().len(), 1);
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
        ErrorKind::Conflict
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

/// LIN-25: Ressourcen hängen am Meilenstein; ohne ihn bleiben sie am Projekt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn milestone_resources_stay_with_the_project(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let who = SeedUser::Alice;
    let m = call(
        &app,
        who,
        "save_milestone",
        json!({ "project": "P-ENG-1", "name": "Go-Live" }),
    )
    .await
    .unwrap();
    let doc = call(
        &app,
        who,
        "save_document",
        json!({ "title": "Checkliste", "project": "P-ENG-1" }),
    )
    .await
    .unwrap();
    let doc_id = doc["uuid"].as_str().or(doc["id"].as_str()).unwrap();
    let scope = json!({ "project": "P-ENG-1", "milestone": "Go-Live" });
    let add = |extra: Value| {
        let mut a = scope.clone();
        a.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        a
    };
    call(
        &app,
        who,
        "save_milestone_resource",
        add(json!({ "url": "https://example.com/report", "title": "Bericht" })),
    )
    .await
    .unwrap();
    call(
        &app,
        who,
        "save_milestone_resource",
        add(json!({ "document": doc_id })),
    )
    .await
    .unwrap();
    let r = call(&app, who, "get_milestone_resources", scope.clone())
        .await
        .unwrap();
    assert_eq!(r["links"][0]["title"], "Bericht");
    assert_eq!(r["documents"][0]["title"], "Checkliste");

    // Genau eine Art je Aufruf; ein fremdes Dokument gibt es in diesem Projekt nicht.
    assert_eq!(
        err_kind(
            call(
                &app,
                who,
                "save_milestone_resource",
                add(json!({ "url": "https://example.com", "document": doc_id })),
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        err_kind(
            call(
                &app,
                who,
                "save_milestone_resource",
                add(json!({ "document": "00000000-0000-4000-8000-000000000001" })),
            )
            .await
        ),
        ErrorKind::NotFound
    );

    // Löschen des Meilensteins: Dokument und Link bleiben am Projekt.
    let view = ctx_for(who);
    app.delete_milestone(&view, "P-ENG-1", m["id"].as_str().unwrap())
        .await
        .unwrap();
    let left = app.list_resources(&view, "P-ENG-1", None).await.unwrap();
    assert!(left.links.iter().any(|l| l.label == "Bericht"));
    assert!(left.documents.iter().any(|d| d.title == "Checkliste"));
}

/// LIN-25: Abschließen ist bewusst, friert die Liste ein und behält den Terminverlauf.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn milestone_completion_is_deliberate(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let who = SeedUser::Alice;
    for name in ["Go-Live", "Nachbereitung"] {
        call(
            &app,
            who,
            "save_milestone",
            json!({ "project": "P-ENG-1", "name": name, "targetDate": "2027-01-10" }),
        )
        .await
        .unwrap();
    }
    let open_issue = call(
        &app,
        who,
        "save_issue",
        json!({ "team": "ENG", "title": "Offen", "project": "P-ENG-1", "milestone": "Go-Live" }),
    )
    .await
    .unwrap();
    call(
        &app,
        who,
        "save_issue",
        json!({ "team": "ENG", "title": "Fertig", "project": "P-ENG-1",
                "milestone": "Go-Live", "state": "completed" }),
    )
    .await
    .unwrap();

    // Terminverlauf: nur echte Änderungen zählen.
    let move_date = |d: &str| json!({ "project": "P-ENG-1", "id": "Go-Live", "targetDate": d });
    for d in ["2027-01-24", "2027-01-24"] {
        call(&app, who, "save_milestone", move_date(d))
            .await
            .unwrap();
    }
    let scope = json!({ "project": "P-ENG-1", "query": "Go-Live" });
    let g = call(&app, who, "get_milestone", scope.clone())
        .await
        .unwrap();
    assert_eq!(
        g["targetDateHistory"].as_array().unwrap().len(),
        1,
        "{}",
        g["targetDateHistory"]
    );
    assert_eq!(g["targetDateHistory"][0]["from"], "2027-01-10");
    assert_eq!(g["targetDateHistory"][0]["to"], "2027-01-24");
    assert_ne!(g["status"], "done", "erledigte Issues schließen nicht ab");

    let complete = |extra: Value| {
        let mut a = json!({ "project": "P-ENG-1", "milestone": "Go-Live" });
        a.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        a
    };
    assert_eq!(
        err_kind(
            call(
                &app,
                who,
                "complete_milestone",
                complete(json!({ "openIssues": "move", "moveTo": "Go-Live" })),
            )
            .await
        ),
        ErrorKind::InvalidRequest,
        "nicht in sich selbst"
    );
    let done = call(
        &app,
        who,
        "complete_milestone",
        complete(json!({ "openIssues": "move", "moveTo": "Nachbereitung", "note": "Live." })),
    )
    .await
    .unwrap();
    assert_eq!(done["status"], "done");

    // Das offene Issue wanderte, die eingefrorene Liste kennt beide.
    let moved = call(
        &app,
        who,
        "get_issue",
        json!({ "id": open_issue["id"], "fields": ["projectMilestone"] }),
    )
    .await
    .unwrap();
    assert_eq!(moved["projectMilestone"]["name"], "Nachbereitung");
    let frozen = app
        .milestone_snapshot(&ctx_for(who), "P-ENG-1", "Go-Live")
        .await
        .unwrap();
    assert_eq!(frozen.len(), 2);
    let g = call(&app, who, "get_milestone", scope.clone())
        .await
        .unwrap();
    assert_eq!(g["resultNote"], "Live.");

    assert_eq!(
        err_kind(
            call(
                &app,
                who,
                "complete_milestone",
                complete(json!({ "openIssues": "keep" })),
            )
            .await
        ),
        ErrorKind::Conflict
    );
    let again = call(
        &app,
        who,
        "reopen_milestone",
        json!({ "project": "P-ENG-1", "milestone": "Go-Live" }),
    )
    .await
    .unwrap();
    assert_ne!(again["status"], "done");
    let g = call(&app, who, "get_milestone", scope).await.unwrap();
    assert_eq!(g["resultNote"], "Live.", "die Notiz bleibt");
}

/// LIN-25: Umwandlung nimmt Issues und Ressourcen mit, Remind me greift am Meilenstein.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_project_team_does_not_block_milestone_convert(pool: sqlx::PgPool) {
    let app = setup(pool.clone()).await;
    let who = SeedUser::Alice;
    // Alice ist nur in ENG: Das zweite Projekt-Team wird privat und damit für sie unsichtbar.
    call(
        &app,
        who,
        "save_project",
        json!({ "id": "P-ENG-1", "addTeams": ["WEB"] }),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE team SET visibility = 'private' WHERE key = 'WEB'")
        .execute(&pool)
        .await
        .unwrap();
    call(
        &app,
        who,
        "save_milestone",
        json!({ "project": "P-ENG-1", "name": "Geteilt" }),
    )
    .await
    .unwrap();

    let converted = call(
        &app,
        who,
        "convert_milestone_to_project",
        json!({ "project": "P-ENG-1", "milestone": "Geteilt" }),
    )
    .await
    .unwrap();
    assert_eq!(converted["name"], "Geteilt");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn milestone_converts_to_project_and_reminds(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    let who = SeedUser::Alice;
    let ctx = ctx_for(who);
    let m = call(
        &app,
        who,
        "save_milestone",
        json!({ "project": "P-ENG-1", "name": "Spin-off", "description": "Eigenes Vorhaben" }),
    )
    .await
    .unwrap();
    let id = m["id"].as_str().unwrap();
    let issue = call(
        &app,
        who,
        "save_issue",
        json!({ "team": "ENG", "title": "Teil davon", "project": "P-ENG-1", "milestone": "Spin-off" }),
    )
    .await
    .unwrap();
    call(
        &app,
        who,
        "save_milestone_resource",
        json!({ "project": "P-ENG-1", "milestone": "Spin-off", "url": "https://example.com/a" }),
    )
    .await
    .unwrap();

    let at = chrono::Utc::now() + chrono::Duration::days(1);
    let r = app.set_reminder(&ctx, "milestone", id, Some(at)).await;
    // Uhr im Test: 2026-10-10; der Zeitpunkt liegt sicher in der Zukunft.
    assert!(r.unwrap().is_some());
    assert!(
        app.get_reminder(&ctx, "milestone", id)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        app.set_reminder(
            &ctx,
            "milestone",
            "00000000-0000-4000-8000-000000000001",
            Some(at)
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::NotFound
    );

    let created = call(
        &app,
        who,
        "convert_milestone_to_project",
        json!({ "project": "P-ENG-1", "milestone": "Spin-off" }),
    )
    .await
    .unwrap();
    assert_eq!(created["name"], "Spin-off");
    let new_id = created["id"].as_str().unwrap();
    let moved = call(
        &app,
        who,
        "get_issue",
        json!({ "id": issue["id"], "fields": ["project", "projectMilestone"] }),
    )
    .await
    .unwrap();
    assert_eq!(moved["project"], "Spin-off");
    assert!(moved["projectMilestone"].is_null());
    let res = app.list_resources(&ctx, new_id, None).await.unwrap();
    assert_eq!(res.links.len(), 1);
    assert_eq!(
        err_kind(
            call(
                &app,
                who,
                "get_milestone",
                json!({ "project": "P-ENG-1", "query": "Spin-off" }),
            )
            .await
        ),
        ErrorKind::NotFound
    );
}
