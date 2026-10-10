#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP10: Templates gegen echte Datenbank (Seed `acme`): Verwaltung, Geltungsbereich und Vererbung,
//! Merge-Regeln, Sub-Issues in einer Transaktion mit lückenlosen Nummern, Projekt- und
//! Dokument-Templates, Default-Templates.

use lineal_app::ports::Store;
use lineal_app::ports::repos::template::DefaultSlot;
use lineal_app::usecases::document::{ParentRefs, SaveDocumentInput};
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::usecases::project::SaveProjectInput;
use lineal_app::usecases::template::{SaveTemplateInput, TemplateView};
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::enums::TemplateType;
use lineal_testkit::seed::planning::seed_cycle_id;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct Env {
    app: App,
    pool: PgPool,
}

async fn setup(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    Env { app, pool }
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

async fn label_id(env: &Env, name: &str) -> String {
    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM issue_label WHERE name = $1 AND team_id IS NULL")
            .bind(name)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    id.to_string()
}

async fn make_template(
    env: &Env,
    who: SeedUser,
    ty: TemplateType,
    name: &str,
    team: Option<&str>,
    data: Value,
) -> TemplateView {
    env.app
        .save_template(
            &ctx_for(who),
            SaveTemplateInput {
                template_type: Some(ty),
                name: Some(name.into()),
                team: team.map(str::to_owned),
                template_data: Some(data),
                ..Default::default()
            },
        )
        .await
        .unwrap()
}

async fn last_issue_number(env: &Env, team: lineal_domain::ids::TeamId) -> i32 {
    sqlx::query_scalar("SELECT last_issue_number FROM team WHERE id = $1")
        .bind(team.0)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

fn issue_input(team: &str) -> CreateIssueInput {
    CreateIssueInput {
        team: team.into(),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn template_with_three_sub_issues_uses_gapless_numbers_in_one_transaction(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    // Vorher ein normales Issue, damit die Nummern nicht bei 1 beginnen.
    env.app
        .create_issue(
            &alice,
            CreateIssueInput {
                title: Some("Vorher".into()),
                ..issue_input("ENG")
            },
        )
        .await
        .unwrap();
    let tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Neues Feature",
        Some("ENG"),
        json!({
            "title": "Feature: ",
            "description": "## Ziel\n{{placeholder: Was soll der Nutzer danach koennen?}}",
            "priority": 3,
            "subIssues": [
                { "title": "Design abstimmen" },
                { "title": "Implementierung", "subIssues": [ { "title": "Backend" } ] },
                { "title": "Tests und Review", "dueIn": "P10D" }
            ]
        }),
    )
    .await;
    let before = last_issue_number(&env, SeedData::acme().eng).await;
    let out = env
        .app
        .create_issue_from_template(
            &alice,
            &tpl.template.id.to_string(),
            CreateIssueInput {
                title: Some("Dark Mode".into()),
                ..issue_input("ENG")
            },
            &[],
        )
        .await
        .unwrap();
    // Wurzel N, dann Tiefensuche: Design N+1, Implementierung N+2, Backend N+3, Tests N+4.
    let numbers: Vec<i32> = std::iter::once(out.root.issue.number)
        .chain(out.children.iter().map(|c| c.issue.number))
        .collect();
    assert_eq!(numbers, (before + 1..=before + 5).collect::<Vec<_>>());
    assert_eq!(
        last_issue_number(&env, SeedData::acme().eng).await,
        before + 5
    );
    let titles: Vec<_> = out
        .children
        .iter()
        .map(|c| c.issue.title.as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "Design abstimmen",
            "Implementierung",
            "Backend",
            "Tests und Review"
        ]
    );
    assert_eq!(out.children[0].issue.parent_id, Some(out.root.issue.id));
    assert_eq!(
        out.children[2].issue.parent_id,
        Some(out.children[1].issue.id)
    );
    let sorts: Vec<_> = [0, 1, 3]
        .iter()
        .map(|i| out.children[*i].issue.sub_issue_sort_order)
        .collect();
    assert_eq!(sorts, [Some(1.0), Some(2.0), Some(3.0)]);
    // Priorität des Wurzel-Issues, Platzhalter entfernt, lastAppliedTemplate gesetzt.
    assert_eq!(out.root.issue.priority.value(), 3);
    assert_eq!(out.root.issue.description.as_deref(), Some("## Ziel\n"));
    let applied: Option<Uuid> =
        sqlx::query_scalar("SELECT last_applied_template_id FROM issue WHERE id = $1")
            .bind(out.root.issue.id.0)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(applied, Some(tpl.template.id.0));
    // Je Issue ein IssueCreated-Event (1 Vorher-Issue + 5).
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox_event WHERE type = 'IssueCreated'")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(events, 6);

    // Schlägt ein Sub-Issue fehl (Assignee ohne Team-Zugriff), wird alles zurückgerollt.
    let bad = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Fehlerhaft",
        Some("ENG"),
        json!({ "title": "X", "subIssues": [
            { "title": "ok" },
            { "title": "kaputt", "assigneeId": SeedData::acme().carol.to_string() }
        ] }),
    )
    .await;
    let before = last_issue_number(&env, SeedData::acme().eng).await;
    let err = env
        .app
        .create_issue_from_template(&alice, "Fehlerhaft", issue_input("ENG"), &[])
        .await
        .err()
        .unwrap();
    assert_ne!(err.kind, ErrorKind::Internal);
    assert_eq!(err.details["path"], "subIssues[1]", "{err:?}");
    assert_eq!(last_issue_number(&env, SeedData::acme().eng).await, before);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM issue WHERE title IN ('X', 'ok', 'kaputt')")
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    drop(bad);

    // Template mit Sub-Issues + parentId -> invalid_request.
    let r = env
        .app
        .create_issue_from_template(
            &alice,
            &tpl.template.id.to_string(),
            CreateIssueInput {
                parent: Some(out.root.issue.id.to_string()),
                title: Some("Kind".into()),
                ..issue_input("ENG")
            },
            &[],
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
}

#[sqlx::test(migrations = "./migrations")]
async fn merge_rules_for_description_labels_and_scalars(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let bug = label_id(&env, "Bug").await;
    let feature = label_id(&env, "Feature").await;
    // Label-Gruppe `Platform` (singleSelect) mit iOS und Android.
    let org = SeedData::acme().org.0;
    let group = Uuid::now_v7();
    sqlx::query("INSERT INTO issue_label (id, organization_id, name, color, is_group, group_type) VALUES ($1, $2, 'Platform', '#112233', true, 'singleSelect')")
        .bind(group)
        .bind(org)
        .execute(&pool)
        .await
        .unwrap();
    for name in ["iOS", "Android"] {
        sqlx::query("INSERT INTO issue_label (id, organization_id, name, color, parent_id) VALUES ($1, $2, $3, '#112233', $4)")
            .bind(Uuid::now_v7())
            .bind(org)
            .bind(name)
            .bind(group)
            .execute(&pool)
            .await
            .unwrap();
    }
    let ios = label_id(&env, "iOS").await;
    let tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Standard",
        Some("ENG"),
        json!({
            "title": "Bug: ",
            "description": "Schritte\n1. {{placeholder: zuerst}}\n\nErwartet",
            "stateType": "started",
            "priority": 2,
            "estimate": null,
            "labelIds": [bug, ios],
            "dueIn": "P5D",
            "cycle": "current"
        }),
    )
    .await;
    let apply = |input: CreateIssueInput, nulls: Vec<String>| {
        let env = &env;
        let alice = alice.clone();
        let id = tpl.template.id.to_string();
        async move {
            env.app
                .create_issue_from_template(&alice, &id, input, &nulls)
                .await
        }
    };
    let names = |v: &lineal_app::usecases::issue::IssueView| {
        let mut n: Vec<String> = v.extras.label_names.clone();
        n.sort();
        n
    };

    // Leere Beschreibung behält den Template-Body (ohne Platzhalter); fehlender Titel -> Template-Titel.
    let a = apply(
        CreateIssueInput {
            description: Some("   ".into()),
            ..issue_input("ENG")
        },
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(
        a.root.issue.description.as_deref(),
        Some("Schritte\n1.\n\nErwartet")
    );
    assert_eq!(a.root.issue.title, "Bug:");
    // Status aus Kategorie, Priorität, Fälligkeit relativ (Europe/Berlin), Cycle `current`.
    assert_eq!(a.root.issue.state_type.as_str(), "started");
    assert_eq!(a.root.issue.priority.value(), 2);
    assert_eq!(a.root.issue.due_date.unwrap().to_string(), "2026-10-15");
    assert_eq!(a.root.issue.cycle_id, Some(seed_cycle_id(2)));
    assert_eq!(names(&a.root), ["Bug", "iOS"]);

    // Nicht leere Beschreibung ersetzt; Labels werden vereinigt; singleSelect: Input-Label gewinnt.
    let b = apply(
        CreateIssueInput {
            title: Some("Eigener Titel".into()),
            description: Some("Eigener Text".into()),
            labels: Some(vec![feature.clone(), "Android".into()]),
            priority: Some(4),
            ..issue_input("ENG")
        },
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(b.root.issue.title, "Eigener Titel");
    assert_eq!(b.root.issue.description.as_deref(), Some("Eigener Text"));
    assert_eq!(names(&b.root), ["Android", "Bug", "Feature"]);
    assert_eq!(b.root.issue.priority.value(), 4);

    // `labels: []` behält die Template-Labels; explizites `null` überschreibt Skalare.
    let c = apply(
        CreateIssueInput {
            labels: Some(vec![]),
            ..issue_input("ENG")
        },
        vec![
            "priority".into(),
            "dueDate".into(),
            "cycle".into(),
            "state".into(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(names(&c.root), ["Bug", "iOS"]);
    assert_eq!(c.root.issue.priority.value(), 0);
    assert!(c.root.issue.due_date.is_none());
    assert!(c.root.issue.cycle_id.is_none());
    assert_eq!(c.root.issue.state_type.as_str(), "backlog");

    // Kein Titel weder im Input noch im Template -> invalid_request.
    let notitle = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Ohne Titel",
        Some("ENG"),
        json!({ "priority": 1 }),
    )
    .await;
    let r = env
        .app
        .create_issue_from_template(
            &alice,
            &notitle.template.id.to_string(),
            issue_input("ENG"),
            &[],
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    // Zwei Geschwister einer singleSelect-Gruppe im Template selbst -> invalid_request.
    let android = label_id(&env, "Android").await;
    let clash = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Konflikt",
        Some("ENG"),
        json!({ "title": "K", "labelIds": [ios, android] }),
    )
    .await;
    let r = env
        .app
        .create_issue_from_template(
            &alice,
            &clash.template.id.to_string(),
            issue_input("ENG"),
            &[],
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    // Fehlende Labels (archiviert) werden still übersprungen und als Warnung gemeldet.
    sqlx::query("UPDATE issue_label SET archived_at = now() WHERE id = $1")
        .bind(Uuid::parse_str(&bug).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let d = apply(issue_input("ENG"), vec![]).await.unwrap();
    assert_eq!(names(&d.root), ["iOS"]);
    assert!(d.warnings.contains(&"labelIds".to_string()));
}

#[sqlx::test(migrations = "./migrations")]
async fn workspace_templates_scope_inheritance_and_visibility(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    // Workspace-Template: Gäste (Carol) -> forbidden.
    let r = env
        .app
        .save_template(
            &ctx_for(SeedUser::Carol),
            SaveTemplateInput {
                template_type: Some(TemplateType::Document),
                name: Some("PRD".into()),
                template_data: Some(json!({ "title": "PRD", "content": "## Problem" })),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Forbidden);
    let prd = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Document,
        "PRD",
        None,
        json!({ "title": "PRD", "content": "## Problem\n{{placeholder: Was?}}" }),
    )
    .await;
    // Workspace-Template mit team-gebundenen Werten -> invalid_request mit details.field.
    for data in [
        json!({ "title": "X", "stateId": Uuid::now_v7().to_string() }),
        json!({ "title": "X", "cycle": "current" }),
    ] {
        let err = env
            .app
            .save_template(
                &alice,
                SaveTemplateInput {
                    template_type: Some(TemplateType::Issue),
                    name: Some("WS".into()),
                    template_data: Some(data),
                    ..Default::default()
                },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(err.kind, ErrorKind::InvalidRequest);
        assert!(err.details["field"].is_string(), "{err:?}");
    }
    // Strukturfehler: unbekanntes Feld, dueIn + dueDate, zu tief, zu viele Sub-Issues, schlechte Priorität.
    let deep = json!({ "title": "t", "subIssues": [ { "title": "1", "subIssues": [ { "title": "2", "subIssues": [ { "title": "3", "subIssues": [ { "title": "4" } ] } ] } ] } ] });
    let many = json!({ "title": "t", "subIssues": (0..51).map(|i| json!({ "title": format!("s{i}") })).collect::<Vec<_>>() });
    for data in [
        json!({ "title": "t", "nope": 1 }),
        json!({ "title": "t", "dueIn": "P1D", "dueDate": "2026-12-01" }),
        json!({ "title": "t", "dueIn": "1 Tag" }),
        json!({ "title": "t", "priority": 9 }),
        deep,
        many,
    ] {
        let r = env
            .app
            .save_template(
                &alice,
                SaveTemplateInput {
                    template_type: Some(TemplateType::Issue),
                    name: Some("Bad".into()),
                    team: Some("ENG".into()),
                    template_data: Some(data.clone()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(kind(r), ErrorKind::InvalidRequest, "{data}");
    }
    // Doppelter Name im selben Geltungsbereich -> conflict; Name auf anderer Ebene erlaubt.
    let dup = env
        .app
        .save_template(
            &alice,
            SaveTemplateInput {
                template_type: Some(TemplateType::Document),
                name: Some("prd".into()),
                template_data: Some(json!({})),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(dup), ErrorKind::Conflict);

    // Team-Template in ENG: im Sub-Team WEB ohne Kopie sichtbar, in DES (privat, kein Kind) nicht.
    let eng_tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "ENG-Standard",
        Some("ENG"),
        json!({ "title": "Eng: " }),
    )
    .await;
    let names = |v: Vec<TemplateView>| v.into_iter().map(|t| t.template.name).collect::<Vec<_>>();
    let web = names(
        env.app
            .list_templates(&alice, Some("WEB"), None)
            .await
            .unwrap(),
    );
    assert_eq!(web, ["ENG-Standard", "PRD"]);
    let eng_only = names(
        env.app
            .list_templates(&alice, Some("ENG"), Some(TemplateType::Issue))
            .await
            .unwrap(),
    );
    assert_eq!(eng_only, ["ENG-Standard"]);
    env.app
        .create_issue_from_template(&alice, "ENG-Standard", issue_input("WEB"), &[])
        .await
        .unwrap();
    let r = env
        .app
        .create_issue_from_template(
            &ctx_for(SeedUser::Bob),
            "ENG-Standard",
            issue_input("DES"),
            &[],
        )
        .await;
    assert_eq!(kind(r), ErrorKind::NotFound);

    // WEB passt das geerbte Template an: Kopie mit inheritedFrom verdeckt das Original in WEB.
    let copy = env
        .app
        .save_template(
            &alice,
            SaveTemplateInput {
                id: Some("ENG-Standard".into()),
                as_team: Some("WEB".into()),
                template_data: Some(json!({ "title": "Web: " })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(copy.template.inherited_from_id, Some(eng_tpl.template.id));
    assert_eq!(copy.team.as_ref().unwrap().1, "WEB");
    let web = env
        .app
        .list_templates(&alice, Some("WEB"), Some(TemplateType::Issue))
        .await
        .unwrap();
    assert_eq!(web.len(), 1);
    assert_eq!(web[0].template.id, copy.template.id);
    let eng = env
        .app
        .list_templates(&alice, Some("ENG"), Some(TemplateType::Issue))
        .await
        .unwrap();
    assert_eq!(eng[0].template.id, eng_tpl.template.id);
    // Name mehrdeutig (WEB-Kopie und ENG-Original heißen gleich, Auflösung per Name) -> ambiguous_reference.
    assert_eq!(
        kind(env.app.get_template(&alice, "ENG-Standard").await),
        ErrorKind::AmbiguousReference
    );

    // Private Team-Templates: für Nicht-Mitglieder unsichtbar (Liste, get, Anwenden, Änderung).
    let secret = make_template(
        &env,
        SeedUser::Bob,
        TemplateType::Issue,
        "Design-Intern",
        Some("DES"),
        json!({ "title": "d" }),
    )
    .await;
    let id = secret.template.id.to_string();
    assert!(
        names(env.app.list_templates(&alice, None, None).await.unwrap())
            .iter()
            .all(|n| n != "Design-Intern")
    );
    assert!(
        names(env.app.list_templates(&bob, None, None).await.unwrap())
            .iter()
            .any(|n| n == "Design-Intern")
    );
    assert_eq!(
        kind(env.app.get_template(&alice, &id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(env.app.list_templates(&alice, Some("DES"), None).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            env.app
                .create_issue_from_template(&alice, &id, issue_input("ENG"), &[])
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(env.app.delete_template(&alice, &id).await),
        ErrorKind::NotFound
    );
    // Löschen archiviert und entfernt Default-Verweise.
    env.app
        .set_team_default_template(&bob, "DES", DefaultSlot::Members, Some(&id))
        .await
        .unwrap();
    env.app.delete_template(&bob, &id).await.unwrap();
    assert_eq!(
        kind(env.app.get_template(&bob, &id).await),
        ErrorKind::NotFound
    );
    let _ = prd;
}

#[sqlx::test(migrations = "./migrations")]
async fn default_templates_validate_type_and_availability(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let issue_tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Bug",
        Some("ENG"),
        json!({ "title": "B" }),
    )
    .await;
    let doc_tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Document,
        "Notiz",
        Some("ENG"),
        json!({}),
    )
    .await;
    let id = issue_tpl.template.id.to_string();
    env.app
        .set_team_default_template(&alice, "ENG", DefaultSlot::Members, Some(&id))
        .await
        .unwrap();
    // WEB (Sub-Team) darf ein ENG-Template als Default nutzen.
    env.app
        .set_team_default_template(&alice, "WEB", DefaultSlot::NonMembers, Some(&id))
        .await
        .unwrap();
    // Falscher Typ -> invalid_request.
    let r = env
        .app
        .set_team_default_template(
            &alice,
            "ENG",
            DefaultSlot::Members,
            Some(&doc_tpl.template.id.to_string()),
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    let r = env
        .app
        .set_team_default_template(&alice, "ENG", DefaultSlot::Project, Some(&id))
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    // Nicht im Team verfügbar (DES ist kein Sub-Team von ENG) -> not_found für Alice (DES unsichtbar).
    let r = env
        .app
        .set_team_default_template(&alice, "DES", DefaultSlot::Members, Some(&id))
        .await;
    assert_eq!(kind(r), ErrorKind::NotFound);
    // Löschen des Templates setzt die Verweise auf null.
    env.app.delete_template(&alice, &id).await.unwrap();
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM team WHERE default_template_for_members_id IS NOT NULL \
         OR default_template_for_non_members_id IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn document_template_and_project_template(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let doc_tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Document,
        "Testplan",
        None,
        json!({ "title": "Testplan", "content": "## Scope\n{{placeholder: Was?}}\n## Faelle", "icon": "Check" }),
    )
    .await;
    // Dokument aus Template: Titel/Inhalt/Icon vorbelegt, Platzhalter entfernt, Input gewinnt.
    let d = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                template: Some(doc_tpl.template.id.to_string()),
                parent: ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(d.doc.title, "Testplan");
    assert_eq!(d.doc.content, "## Scope\n\n## Faelle");
    assert_eq!(d.doc.icon.as_deref(), Some("Check"));
    assert_eq!(d.doc.last_applied_template_id, Some(doc_tpl.template.id));
    let d2 = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                template: Some("Testplan".into()),
                title: Some("Eigener Plan".into()),
                content: Some("Eigener Inhalt".into()),
                parent: ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (d2.doc.title.as_str(), d2.doc.content.as_str()),
        ("Eigener Plan", "Eigener Inhalt")
    );
    // Template bei Update ist unzulässig; falscher Template-Typ -> invalid_request.
    let r = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(d.doc.id.to_string()),
                template: Some("Testplan".into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);

    // Projekt-Template mit Meilensteinen, zwei Issues (eines mit Sub-Issue-Template) und einem Dokument.
    let sub_tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Review",
        Some("ENG"),
        json!({ "title": "Review", "priority": 2 }),
    )
    .await;
    let issue_tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Epic",
        Some("ENG"),
        json!({ "title": "Epic", "subIssues": [ { "title": "A" }, { "title": "B", "templateId": sub_tpl.template.id.to_string() } ] }),
    )
    .await;
    let project_tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Project,
        "Launch",
        Some("ENG"),
        json!({
            "name": "Launch: ",
            "summary": "Standard-Launch",
            "description": "Ziel {{placeholder: x}}",
            "priority": 2,
            "statusType": "planned",
            "leadId": "creator",
            "startIn": "P1D",
            "targetIn": "P4W",
            "milestones": [ { "name": "Alpha", "targetIn": "P1W" }, { "name": "Beta", "targetIn": "P3W" } ],
            "issues": [
                { "milestone": "Alpha", "issue": { "title": "Kickoff" } },
                { "templateId": issue_tpl.template.id.to_string(), "milestone": "Beta" }
            ],
            "documents": [ { "templateId": doc_tpl.template.id.to_string() }, { "document": { "title": "Notizen", "content": "leer" } } ]
        }),
    )
    .await;
    let before = last_issue_number(&env, SeedData::acme().eng).await;
    let view = env
        .app
        .save_project(
            &alice,
            SaveProjectInput {
                name: Some("Launch 2026".into()),
                template: Some(project_tpl.template.id.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(view.project.name, "Launch 2026");
    assert_eq!(view.project.priority, 2);
    assert_eq!(view.project.content.as_deref(), Some("Ziel"));
    assert_eq!(view.project.description.as_deref(), Some("Standard-Launch"));
    assert_eq!(view.project.lead_id, Some(SeedData::acme().alice));
    assert_eq!(view.project.start_date.unwrap().to_string(), "2026-10-11");
    assert_eq!(view.project.target_date.unwrap().to_string(), "2026-11-08");
    let project_id = view.project.id.0;
    let milestones: Vec<(String, Option<chrono::NaiveDate>)> = sqlx::query_as(
        "SELECT name, target_date FROM project_milestone WHERE project_id = $1 ORDER BY sort_order",
    )
    .bind(project_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        milestones.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(),
        ["Alpha", "Beta"]
    );
    assert_eq!(milestones[0].1.unwrap().to_string(), "2026-10-18");
    let issues: Vec<(String, Option<Uuid>, Option<i32>, i32)> = sqlx::query_as(
        "SELECT title, project_milestone_id, priority, number FROM issue WHERE project_id = $1 ORDER BY number",
    )
    .bind(project_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        issues.iter().map(|i| i.0.as_str()).collect::<Vec<_>>(),
        ["Kickoff", "Epic", "A", "B"]
    );
    // Lückenlose Nummern über Projekt-Issues und Sub-Issues.
    assert_eq!(
        issues.iter().map(|i| i.3).collect::<Vec<_>>(),
        (before + 1..=before + 4).collect::<Vec<_>>()
    );
    assert!(
        issues
            .iter()
            .all(|i| i.1.is_some() || i.0 == "A" || i.0 == "B")
    );
    let docs: Vec<String> =
        sqlx::query_scalar("SELECT title FROM document WHERE project_id = $1 ORDER BY sort_order")
            .bind(project_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(docs, ["Testplan", "Notizen"]);
    // Template bei Update -> invalid_request.
    let r = env
        .app
        .save_project(
            &alice,
            SaveProjectInput {
                id: Some(view.project.id.to_string()),
                template: Some("Launch".into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    // Fehler in einem Kind (Dokument-Template falscher Typ per ID) -> alles zurückgerollt.
    let broken = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Project,
        "Kaputt",
        Some("ENG"),
        json!({
            "name": "Kaputt",
            "issues": [ { "issue": { "title": "Eins" } } ],
            "documents": [ { "templateId": issue_tpl.template.id.to_string() } ]
        }),
    )
    .await;
    let before = last_issue_number(&env, SeedData::acme().eng).await;
    let r = env
        .app
        .save_project(
            &alice,
            SaveProjectInput {
                template: Some(broken.template.id.to_string()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    assert_eq!(last_issue_number(&env, SeedData::acme().eng).await, before);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM project WHERE name = 'Kaputt'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn template_content_is_sanitized_on_save_and_apply(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let evil = "Hi <script>alert(1)</script>[x](javascript:alert(2)) <img src=x onerror=alert(3)>";
    let tpl = make_template(
        &env,
        SeedUser::Alice,
        TemplateType::Issue,
        "Boese",
        Some("ENG"),
        json!({
            "title": "T <script>x</script>",
            "description": evil,
            "subIssues": [ { "title": "S <img src=x onerror=y>", "description": evil } ]
        }),
    )
    .await;
    // Beim Speichern bereinigt.
    let stored = tpl.template.template_data.to_string();
    for bad in ["<script", "javascript:", "onerror"] {
        assert!(!stored.contains(bad), "{bad} in {stored}");
    }
    // Auch wenn ein altes Template ungeprueft in der DB steht, bereinigt das Anwenden.
    sqlx::query("UPDATE template SET template_data = $2 WHERE id = $1")
        .bind(tpl.template.id.0)
        .bind(json!({
            "title": "T <script>x</script>",
            "description": evil,
            "subIssues": [ { "title": "S <img src=x onerror=y>", "description": evil } ]
        }))
        .execute(&pool)
        .await
        .unwrap();
    let out = env
        .app
        .create_issue_from_template(&alice, "Boese", issue_input("ENG"), &[])
        .await
        .unwrap();
    for v in std::iter::once(&out.root).chain(out.children.iter()) {
        let all = format!(
            "{} {}",
            v.issue.title,
            v.issue.description.clone().unwrap_or_default()
        );
        for bad in ["<script", "javascript:", "onerror", "<img"] {
            assert!(!all.contains(bad), "{bad} in {all}");
        }
    }
}
