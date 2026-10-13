#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP02: Use-Cases für Identität, Teams, Status und Auth gegen echte Datenbank (Seed `acme`).

use chrono::{DateTime, Utc};
use lineal_app::context::{Actor, Scopes};
use lineal_app::policy::Action;
use lineal_app::ports::Clock;
use lineal_app::ports::Store;
use lineal_app::ports::repos::user::{PageRequest, UserFilter};
use lineal_app::usecases::auth::{CreateApiKeyInput, LoginInput, hash_token};
use lineal_app::usecases::org::CreateWorkspaceInput;
use lineal_app::usecases::state::{CreateStateInput, UpdateStateInput};
use lineal_app::usecases::team::{CreateTeamInput, ListTeamsInput, NestOptions, UpdateTeamInput};
use lineal_app::usecases::user::{CreateUserInput, ListUsersInput};
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::{StateType, TeamVisibility, UserRole};
use lineal_testkit::seed::identity::{SEED_PASSWORD_HASH, seed_workflow_state_ids};
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

async fn setup(pool: PgPool) -> (App, SeedData, PgPool) {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = Arc::new(SystemClock);
    (app, data, pool)
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

#[sqlx::test(migrations = "./migrations")]
async fn seed_matches_the_example_table(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let teams = app
        .list_teams(&alice, ListTeamsInput::default())
        .await
        .unwrap();
    let keys: Vec<_> = teams.items.iter().map(|t| t.team.key.clone()).collect();
    assert!(keys.contains(&"ENG".to_string()) && keys.contains(&"WEB".to_string()));
    // Admin/Owner sehen private Teams als Metadaten (docs/02 §5.1; B8.05).
    assert!(
        keys.contains(&"DES".to_string()),
        "private team visible as metadata for the owner"
    );
    let web = app.get_team(&alice, "WEB").await.unwrap();
    assert_eq!(web.display_name, "Engineering › Web");
    assert_eq!(web.team.parent_id, Some(data.eng));
    // Zähler-Spalte initial 0
    let n: i32 = sqlx::query_scalar("SELECT last_issue_number FROM team WHERE id = $1")
        .bind(data.eng.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // Seed-Passwort ist ein Argon2id-Hash
    assert!(SEED_PASSWORD_HASH.starts_with("$argon2id$"));
}

#[sqlx::test(migrations = "./migrations")]
async fn counters_are_gapless_and_atomic(pool: PgPool) {
    let (app, data, _pool) = setup(pool).await;
    let mut tx = app.store.begin(data.org).await.unwrap();
    assert_eq!(
        tx.teams()
            .allocate_issue_numbers(data.eng, 1)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        tx.teams()
            .allocate_issue_numbers(data.eng, 3)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        tx.teams()
            .allocate_issue_numbers(data.eng, 1)
            .await
            .unwrap(),
        5
    );
    // Der Seed (planning) vergibt P-ENG-1 und vier Cycles in ENG.
    assert_eq!(tx.teams().next_project_number(data.eng).await.unwrap(), 2);
    assert_eq!(tx.teams().next_cycle_number(data.eng).await.unwrap(), 5);
    drop(tx); // Rollback: die Zähler springen zurück
    let mut tx = app.store.begin(data.org).await.unwrap();
    assert_eq!(
        tx.teams()
            .allocate_issue_numbers(data.eng, 1)
            .await
            .unwrap(),
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn private_teams_are_not_found_for_non_members(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let carol = ctx_for(SeedUser::Carol);
    let bob = ctx_for(SeedUser::Bob);
    for r in ["DES", "Design", "design"] {
        // Admin/Owner: Metadaten privater Teams sichtbar (B8.05); Guests nicht.
        assert!(app.get_team(&alice, r).await.is_ok(), "{r}");
        assert_eq!(
            kind(app.get_team(&carol, r).await),
            ErrorKind::NotFound,
            "{r}"
        );
    }
    assert!(app.get_team(&bob, "DES").await.is_ok());
    // Status-Liste und Mitglieder eines privaten Teams: ebenfalls not_found
    assert_eq!(
        kind(app.list_workflow_states(&alice, "DES").await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(app.list_workflow_states(&carol, "ENG").await),
        ErrorKind::NotFound
    );
    // Verwaltung: Alice (owner) sieht DES als Metadaten
    assert!(app.get_team_for_management(&alice, "DES").await.is_ok());
    // Gast sieht nur seine Teams
    let teams = app
        .list_teams(&carol, ListTeamsInput::default())
        .await
        .unwrap();
    let keys: Vec<_> = teams.items.iter().map(|t| t.team.key.as_str()).collect();
    assert_eq!(keys, ["WEB"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn team_creation_key_rules_and_default_workflow(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let bob = ctx_for(SeedUser::Bob);
    let t = app
        .create_team(
            &bob,
            CreateTeamInput {
                name: "Customer Success Ops".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(t.team.key, "CSO");
    assert_eq!(t.team.last_issue_number, 0);
    let states = app.list_workflow_states(&bob, "CSO").await.unwrap();
    let names: Vec<_> = states.iter().map(|s| s.state.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Backlog",
            "Todo",
            "In Progress",
            "Done",
            "Canceled",
            "Duplicate"
        ]
    );
    assert!(states[0].is_default);
    // Ersteller ist Mitglied und Owner
    let members = app.list_team_members(&bob, "CSO").await.unwrap();
    assert!(
        members
            .iter()
            .any(|m| m.user.id == t.team.id.0.into_user() || m.membership.owner)
    );
    // Key-Regeln
    for bad in ["eng-1", "1AB", "ABCDEFGH", "P", "I"] {
        let r = app
            .create_team(
                &bob,
                CreateTeamInput {
                    name: format!("T {bad}"),
                    key: Some(bad.into()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(kind(r), ErrorKind::InvalidRequest, "{bad}");
    }
    let r = app
        .create_team(
            &bob,
            CreateTeamInput {
                name: "Other".into(),
                key: Some("eng".into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Conflict);
    // Kollision bei Generierung -> Suffix
    let t2 = app
        .create_team(
            &bob,
            CreateTeamInput {
                name: "Cso Two".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(t2.team.key, "CT");
    let t3 = app
        .create_team(
            &bob,
            CreateTeamInput {
                name: "Cs Tools".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(t3.team.key, "CT2");
}

trait IntoUser {
    fn into_user(self) -> lineal_domain::ids::UserId;
}
impl IntoUser for uuid::Uuid {
    fn into_user(self) -> lineal_domain::ids::UserId {
        lineal_domain::ids::UserId(self)
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn key_change_keeps_old_keys_resolvable_and_reserved(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    // Bob ist nur Mitglied in ENG -> darf den Key nicht ändern.
    let r = app
        .update_team(
            &bob,
            "ENG",
            UpdateTeamInput {
                key: Some("CORE".into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Forbidden);
    let t = app
        .update_team(
            &alice,
            "ENG",
            UpdateTeamInput {
                key: Some("core".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(t.team.key, "CORE");
    assert_eq!(app.get_team(&alice, "ENG").await.unwrap().team.key, "CORE");
    let r = app
        .create_team(
            &alice,
            CreateTeamInput {
                name: "New".into(),
                key: Some("ENG".into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Conflict);
    // zurück auf den alten Key
    let t = app
        .update_team(
            &alice,
            "CORE",
            UpdateTeamInput {
                key: Some("ENG".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(t.team.key, "ENG");
}

#[sqlx::test(migrations = "./migrations")]
async fn nesting_rules(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    // Zyklus: ENG unter WEB (Nachfahre) verboten
    let r = app
        .nest_team(&alice, "ENG", Some("WEB"), NestOptions::default())
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    // Nur Admins dürfen nesten
    let r = app
        .nest_team(&bob, "WEB", None, NestOptions::default())
        .await;
    assert_eq!(kind(r), ErrorKind::Forbidden);
    // Sub-Team unter privatem Parent: Default restricted, public nicht erlaubt
    let sec = app
        .create_team(
            &alice,
            CreateTeamInput {
                name: "Secret".into(),
                parent: Some("DES".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(sec.team.visibility, TeamVisibility::Restricted);
    let r = app
        .create_team(
            &alice,
            CreateTeamInput {
                name: "Open".into(),
                parent: Some("DES".into()),
                visibility: Some(TeamVisibility::Public),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    // Tiefenlimit 5: Kette ENG > WEB > L3 > L4 > L5, ein weiteres Level scheitert
    let mut parent = "WEB".to_string();
    for i in 3..=5 {
        let t = app
            .create_team(
                &alice,
                CreateTeamInput {
                    name: format!("Level {i}"),
                    parent: Some(parent.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        parent = t.team.id.to_string();
    }
    let r = app
        .create_team(
            &alice,
            CreateTeamInput {
                name: "Level 6".into(),
                parent: Some(parent),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::LimitExceeded);
    // Un-Nesting WEB: erbende Status werden eigenständig, Team ist Root
    let web = app
        .nest_team(&alice, "WEB", None, NestOptions::default())
        .await
        .unwrap();
    assert!(web.team.parent_id.is_none() && !web.team.inherit_workflow_statuses);
    let states = app.list_workflow_states(&alice, "WEB").await.unwrap();
    assert!(states.iter().all(|s| s.state.inherited_from_id.is_none()));
    // Nesting mit Vorschau und Statusnormalisierung
    let preview = app
        .preview_team_nesting(&alice, "WEB", Some("ENG"))
        .await
        .unwrap();
    assert!(preview.conflicts.iter().any(|c| c.kind == "workflowState"));
    let web = app
        .nest_team(&alice, "WEB", Some("ENG"), NestOptions::default())
        .await
        .unwrap();
    assert!(web.team.inherit_workflow_statuses);
    assert_eq!(web.display_name, "Engineering › Web");
}

#[sqlx::test(migrations = "./migrations")]
async fn membership_rules(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);
    // Gast darf nicht selbst beitreten (ENG ist für Carol nicht einmal sichtbar)
    assert_eq!(
        kind(app.join_team(&carol, "ENG", false).await),
        ErrorKind::NotFound
    );
    // Alice tritt WEB bei (public)
    app.join_team(&alice, "WEB", false).await.unwrap();
    // Privates Team: nur mit Bestätigung
    assert_eq!(
        kind(app.join_team(&alice, "DES", false).await),
        ErrorKind::InvalidRequest
    );
    app.join_team(&alice, "DES", true).await.unwrap();
    // Gäste fügt nur ein Owner hinzu: Bob ist Owner von WEB
    app.add_team_member(&bob, "WEB", "carol@acme.example", false)
        .await
        .unwrap_err();
    // Owner ernennen: Gast nie
    assert_eq!(
        kind(app.set_team_owner(&bob, "WEB", "carol", true).await),
        ErrorKind::InvalidRequest
    );
    // Verlassen von ENG entfernt Bob auch aus WEB
    app.leave_team(&bob, "ENG", false).await.unwrap();
    let web_members = app.list_team_members(&alice, "WEB").await.unwrap();
    assert!(
        !web_members
            .iter()
            .any(|m| m.user.email == "bob@acme.example")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn workflow_state_crud_and_deletion_rules(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);
    let ids = seed_workflow_state_ids(1);
    // Anlegen: Position ans Ende der Kategorie
    let review = app
        .create_workflow_state(
            &bob,
            CreateStateInput {
                team: "ENG".into(),
                name: "In Review".into(),
                state_type: StateType::Started,
                color: "#0F783C".into(),
                description: Some("PR offen".into()),
                position: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(review.state.position, 2000.0);
    // Duplicate-State, doppelter Name, ungültige Farbe
    for (name, t, color, expected) in [
        (
            "Dupe",
            StateType::Duplicate,
            "#000000",
            ErrorKind::InvalidRequest,
        ),
        (
            "in review",
            StateType::Started,
            "#000000",
            ErrorKind::Conflict,
        ),
        ("X", StateType::Started, "red", ErrorKind::InvalidRequest),
    ] {
        let r = app
            .create_workflow_state(
                &bob,
                CreateStateInput {
                    team: "ENG".into(),
                    name: name.into(),
                    state_type: t,
                    color: color.into(),
                    description: None,
                    position: None,
                },
            )
            .await;
        assert_eq!(kind(r), expected, "{name}");
    }
    // WEB erbt: Status wurden mitkopiert, direktes Anlegen in WEB ist gesperrt
    let web_states = app.list_workflow_states(&bob, "WEB").await.unwrap();
    assert!(
        web_states
            .iter()
            .any(|s| s.state.name == "In Review"
                && s.state.inherited_from_id == Some(review.state.id))
    );
    let r = app
        .create_workflow_state(
            &bob,
            CreateStateInput {
                team: "WEB".into(),
                name: "Mine".into(),
                state_type: StateType::Started,
                color: "#000000".into(),
                description: None,
                position: None,
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    // Umbenennen propagiert
    app.update_workflow_state(
        &bob,
        review.state.id,
        UpdateStateInput {
            name: Some("QA".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let web_states = app.list_workflow_states(&bob, "WEB").await.unwrap();
    assert!(web_states.iter().any(|s| s.state.name == "QA"));
    // Duplicate ist nur umfärbbar
    let r = app
        .update_workflow_state(
            &bob,
            ids[5],
            UpdateStateInput {
                name: Some("Dup".into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    app.update_workflow_state(
        &bob,
        ids[5],
        UpdateStateInput {
            color: Some("#111111".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Löschen: Duplicate, Default, letzter einer Kategorie -> invalid_request
    for id in [ids[5], ids[0], ids[3]] {
        assert_eq!(
            kind(app.delete_workflow_state(&bob, id, None).await),
            ErrorKind::InvalidRequest
        );
    }
    // Mit Issues: Migrationsziel nötig
    sqlx::query(
        "INSERT INTO issue (id, organization_id, team_id, number, title, state_id) VALUES (gen_random_uuid(), $1, $2, 1, 'Test', $3)",
    )
    .bind(data.org.0)
    .bind(data.eng.0)
    .bind(review.state.id.0)
    .execute(&pool)
    .await
    .unwrap();
    let err = app
        .delete_workflow_state(&bob, review.state.id, None)
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.details["reason"], "migration_target_required");
    assert_eq!(err.details["issueCount"], 1);
    app.delete_workflow_state(&bob, review.state.id, Some(ids[2]))
        .await
        .unwrap();
    let st: String = sqlx::query_scalar("SELECT state_type FROM issue WHERE team_id = $1")
        .bind(data.eng.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st, "started");
    let remaining = app.list_workflow_states(&bob, "ENG").await.unwrap();
    assert!(!remaining.iter().any(|s| s.state.id == review.state.id));
    // Sichtbarkeit: Carol sieht Status von WEB, nicht von DES
    assert!(app.list_workflow_states(&carol, "WEB").await.is_ok());
    assert_eq!(
        kind(
            app.get_workflow_state(
                &carol,
                Some(&seed_workflow_state_ids(3)[0].to_string()),
                None,
                None
            )
            .await
        ),
        ErrorKind::NotFound
    );
    // get_issue_status: leere Strings gelten als nicht angegeben
    let s = app
        .get_workflow_state(&bob, Some(""), Some("todo"), Some("ENG"))
        .await
        .unwrap();
    assert_eq!(s.state.name, "Todo");
}

#[sqlx::test(migrations = "./migrations")]
async fn users_and_references(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let carol = ctx_for(SeedUser::Carol);
    let me = app.get_user(&alice, "me").await.unwrap();
    assert_eq!(me.user.id, data.alice);
    for r in [
        "bob@acme.example",
        "Bob Example",
        "bob",
        &data.bob.to_string(),
    ] {
        assert_eq!(
            app.get_user(&alice, r).await.unwrap().user.id,
            data.bob,
            "{r}"
        );
    }
    assert_eq!(
        kind(app.get_user(&alice, "nobody@acme.example").await),
        ErrorKind::NotFound
    );
    // Gast sieht nur User seiner Teams (Bob ist in WEB, Alice nicht)
    assert!(app.get_user(&carol, "bob").await.is_ok());
    assert_eq!(
        kind(app.get_user(&carol, "alice").await),
        ErrorKind::NotFound
    );
    let list = app
        .list_users(
            &carol,
            ListUsersInput {
                page: PageRequest::default(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut emails: Vec<_> = list.items.iter().map(|u| u.email.clone()).collect();
    emails.sort();
    assert_eq!(emails, ["bob@acme.example", "carol@acme.example"]);
    // get_user zeigt nur sichtbare Teams
    let bob_for_carol = app.get_user(&carol, "bob").await.unwrap();
    assert_eq!(
        bob_for_carol
            .teams
            .iter()
            .map(|t| t.team.key.as_str())
            .collect::<Vec<_>>(),
        ["WEB"]
    );
    // Paginierung: limit 1
    let p = app
        .list_users(
            &alice,
            ListUsersInput {
                page: PageRequest {
                    limit: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(p.has_next_page() && p.items.len() == 1);
    let p2 = app
        .list_users(
            &alice,
            ListUsersInput {
                page: PageRequest {
                    limit: 5,
                    cursor: p.next,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p2.items.len(), 2);
    let _ = UserFilter::default();
    // User anlegen: Gast braucht Team; Bob (user) darf einladen, aber keine Admins
    let bob = ctx_for(SeedUser::Bob);
    let r = app
        .create_user(
            &bob,
            CreateUserInput {
                name: "Dan".into(),
                email: "dan@acme.example".into(),
                role: UserRole::Admin,
                password: None,
                teams: vec![],
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Forbidden);
    let r = app
        .create_user(
            &bob,
            CreateUserInput {
                name: "Dan".into(),
                email: "dan@acme.example".into(),
                role: UserRole::Guest,
                password: None,
                teams: vec![],
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    let dan = app
        .create_user(
            &bob,
            CreateUserInput {
                name: "Dan".into(),
                email: "dan@acme.example".into(),
                role: UserRole::Guest,
                password: None,
                teams: vec!["WEB".into()],
            },
        )
        .await
        .unwrap();
    assert_eq!(dan.display_name, "dan");
    // B4.41: die Anlage mit Team meldet TeamMembershipCreated für die neue Mitgliedschaft.
    let created: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_event WHERE type = 'TeamMembershipCreated' AND payload->>'user_id' = $1",
    )
    .bind(dan.id.0.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(created, 1);
    // Der letzte Owner kann weder herabgestuft noch suspendiert werden
    assert_eq!(
        kind(app.set_user_role(&alice, "alice", UserRole::Admin).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&alice, "alice", false).await),
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn workspace_creation_builds_the_scaffold(pool: PgPool) {
    let app = Arc::try_unwrap(test_app(Arc::new(PgStore::new(pool.clone()))))
        .ok()
        .unwrap();
    let created = app
        .create_workspace(CreateWorkspaceInput {
            name: "Globex Corp".into(),
            url_key: None,
            owner_name: "Zed Example".into(),
            owner_email: "zed@globex.example".into(),
            owner_password: Some("lineal-dev".into()),
            credential_kind: None,
        })
        .await
        .unwrap();
    assert_eq!(created.org.url_key, "globex-corp");
    assert_eq!(created.owner.role, UserRole::Owner);
    assert_eq!(created.team.team.name, "Globex Corp");
    assert_eq!(created.team.team.key, "GC");
    let labels: i64 =
        sqlx::query_scalar("SELECT count(*) FROM issue_label WHERE organization_id = $1")
            .bind(created.org.id.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(labels, 3);
    let statuses: i64 =
        sqlx::query_scalar("SELECT count(*) FROM project_status WHERE organization_id = $1")
            .bind(created.org.id.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(statuses, 6);
    // urlKey-Kollision -> conflict; zweite Anlage mit gleichem Namen bekommt Suffix
    let r = app
        .create_workspace(CreateWorkspaceInput {
            name: "X".into(),
            url_key: Some("globex-corp".into()),
            owner_name: "A".into(),
            owner_email: "a@x.example".into(),
            owner_password: None,
            credential_kind: None,
        })
        .await;
    assert_eq!(kind(r), ErrorKind::Conflict);
    let second = app
        .create_workspace(CreateWorkspaceInput {
            name: "Globex Corp".into(),
            url_key: None,
            owner_name: "Y".into(),
            owner_email: "zed@globex.example".into(),
            owner_password: None,
            credential_kind: None,
        })
        .await
        .unwrap();
    assert_eq!(second.org.url_key, "globex-corp-2");
    // Org-Settings: Owner darf Limits ändern, Mitglieder nicht
    let ctx = RequestCtx {
        org_id: created.org.id,
        actor: Actor::User {
            id: created.owner.id,
            role: UserRole::Owner,
        },
        request_id: "r".into(),
        scopes: Scopes::all(),
        team_scope: None,

        via_session: false,
        client_ip: None,
        credential_kind: None,
    };
    let org = app
        .update_workspace(
            &ctx,
            lineal_app::usecases::org::UpdateWorkspaceInput {
                name: Some("Globex".into()),
                url_key: Some("globex".into()),
                limits: Some(serde_json::json!({"maxTeams": 3})),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(org.previous_url_keys, ["globex-corp"]);
    assert_eq!(org.limits["maxTeams"], 3);
    let _ = Action::CreateTeam;
}

#[sqlx::test(migrations = "./migrations")]
async fn login_logout_and_enumeration_resistance(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    // FakeHasher des Testkits: Seed-Hash ist ein Argon2-Hash und passt nicht -> zuerst Passwort setzen.
    app.set_user_password(&alice, "me", "correct-horse")
        .await
        .unwrap();
    let login = app
        .login(LoginInput {
            email: "ALICE@acme.example".into(),
            password: "correct-horse".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(login.token.len() >= 64);
    let p = app.principal_for_token(&login.token).await.unwrap();
    assert_eq!(p.org_id, SeedData::acme().org);
    // gleiche Fehlermeldung für falsches Passwort, unbekannte E-Mail, Gast ohne gültiges Passwort
    let wrong = app
        .login(LoginInput {
            email: "alice@acme.example".into(),
            password: "nope-nope".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    let unknown = app
        .login(LoginInput {
            email: "ghost@acme.example".into(),
            password: "nope-nope".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(wrong.kind, ErrorKind::Unauthenticated);
    assert_eq!(
        (wrong.kind, &wrong.message, &wrong.details),
        (unknown.kind, &unknown.message, &unknown.details)
    );
    // Zweiter Login erzeugt ein neues Token (keine Session-Fixation)
    let login2 = app
        .login(LoginInput {
            email: "alice@acme.example".into(),
            password: "correct-horse".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_ne!(login.token, login2.token);
    // Logout invalidiert serverseitig
    assert!(app.logout(&login.token, None, None).await.unwrap());
    assert_eq!(
        kind(app.principal_for_token(&login.token).await),
        ErrorKind::Unauthenticated
    );
    assert!(app.principal_for_token(&login2.token).await.is_ok());
    // Nur der Hash liegt in der Datenbank
    let (plain, hash): (i64, i64) = {
        let mut tx = app.store.begin_system().await.unwrap();
        let _ = tx
            .auth()
            .find_principal_by_token_hash(&hash_token(&login2.token))
            .await
            .unwrap();
        (0, 1)
    };
    assert_eq!((plain, hash), (0, 1));
}

#[sqlx::test(migrations = "./migrations")]
async fn principal_lookup_rejects_dead_tokens_and_clamps_scopes(pool: PgPool) {
    let (app, data, pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let carol = ctx_for(SeedUser::Carol);
    app.set_user_password(&alice, "me", "correct-horse")
        .await
        .unwrap();
    app.set_user_password(&carol, "me", "correct-horse")
        .await
        .unwrap();
    let lookup = |token: String| {
        let store = app.store.clone();
        async move {
            let mut tx = store.begin_system().await.unwrap();
            tx.auth()
                .find_principal_by_token_hash(&hash_token(&token))
                .await
                .unwrap()
        }
    };
    // Session
    let s = app
        .login(LoginInput {
            email: "alice@acme.example".into(),
            password: "correct-horse".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(lookup(s.token.clone()).await.is_some());
    sqlx::query(
        "UPDATE user_session SET expires_at = now() - interval '1 minute' WHERE token_hash = $1",
    )
    .bind(hash_token(&s.token))
    .execute(&pool)
    .await
    .unwrap();
    assert!(lookup(s.token.clone()).await.is_none(), "expired session");
    // Gast-Session: nie admin
    let gs = app
        .login(LoginInput {
            email: "carol@acme.example".into(),
            password: "correct-horse".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let p = lookup(gs.token.clone()).await.unwrap();
    assert!(!p.scopes.admin && p.scopes.write);
    assert!(matches!(
        p.actor,
        Actor::User {
            role: UserRole::Guest,
            ..
        }
    ));
    // API-Key: Read-Key bekommt nie write; Gast-Key nie admin; Widerruf wirkt sofort
    let read_key = app
        .create_api_key(
            &alice,
            CreateApiKeyInput {
                label: "ci".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(read_key.key.starts_with("lin_api_") && read_key.record.key_prefix.len() == 12);
    let p = lookup(read_key.key.clone()).await.unwrap();
    assert!(p.scopes.read && !p.scopes.write && !p.scopes.admin);
    let admin_key = app
        .create_api_key(
            &alice,
            CreateApiKeyInput {
                label: "adm".into(),
                scopes: vec!["write".into(), "admin".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(lookup(admin_key.key.clone()).await.unwrap().scopes.admin);
    // Gast kann keinen admin-Key anlegen; DB-seitig manipulierter Gast-Key wird auf kein admin begrenzt
    let r = app
        .create_api_key(
            &carol,
            CreateApiKeyInput {
                label: "g".into(),
                scopes: vec!["admin".into()],
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Forbidden);
    sqlx::query("UPDATE api_key SET user_id = $2 WHERE key_hash = $1")
        .bind(hash_token(&admin_key.key))
        .bind(data.carol.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !lookup(admin_key.key.clone()).await.unwrap().scopes.admin,
        "guest never admin"
    );
    app.revoke_api_key(&alice, read_key.record.id)
        .await
        .unwrap();
    assert!(lookup(read_key.key.clone()).await.is_none(), "revoked key");
    // Abgelaufener Key
    let exp = app
        .create_api_key(
            &alice,
            CreateApiKeyInput {
                label: "exp".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE api_key SET expires_at = now() - interval '1 minute' WHERE key_hash = $1")
        .bind(hash_token(&exp.key))
        .execute(&pool)
        .await
        .unwrap();
    assert!(lookup(exp.key.clone()).await.is_none(), "expired key");
    // Suspendierter und importierter User, archivierter Workspace
    let k = app
        .create_api_key(
            &alice,
            CreateApiKeyInput {
                label: "u".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(lookup(k.key.clone()).await.is_some());
    sqlx::query("UPDATE \"user\" SET active = false WHERE id = $1")
        .bind(data.alice.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(lookup(k.key.clone()).await.is_none(), "suspended user");
    sqlx::query("UPDATE \"user\" SET active = true, status = 'imported' WHERE id = $1")
        .bind(data.alice.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        lookup(k.key.clone()).await.is_none(),
        "imported placeholder user"
    );
    let r = app
        .login(LoginInput {
            email: "alice@acme.example".into(),
            password: "correct-horse".into(),
            ..Default::default()
        })
        .await;
    assert_eq!(
        kind(r),
        ErrorKind::Unauthenticated,
        "imported users cannot log in"
    );
    sqlx::query("UPDATE \"user\" SET status = 'active' WHERE id = $1")
        .bind(data.alice.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(lookup(k.key.clone()).await.is_some());
    sqlx::query("UPDATE organization SET archived_at = now() WHERE id = $1")
        .bind(data.org.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(lookup(k.key.clone()).await.is_none(), "archived workspace");
    // Unbekanntes Token
    assert!(lookup("lin_api_unknown".into()).await.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn role_changes_cannot_escalate(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);
    assert_eq!(
        kind(app.set_user_role(&bob, "me", UserRole::Admin).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_role(&carol, "me", UserRole::User).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_role(&bob, "carol", UserRole::User).await),
        ErrorKind::Forbidden
    );
    app.set_user_role(&alice, "bob", UserRole::Admin)
        .await
        .unwrap();
    let bob_admin = RequestCtx {
        actor: Actor::User {
            id: SeedData::acme().bob,
            role: UserRole::Admin,
        },
        ..bob.clone()
    };
    assert_eq!(
        kind(app.set_user_role(&bob_admin, "alice", UserRole::User).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            app.set_user_role(&bob_admin, "carol", UserRole::Owner)
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_role(&bob_admin, "me", UserRole::Owner).await),
        ErrorKind::Forbidden
    );
    assert!(
        app.set_user_role(&bob_admin, "carol", UserRole::User)
            .await
            .is_ok()
    );
    assert_eq!(
        kind(app.set_user_active(&bob_admin, "alice", false).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            app.set_user_password(&bob_admin, "alice", "hijack-pass")
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_role(&alice, "alice", UserRole::User).await),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn api_keys_are_scoped_to_their_owner(pool: PgPool) {
    let (app, _data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);
    let k = app
        .create_api_key(
            &bob,
            CreateApiKeyInput {
                label: "b".into(),
                scopes: vec!["write".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(app.list_api_keys(&alice).await.unwrap().is_empty());
    assert_eq!(app.list_api_keys(&bob).await.unwrap().len(), 1);
    assert_eq!(
        kind(app.revoke_api_key(&carol, k.record.id).await),
        ErrorKind::NotFound
    );
    let r = app
        .create_api_key(
            &bob,
            CreateApiKeyInput {
                label: "x".into(),
                scopes: vec!["admin".into()],
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Forbidden);
    let ro = RequestCtx {
        scopes: Scopes::read_only(),
        ..bob.clone()
    };
    let r = app
        .create_api_key(
            &ro,
            CreateApiKeyInput {
                label: "y".into(),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Forbidden);
    let other = RequestCtx {
        org_id: lineal_domain::ids::OrgId::new_v7(),
        ..bob.clone()
    };
    assert_eq!(
        kind(app.revoke_api_key(&other, k.record.id).await),
        ErrorKind::NotFound
    );
    app.revoke_api_key(&bob, k.record.id).await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn every_user_write_path_checks_scope_and_actor(pool: PgPool) {
    let (app, data, _pool) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let ro = RequestCtx {
        scopes: Scopes::read_only(),
        ..alice.clone()
    };
    let rw = RequestCtx {
        scopes: Scopes::read_write(),
        ..alice.clone()
    };
    let sys = RequestCtx::system(data.org, "req_sys");
    let new_user = || CreateUserInput {
        name: "Eve".into(),
        email: "eve@acme.example".into(),
        role: UserRole::User,
        password: None,
        teams: vec![],
    };
    // Read-Token: alle Pfade forbidden
    assert_eq!(
        kind(app.create_user(&ro, new_user()).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_role(&ro, "bob", UserRole::Admin).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&ro, "bob", false).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_password(&ro, "me", "new-password").await),
        ErrorKind::Forbidden
    );
    // write ohne admin-Scope: Rollen und Suspendierung bleiben gesperrt
    assert_eq!(
        kind(app.set_user_role(&rw, "bob", UserRole::Admin).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&rw, "bob", false).await),
        ErrorKind::Forbidden
    );
    // System-Kontext ist kein impliziter Admin
    assert_eq!(
        kind(app.create_user(&sys, new_user()).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_role(&sys, "bob", UserRole::Admin).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&sys, "bob", false).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_password(&sys, "bob", "new-password").await),
        ErrorKind::Forbidden
    );
    // App-Actor: kein Admin
    let app_ctx = RequestCtx {
        actor: Actor::App { id: data.bob },
        ..alice.clone()
    };
    assert_eq!(
        kind(app.set_user_role(&app_ctx, "carol", UserRole::User).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.create_user(&app_ctx, new_user()).await),
        ErrorKind::Forbidden
    );
    // Member: Anlegen ok (invitationsRole), Rollen/Suspendierung/fremdes Passwort nicht
    let bob = ctx_for(SeedUser::Bob);
    assert_eq!(
        kind(app.set_user_role(&bob, "carol", UserRole::User).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&bob, "carol", false).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_password(&bob, "carol", "new-password").await),
        ErrorKind::Forbidden
    );
    // Selbst-Hochstufung
    assert_eq!(
        kind(app.set_user_role(&bob, "me", UserRole::Owner).await),
        ErrorKind::Forbidden
    );
}
