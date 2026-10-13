#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Paket 8 (Validierung, Fehlercodes, Rechte): Regressionstests über die GraphQL-Schicht.

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

fn code(res: &async_graphql::Response) -> String {
    let e = res.errors.first().expect("expected error");
    let v = serde_json::to_value(e).unwrap();
    v["extensions"]["code"].as_str().unwrap_or("").to_owned()
}

const VIEWER_UPDATE: &str =
    "mutation($i: ViewerUpdateInput!) { viewerUpdate(input: $i) { success viewer { name } } }";
const CREATE_WS: &str =
    "mutation($i: CreateWorkspaceInput!) { createWorkspace(input: $i) { success } }";
const TEAM_CREATE: &str = "mutation($i: TeamCreateInput!) { teamCreate(input: $i) { success } }";
const LABEL_CREATE: &str =
    "mutation($i: IssueLabelCreateInput!) { issueLabelCreate(input: $i) { success } }";

/// B8.01: Namen mit Zeilenumbruch, Bidi-/Zero-Width-Zeichen werden überall abgelehnt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn names_reject_line_breaks_and_invisible_characters(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    for bad in [
        "X\nERROR: fake line",
        "a\u{2028}b",
        "a\u{202E}b",
        "a\u{200B}b",
    ] {
        let alice = || ctx_for(SeedUser::Alice);
        let r = run(&schema, alice(), VIEWER_UPDATE, json!({"i": {"name": bad}})).await;
        assert_eq!(code(&r), "invalid_request", "viewerUpdate {bad:?}");
        let r = run(&schema, alice(), CREATE_WS, json!({"i": {"name": bad}})).await;
        assert_eq!(code(&r), "invalid_request", "createWorkspace {bad:?}");
        let r = run(&schema, alice(), TEAM_CREATE, json!({"i": {"name": bad}})).await;
        assert_eq!(code(&r), "invalid_request", "teamCreate {bad:?}");
        let r = run(&schema, alice(), LABEL_CREATE, json!({"i": {"name": bad}})).await;
        assert_eq!(code(&r), "invalid_request", "issueLabelCreate {bad:?}");
    }
    // Normalisierung: NFD wird als NFC gespeichert.
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        VIEWER_UPDATE,
        json!({"i": {"name": "Cafe\u{301} Alice"}}),
    )
    .await;
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let v = r.data.into_json().unwrap();
    assert_eq!(v["viewerUpdate"]["viewer"]["name"], "Caf\u{e9} Alice");
}

const ISSUE_CREATE: &str = "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }";

/// B8.02: NUL-Zeichen in beliebigen Textfeldern liefern `invalid_request` statt `internal`.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn nul_characters_are_invalid_request_not_internal(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    let r = run(
        &schema,
        alice(),
        ISSUE_CREATE,
        json!({"i": {"teamId": "ENG", "title": "ok", "description": "a\u{0}b"}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    let r = run(
        &schema,
        alice(),
        TEAM_CREATE,
        json!({"i": {"name": "Nul Team", "description": "x\u{0}y"}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    let r = run(
        &schema,
        alice(),
        LABEL_CREATE,
        json!({"i": {"name": "n", "description": "x\u{0}y"}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
}

fn ext(res: &async_graphql::Response) -> Value {
    let e = res.errors.first().expect("expected error");
    serde_json::to_value(e).unwrap()["extensions"].clone()
}

/// B8.15: ungültige Eingaben (Argument-Coercion, Enum-Werte, Zeitpunkte) sind `invalid_request`.
/// B8.18: `extensions.details` ist ein Objekt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn invalid_inputs_are_invalid_request_with_object_details(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    let r = run(
        &schema,
        alice(),
        "mutation { teamUpdate(id: \"ENG\", input: { issueEstimationType: \"bogus\" }) { success } }",
        json!({}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    // Details bleiben ein Objekt (kein JSON-String).
    let r = run(
        &schema,
        alice(),
        "mutation($u: String!, $r: String!) { userRoleUpdate(userId: $u, role: $r) { success } }",
        json!({"u": SeedData::acme().bob.to_string(), "r": "superhero"}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    assert!(ext(&r)["details"].is_object(), "{}", ext(&r));
    assert!(ext(&r)["requestId"].is_string());
    let r = run(
        &schema,
        alice(),
        "mutation($i: NotificationUpdateInput!) { notificationUpdate(id: \"00000000-0000-0000-0000-000000000000\", input: $i) { success } }",
        json!({"i": {"snoozedUntilAt": "morgen"}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
}

/// B8.16: `first` außerhalb 1..=250 ist überall `invalid_request`.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn first_out_of_range_is_invalid_request(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    for q in [
        "{ issues(first: 0) { nodes { id } } }",
        "{ issues(first: -1) { nodes { id } } }",
        "{ issues(first: 251) { nodes { id } } }",
        "{ teams(first: 0) { id } }",
        "{ notifications(first: -1) { nodes { id } } }",
    ] {
        let r = run(&schema, ctx_for(SeedUser::Alice), q, json!({})).await;
        assert_eq!(code(&r), "invalid_request", "{q}: {:?}", r.errors);
    }
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        "{ issues(first: 5, after: \"\") { nodes { id } } }",
        json!({}),
    )
    .await;
    assert!(r.errors.is_empty(), "{:?}", r.errors);
}

/// B8.14: Datumseingaben strikt `YYYY-MM-DD` mit plausiblem Jahr.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn dates_are_strict_and_plausible(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    for bad in [
        "2026-2-3",
        "+2026-02-03",
        "26-02-03",
        "0001-01-01",
        "9999-12-31",
    ] {
        let r = run(
            &schema,
            ctx_for(SeedUser::Alice),
            "mutation($i: ProjectInput!) { projectCreate(input: $i) { success } }",
            json!({"i": {"name": "P", "setTeams": ["ENG"], "startDate": bad}}),
        )
        .await;
        assert_eq!(code(&r), "invalid_request", "project {bad}: {:?}", r.errors);
        let r = run(
            &schema,
            ctx_for(SeedUser::Alice),
            ISSUE_CREATE,
            json!({"i": {"teamId": "ENG", "title": "t", "dueDate": bad}}),
        )
        .await;
        assert_eq!(code(&r), "invalid_request", "issue {bad}: {:?}", r.errors);
    }
}

fn ok(r: &async_graphql::Response, what: &str) {
    assert!(r.errors.is_empty(), "{what}: {:?}", r.errors);
}

/// B8.03: `workspaceInitiativesRole` wirkt beim Anlegen einer Initiative ohne Lead-Team.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn workspace_initiatives_role_is_enforced(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    const INIT: &str = "mutation($i: InitiativeInput!) { initiativeCreate(input: $i) { success } }";
    let r = run(
        &schema,
        ctx_for(SeedUser::Bob),
        INIT,
        json!({"i": {"name": "I1"}}),
    )
    .await;
    ok(&r, "default role user");
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        "mutation($i: SecuritySettingsUpdateInput!) { workspaceSecuritySettingsUpdate(input: $i) { success } }",
        json!({"i": {"workspaceInitiativesRole": "admin"}}),
    )
    .await;
    ok(&r, "settings");
    let r = run(
        &schema,
        ctx_for(SeedUser::Bob),
        INIT,
        json!({"i": {"name": "I2"}}),
    )
    .await;
    assert_eq!(code(&r), "forbidden", "{:?}", r.errors);
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        INIT,
        json!({"i": {"name": "I3"}}),
    )
    .await;
    ok(&r, "admin allowed");
}

/// B8.04: Workspace-Views (geteilt, ohne Team) nur für Admin/Owner.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn workspace_views_need_admin(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    const VIEW: &str =
        "mutation($i: CustomViewCreateInput!) { customViewCreate(input: $i) { success } }";
    let input =
        json!({"i": {"name": "All", "modelName": "Issue", "filterData": {}, "shared": true}});
    let r = run(&schema, ctx_for(SeedUser::Bob), VIEW, input.clone()).await;
    assert_eq!(code(&r), "forbidden", "{:?}", r.errors);
    let r = run(&schema, ctx_for(SeedUser::Alice), VIEW, input).await;
    ok(&r, "owner allowed");
}

/// B8.05: Admins sehen private Teams ohne Mitgliedschaft als Metadaten. B8.26: Namenskonflikte
/// verraten private Teamnamen nicht.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn admin_sees_private_team_metadata_and_names_do_not_leak(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        "{ team(id: \"DES\") { key } }",
        json!({}),
    )
    .await;
    ok(&r, "alice team(DES)");
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        "{ teams { key } }",
        json!({}),
    )
    .await;
    ok(&r, "alice teams");
    let keys = r.data.into_json().unwrap()["teams"].to_string();
    assert!(keys.contains("DES"), "{keys}");
    // Bob (Mitglied) sieht DES ebenfalls; Carol (Gast, kein Mitglied) nicht.
    let r = run(
        &schema,
        ctx_for(SeedUser::Carol),
        "{ team(id: \"DES\") { key } }",
        json!({}),
    )
    .await;
    assert_eq!(code(&r), "not_found");
    // Alice legt ein privates Team an; Bob (kein Mitglied) kann den Namen verwenden.
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        TEAM_CREATE,
        json!({"i": {"name": "Secret Ops", "visibility": "private"}}),
    )
    .await;
    ok(&r, "private team");
    let r = run(
        &schema,
        ctx_for(SeedUser::Bob),
        TEAM_CREATE,
        json!({"i": {"name": "secret ops"}}),
    )
    .await;
    ok(&r, "bob reuses invisible private name");
    // Sichtbare Teams (Engineering) zählen weiterhin als Konflikt.
    let r = run(
        &schema,
        ctx_for(SeedUser::Bob),
        TEAM_CREATE,
        json!({"i": {"name": "engineering"}}),
    )
    .await;
    assert_eq!(code(&r), "conflict", "{:?}", r.errors);
}

/// B8.06: Selbstbeitritt zu einem öffentlichen Team mit `allMembersCanJoin`.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn self_join_via_membership_create(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    const MEMBER: &str = "mutation($t: String!, $u: String!) { teamMembershipCreate(teamId: $t, userId: $u) { success } }";
    let bob = SeedData::acme().bob.to_string();
    for (name, join) in [("Open Team", true), ("Closed Team", false)] {
        let r = run(
            &schema,
            ctx_for(SeedUser::Alice),
            TEAM_CREATE,
            json!({"i": {"name": name, "key": name[..3].to_uppercase(), "allMembersCanJoin": join}}),
        )
        .await;
        ok(&r, name);
        let r = run(
            &schema,
            ctx_for(SeedUser::Bob),
            MEMBER,
            json!({"t": name, "u": bob}),
        )
        .await;
        if join {
            ok(&r, "self join");
        } else {
            assert_eq!(code(&r), "forbidden", "{:?}", r.errors);
        }
    }
    // Andere hinzufügen bleibt verboten.
    let carol = SeedData::acme().carol.to_string();
    let r = run(
        &schema,
        ctx_for(SeedUser::Bob),
        MEMBER,
        json!({"t": "Open Team", "u": carol}),
    )
    .await;
    assert_eq!(code(&r), "forbidden", "{:?}", r.errors);
}

/// B8.07: Members sehen nur eigene Einladungen. B8.08: ungültige Adressen werden abgelehnt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn invitations_are_scoped_and_emails_validated(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    const INVITE: &str =
        "mutation($i: InviteUsersGqlInput!) { inviteUsers(input: $i) { __typename } }";
    for bad in [
        "a@example.test,b@example.test",
        "a@b@example.test",
        "<script>@x.y",
    ] {
        let r = run(
            &schema,
            ctx_for(SeedUser::Alice),
            INVITE,
            json!({"i": {"emails": [bad]}}),
        )
        .await;
        assert_eq!(code(&r), "invalid_request", "{bad}: {:?}", r.errors);
    }
    let r = run(
        &schema,
        ctx_for(SeedUser::Alice),
        INVITE,
        json!({"i": {"emails": ["alice-invitee@acme.example"]}}),
    )
    .await;
    ok(&r, "alice invites");
    let r = run(
        &schema,
        ctx_for(SeedUser::Bob),
        INVITE,
        json!({"i": {"emails": ["bob-invitee@acme.example"]}}),
    )
    .await;
    ok(&r, "bob invites");
    let list = "{ invitations(pendingOnly: true) { email } }";
    let r = run(&schema, ctx_for(SeedUser::Bob), list, json!({})).await;
    ok(&r, "bob list");
    let v = r.data.into_json().unwrap()["invitations"].to_string();
    assert!(
        v.contains("bob-invitee") && !v.contains("alice-invitee"),
        "{v}"
    );
    let r = run(&schema, ctx_for(SeedUser::Alice), list, json!({})).await;
    let v = r.data.into_json().unwrap()["invitations"].to_string();
    assert!(
        v.contains("bob-invitee") && v.contains("alice-invitee"),
        "{v}"
    );
}

/// B8.09: eigene frühere urlKeys sind zurücksetzbar, reservierte Namen gesperrt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn url_key_reserved_and_switch_back(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    const UPD: &str =
        "mutation($i: WorkspaceUpdateInput!) { workspaceUpdate(input: $i) { success } }";
    let alice = || ctx_for(SeedUser::Alice);
    let r = run(&schema, alice(), UPD, json!({"i": {"urlKey": "login"}})).await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    let r = run(&schema, alice(), UPD, json!({"i": {"urlKey": "acme-2"}})).await;
    ok(&r, "switch to acme-2");
    let r = run(&schema, alice(), UPD, json!({"i": {"urlKey": "acme"}})).await;
    ok(&r, "switch back to acme");
}

/// B8.10: `issueArchive` archiviert nur (nicht im Papierkorb); `trash: true` löscht in den Papierkorb.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issue_archive_does_not_trash(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    let r = run(&schema, alice(), "mutation { issueCreate(input: {teamId: \"ENG\", title: \"to archive\"}) { issue { id } } }", json!({})).await;
    ok(&r, "create");
    let id = r.data.into_json().unwrap()["issueCreate"]["issue"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = run(
        &schema,
        alice(),
        "mutation($id: String!) { issueArchive(id: $id, trash: false) { issue { archivedAt trashed } } }",
        json!({"id": id}),
    )
    .await;
    ok(&r, "archive");
    let v = r.data.into_json().unwrap();
    assert!(v["issueArchive"]["issue"]["archivedAt"].is_string(), "{v}");
    assert_eq!(v["issueArchive"]["issue"]["trashed"], false, "{v}");
    let r = run(
        &schema,
        alice(),
        "mutation($id: String!) { issueUnarchive(id: $id) { issue { archivedAt } } }",
        json!({"id": id}),
    )
    .await;
    ok(&r, "unarchive");
    let r = run(
        &schema,
        alice(),
        "mutation($id: String!) { issueArchive(id: $id, trash: true) { issue { trashed } } }",
        json!({"id": id}),
    )
    .await;
    ok(&r, "trash");
    assert_eq!(
        r.data.into_json().unwrap()["issueArchive"]["issue"]["trashed"],
        true
    );
}

/// B8.19: Namenskollisionen sind durchgängig `conflict`. B8.21: Sortierwerte begrenzt.
/// B8.23: leere IDs im Batch erscheinen als Fehler. B8.25: Client-ID-Kollision.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn conflicts_ranges_batch_and_client_ids(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    const STATE: &str =
        "mutation($i: WorkflowStateCreateInput!) { workflowStateCreate(input: $i) { success } }";
    let input =
        json!({"i": {"teamId": "ENG", "name": "Review", "type": "started", "color": "#aabbcc"}});
    let r = run(&schema, alice(), STATE, input.clone()).await;
    ok(&r, "state");
    let r = run(&schema, alice(), STATE, input).await;
    assert_eq!(code(&r), "conflict", "{:?}", r.errors);

    let r = run(&schema, alice(), "mutation { issueCreate(input: {teamId: \"ENG\", title: \"x\", sortOrder: -1e308}) { success } }", json!({})).await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);

    let r = run(&schema, alice(), "mutation { issueCreate(input: {teamId: \"ENG\", title: \"a\"}) { issue { id identifier } } }", json!({})).await;
    ok(&r, "issue");
    let v = r.data.into_json().unwrap();
    let id = v["issueCreate"]["issue"]["id"].as_str().unwrap().to_owned();
    let ident = v["issueCreate"]["issue"]["identifier"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = run(
        &schema,
        alice(),
        "mutation($i: IssueBatchUpdateInput!) { issueBatchUpdate(input: $i) { issues { id } failures { id code } } }",
        json!({"i": {"ids": ["", ident], "priority": 2}}),
    )
    .await;
    ok(&r, "batch");
    let v = r.data.into_json().unwrap();
    assert_eq!(
        v["issueBatchUpdate"]["failures"][0]["code"], "invalid_request",
        "{v}"
    );
    assert_eq!(
        v["issueBatchUpdate"]["issues"].as_array().unwrap().len(),
        1,
        "{v}"
    );

    let r = run(
        &schema,
        alice(),
        ISSUE_CREATE,
        json!({"i": {"id": id, "teamId": "ENG", "title": "dup"}}),
    )
    .await;
    assert_eq!(code(&r), "conflict", "{:?}", r.errors);
    assert!(
        r.errors[0].message.contains("id already in use"),
        "{:?}",
        r.errors
    );
}

/// B8.27: Team- und Workspace-Änderungen landen im Audit-Log.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn audit_covers_team_and_workspace_changes(pool: sqlx::PgPool) {
    let schema = setup(pool.clone()).await;
    let alice = || ctx_for(SeedUser::Alice);
    let r = run(
        &schema,
        alice(),
        "mutation { workspaceUpdate(input: {name: \"Acme Inc\"}) { success } }",
        json!({}),
    )
    .await;
    ok(&r, "workspace");
    let r = run(
        &schema,
        alice(),
        "mutation { teamUpdate(id: \"ENG\", input: {description: \"hello\"}) { success } }",
        json!({}),
    )
    .await;
    ok(&r, "team");
    let carol = SeedData::acme().carol.to_string();
    let r = run(
        &schema,
        alice(),
        "mutation($u: String!) { teamMembershipCreate(teamId: \"ENG\", userId: $u) { success } }",
        json!({"u": carol}),
    )
    .await;
    ok(&r, "member");
    let actions: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_log ORDER BY occurred_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    for a in ["workspace.updated", "team.updated", "team.member_added"] {
        assert!(actions.iter().any(|x| x == a), "{a} missing in {actions:?}");
    }
}

/// B8.28: Standard-Schätzwert muss auf der Skala liegen. B8.29: Team-Icons nur Allowlist.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn team_estimate_default_and_icon_validated(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    const UPD: &str =
        "mutation($i: TeamUpdateInput!) { teamUpdate(id: \"ENG\", input: $i) { success } }";
    let r = run(&schema, alice(), UPD, json!({"i": {"icon": "<script>"}})).await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    let r = run(
        &schema,
        alice(),
        UPD,
        json!({"i": {"issueEstimationType": "fibonacci", "defaultIssueEstimate": 4}}),
    )
    .await;
    assert_eq!(code(&r), "invalid_request", "{:?}", r.errors);
    let r = run(
        &schema,
        alice(),
        UPD,
        json!({"i": {"issueEstimationType": "fibonacci", "defaultIssueEstimate": 5}}),
    )
    .await;
    ok(&r, "valid default");
}

/// B8.11: Kommentare lösen frühere Issue-Kennungen nach einem Team-Key-Wechsel auf.
/// B8.20: Für unsichtbare Issues nennt der Fehler das Issue, nicht das Team.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn comments_resolve_old_identifiers_and_name_the_issue(pool: sqlx::PgPool) {
    let schema = setup(pool).await;
    let alice = || ctx_for(SeedUser::Alice);
    let r = run(
        &schema,
        alice(),
        "mutation { issueCreate(input: {teamId: \"ENG\", title: \"keyed\"}) { issue { identifier } } }",
        json!({}),
    )
    .await;
    ok(&r, "issue");
    let old = r.data.into_json().unwrap()["issueCreate"]["issue"]["identifier"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = run(
        &schema,
        alice(),
        "mutation { teamUpdate(id: \"ENG\", input: {key: \"CORE\"}) { success } }",
        json!({}),
    )
    .await;
    ok(&r, "key change");
    const COMMENT: &str = "mutation($i: String!) { commentCreate(input: {issueId: $i, body: \"hello\"}) { success } }";
    let r = run(&schema, alice(), COMMENT, json!({"i": old})).await;
    ok(&r, "comment with old identifier");
    let r = run(
        &schema,
        alice(),
        "{ comments(issueId: \"NOPE-1\") { nodes { id } } }",
        json!({}),
    )
    .await;
    let e = serde_json::to_value(r.errors.first().expect("error")).unwrap();
    assert_eq!(e["extensions"]["code"], "not_found");
    assert_eq!(e["extensions"]["details"]["type"], "Issue", "{e}");
}
